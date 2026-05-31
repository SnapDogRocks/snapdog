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

            // Request bus name, appending .instanceN if already taken
            let base_name = format!("{BUS_NAME_PREFIX}.zone{}", zone.index);
            let acquired = if conn.request_name(base_name.as_str()).await.is_ok() {
                true
            } else {
                let mut ok = false;
                for i in 2..=10 {
                    let name = format!("{BUS_NAME_PREFIX}.instance{i}.zone{}", zone.index);
                    if conn.request_name(name.as_str()).await.is_ok() {
                        ok = true;
                        break;
                    }
                }
                ok
            };
            if !acquired {
                tracing::warn!(zone = %zone.name, "Could not acquire D-Bus name, skipping");
                continue;
            }

            conn.object_server()
                .at("/org/mpris/MediaPlayer2", root)
                .await?;
            conn.object_server()
                .at("/org/mpris/MediaPlayer2", player)
                .await?;

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
                Notification::ZoneStateChanged {
                    zone,
                    playback,
                    volume,
                    muted,
                    shuffle,
                    repeat,
                    track_repeat,
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
                        s.repeat = repeat;
                        s.track_repeat = track_repeat;
                    }
                    {
                        let iface = iface_ref.get().await;
                        let _ = iface.playback_status_changed(emitter).await;
                        let _ = iface.volume_changed(emitter).await;
                        let _ = iface.shuffle_changed(emitter).await;
                        let _ = iface.loop_status_changed(emitter).await;
                    }
                }
                Notification::ZoneTrackChanged {
                    zone,
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
                        let _ = iface.metadata_changed(emitter).await;
                        let _ = iface.can_seek_changed(emitter).await;
                        let _ = iface.can_go_next_changed(emitter).await;
                        let _ = iface.can_go_previous_changed(emitter).await;
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
}
