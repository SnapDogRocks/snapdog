// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! MPRIS2 D-Bus interface — one player per zone.

#[cfg(feature = "dbus")]
mod player;
#[cfg(feature = "dbus")]
mod root;

#[cfg(feature = "dbus")]
pub use self::mpris::start_mpris;

#[cfg(feature = "dbus")]
mod mpris {
    use std::collections::HashMap;
    use std::sync::Arc;

    use anyhow::Result;
    use tokio::sync::{Mutex, broadcast, mpsc};
    use zbus::object_server::InterfaceRef;

    use super::player::{PlayerInterface, PlayerState};
    use super::root::RootInterface;
    use crate::api::ws::Notification;
    use crate::config::AppConfig;
    use crate::player::ZoneCommand;

    /// MPRIS2 bus name prefix.
    const BUS_NAME_PREFIX: &str = "org.mpris.MediaPlayer2.snapdog";

    /// Highest `.instanceN` suffix tried when the plain name is taken.
    const MAX_INSTANCE: u32 = 10;

    /// Highest zone number `dbus/snapdog.conf` allows callers to reach on the system bus.
    const POLICY_MAX_ZONES: usize = 16;

    /// Bus names to request for a zone in order: the plain name, then `.instance2.zoneN` to
    /// `.instance10.zoneN`.
    ///
    /// `dbus/snapdog.conf` has to allow sending to every one of these, name by name, for the
    /// zones up to [`POLICY_MAX_ZONES`] (see the test `policy_file_lists_every_bus_name`).
    fn bus_names(zone_index: usize) -> impl Iterator<Item = String> {
        std::iter::once(format!("{BUS_NAME_PREFIX}.zone{zone_index}")).chain(
            (2..=MAX_INSTANCE)
                .map(move |i| format!("{BUS_NAME_PREFIX}.instance{i}.zone{zone_index}")),
        )
    }

    /// Start MPRIS2 interfaces for all zones. Returns connections that must be kept alive.
    ///
    /// # Errors
    ///
    /// Returns an error if D-Bus connection or bus name registration fails.
    pub async fn start_mpris(
        config: &Arc<AppConfig>,
        zone_commands: &HashMap<usize, mpsc::Sender<ZoneCommand>>,
        notify_rx: broadcast::Sender<Arc<str>>,
    ) -> Result<Vec<zbus::Connection>> {
        let use_system = std::env::var("DBUS_SESSION_BUS_ADDRESS").is_err();

        let art_base = {
            let host = if config.http.bind == "::" || config.http.bind == "0.0.0.0" {
                "localhost"
            } else {
                &config.http.bind
            };
            format!("http://{host}:{}", config.http.port)
        };

        let mut connections = Vec::with_capacity(config.zones.len());

        for zone in &config.zones {
            let state = Arc::new(Mutex::new(PlayerState::default()));

            let root = RootInterface::new(&zone.name);
            let player = PlayerInterface::new(
                zone.index,
                art_base.clone(),
                Arc::clone(&state),
                zone_commands[&zone.index].clone(),
            );

            let conn = if use_system {
                zbus::Connection::system().await?
            } else {
                zbus::Connection::session().await?
            };

            // Serve the interfaces before taking the name, so no call reaches the name
            // before something answers it.
            conn.object_server()
                .at("/org/mpris/MediaPlayer2", root)
                .await?;
            conn.object_server()
                .at("/org/mpris/MediaPlayer2", player)
                .await?;

            // Request bus name, appending .instanceN if already taken
            let mut acquired = false;
            for name in bus_names(zone.index) {
                if conn.request_name(name.as_str()).await.is_ok() {
                    acquired = true;
                    break;
                }
            }
            if !acquired {
                tracing::warn!(zone = %zone.name, "Could not acquire D-Bus name, skipping");
                continue;
            }
            if use_system && zone.index > POLICY_MAX_ZONES {
                tracing::warn!(
                    zone = %zone.name,
                    index = zone.index,
                    max = POLICY_MAX_ZONES,
                    "Zone is above the ones the D-Bus policy lists: nobody can call its MPRIS2 \
                     player on the system bus until /usr/share/dbus-1/system.d/snapdog.conf names it"
                );
            }

            let iface_ref: InterfaceRef<PlayerInterface> = conn
                .object_server()
                .interface("/org/mpris/MediaPlayer2")
                .await?;

            let rx = notify_rx.subscribe();
            tokio::spawn(notification_updater(zone.index, state, iface_ref, rx));

            connections.push(conn);
        }

        Ok(connections)
    }

    #[allow(clippy::significant_drop_tightening)]
    async fn notification_updater(
        zone_index: usize,
        state: Arc<Mutex<PlayerState>>,
        iface_ref: InterfaceRef<PlayerInterface>,
        mut rx: broadcast::Receiver<Arc<str>>,
    ) {
        let emitter = iface_ref.signal_emitter();
        loop {
            let msg = match rx.recv().await {
                Ok(m) => m,
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => break,
            };

            let Ok(notif) = serde_json::from_str::<Notification>(&msg) else {
                continue;
            };

            match notif {
                Notification::ZoneChanged {
                    zone,
                    playback,
                    volume,
                    muted,
                    shuffle,
                    repeat,
                    title,
                    artist,
                    album,
                    duration_ms,
                    position_ms,
                    seekable,
                    can_next,
                    can_prev,
                    cover_url,
                    ..
                } if zone == zone_index => {
                    {
                        let mut s = state.lock().await;
                        s.playback = match playback.as_str() {
                            "playing" => "Playing",
                            "paused" => "Paused",
                            _ => "Stopped",
                        }
                        .into();
                        s.volume = volume;
                        s.muted = muted;
                        s.shuffle = shuffle;
                        s.repeat = repeat == snapdog_common::RepeatMode::Playlist;
                        s.track_repeat = repeat == snapdog_common::RepeatMode::Track;
                        s.title = title;
                        s.artist = artist;
                        s.album = album;
                        s.duration_ms = duration_ms;
                        s.position_ms = position_ms;
                        s.seekable = seekable;
                        s.can_next = can_next;
                        s.can_prev = can_prev;
                        s.cover_url = cover_url;
                    }
                    {
                        let iface = iface_ref.get().await;
                        let _ = iface.playback_status_changed(emitter).await;
                        let _ = iface.volume_changed(emitter).await;
                        let _ = iface.shuffle_changed(emitter).await;
                        let _ = iface.loop_status_changed(emitter).await;
                        let _ = iface.metadata_changed(emitter).await;
                        let _ = iface.can_seek_changed(emitter).await;
                        let _ = iface.can_go_next_changed(emitter).await;
                        let _ = iface.can_go_previous_changed(emitter).await;
                    }
                }
                Notification::ZoneVolumeChanged {
                    zone,
                    volume,
                    muted,
                } if zone == zone_index => {
                    {
                        let mut s = state.lock().await;
                        s.volume = volume;
                        s.muted = muted;
                    }
                    {
                        let iface = iface_ref.get().await;
                        let _ = iface.volume_changed(emitter).await;
                    }
                }
                Notification::ZoneProgress {
                    zone, position_ms, ..
                } if zone == zone_index => {
                    state.lock().await.position_ms = position_ms;
                }
                _ => {}
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        /// Values of the `<allow ATTR="...">` rules in a policy file.
        fn allow_rules(policy: &str, attr: &str) -> Vec<String> {
            let open = format!("<allow {attr}=\"");
            policy
                .lines()
                .filter_map(|line| {
                    line.trim()
                        .strip_prefix(open.as_str())?
                        .strip_suffix("\"/>")
                        .map(str::to_owned)
                })
                .collect()
        }

        /// The bus policy has to allow sending to every name `start_mpris` can end up
        /// owning, for every zone up to `POLICY_MAX_ZONES`.
        ///
        /// It lists them one by one. A `send_destination_prefix` rule would be shorter, but
        /// dbus-broker ignores it ("Unknown attribute", so every non-root caller is denied)
        /// and dbus-daemon 1.12 rejects the whole file.
        #[test]
        fn policy_file_lists_every_bus_name() {
            let policy = include_str!("../../dbus/snapdog.conf");
            let expected: Vec<String> = (1..=POLICY_MAX_ZONES).flat_map(bus_names).collect();
            let allowed = allow_rules(policy, "send_destination");
            let missing: Vec<_> = expected
                .iter()
                .filter(|n| !allowed.contains(n))
                .take(5)
                .collect();
            let unused: Vec<_> = allowed
                .iter()
                .filter(|n| !expected.contains(n))
                .take(5)
                .collect();
            assert!(
                missing.is_empty() && unused.is_empty(),
                "dbus/snapdog.conf and bus_names() disagree. Not in the file (first 5): \
                 {missing:?}. In the file but never requested (first 5): {unused:?}"
            );
            assert_eq!(allow_rules(policy, "own_prefix"), [BUS_NAME_PREFIX]);
            assert!(
                !policy.contains("send_destination_prefix=\""),
                "send_destination_prefix is not understood by dbus-broker or dbus-daemon 1.12"
            );
        }

        #[test]
        fn bus_names_try_the_plain_name_first_then_the_instances() {
            let names: Vec<String> = bus_names(3).collect();
            assert_eq!(names.len(), 10);
            assert_eq!(names[0], "org.mpris.MediaPlayer2.snapdog.zone3");
            assert_eq!(names[1], "org.mpris.MediaPlayer2.snapdog.instance2.zone3");
            assert_eq!(names[9], "org.mpris.MediaPlayer2.snapdog.instance10.zone3");
        }
    }
}
