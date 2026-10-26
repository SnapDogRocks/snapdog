// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! MPRIS2 D-Bus interface — one player per zone.

#[cfg(feature = "dbus")]
mod player;
/// Checks shared with the client's policy test (well-formed XML, comments).
#[cfg(all(test, feature = "dbus"))]
#[path = "../../../snapdog-common/test-support/policy_xml.rs"]
mod policy_xml;
#[cfg(feature = "dbus")]
mod root;
#[cfg(all(test, unix, feature = "dbus"))]
mod testing;

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
    /// zones up to [`POLICY_MAX_ZONES`], and nothing else (see the test
    /// `policy_file_is_exactly_what_bus_names_requires`).
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
        use crate::dbus::policy_xml;

        /// The policy file with its comments removed, as trimmed, non-empty lines.
        fn policy_lines(policy: &str) -> Vec<String> {
            policy_xml::strip_comments(policy)
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .map(str::to_owned)
                .collect()
        }

        /// Every line the policy file is allowed to contain, in order. The file grants
        /// exactly this and nothing else.
        fn expected_policy() -> Vec<String> {
            let mut lines: Vec<String> = [
                r#"<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-BUS Bus Configuration 1.0//EN""#,
                r#""http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">"#,
                "<busconfig>",
                r#"<policy context="default">"#,
            ]
            .map(str::to_owned)
            .into();
            lines.push(format!(r#"<allow own_prefix="{BUS_NAME_PREFIX}"/>"#));
            lines.extend(
                (1..=POLICY_MAX_ZONES)
                    .flat_map(bus_names)
                    .map(|n| format!(r#"<allow send_destination="{n}"/>"#)),
            );
            lines.extend(["</policy>", "</busconfig>"].map(str::to_owned));
            lines
        }

        /// The bus policy has to allow sending to every name `start_mpris` can end up
        /// owning, for every zone up to `POLICY_MAX_ZONES`, and nothing besides.
        ///
        /// It lists them one by one. A `send_destination_prefix` rule would be shorter, but
        /// dbus-broker ignores it ("Unknown attribute", so every non-root caller is denied)
        /// and dbus-daemon 1.12 rejects the whole file.
        ///
        /// The whole file is compared, not only the rules that should be there: an extra
        /// `<allow>` (a wildcard `own`, a `send_type`, a prefix rule), a `<deny>`, a second
        /// or different `<policy>` context or a duplicated line each grant or change
        /// something that nobody reviewed, and each fails here.
        #[test]
        fn policy_file_is_exactly_what_bus_names_requires() {
            let policy = include_str!("../../dbus/snapdog.conf");
            policy_xml::assert_well_formed(policy);
            let actual = policy_lines(policy);
            let expected = expected_policy();

            let missing: Vec<_> = expected
                .iter()
                .filter(|l| !actual.contains(l))
                .take(5)
                .collect();
            let unexpected: Vec<_> = actual
                .iter()
                .filter(|l| !expected.contains(l))
                .take(5)
                .collect();
            assert!(
                missing.is_empty() && unexpected.is_empty(),
                "dbus/snapdog.conf and bus_names() disagree. Missing (first 5): {missing:?}. \
                 Not allowed to be there (first 5): {unexpected:?}"
            );
            assert_eq!(
                actual.len(),
                expected.len(),
                "a rule is duplicated, or a line is missing or extra"
            );
            assert_eq!(actual, expected, "same lines, different order or structure");

            // The same facts as named checks, for a readable failure.
            let rules = actual.join("\n");
            assert_eq!(
                rules.matches("<policy").count(),
                1,
                "one policy context only"
            );
            assert!(
                !rules.contains("<deny"),
                "everything not allowed stays denied by default"
            );
            assert!(
                !rules.contains("own=\""),
                "no `own` wildcard or name outside the prefix"
            );
            assert!(
                !rules.contains("send_destination_prefix=\""),
                "send_destination_prefix is not understood by dbus-broker or dbus-daemon 1.12"
            );
            assert!(
                !rules.contains("send_type") && !rules.contains("send_interface"),
                "rules that match on type or interface instead of destination are broader"
            );
            assert!(policy.ends_with('\n'), "the file ends with a newline");
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

    /// `notification_updater` against a real interface on a peer-to-peer connection: what a
    /// notification from the zone does to the state and to what a D-Bus caller reads.
    #[cfg(all(test, unix))]
    mod notification_tests {
        use std::time::Duration;

        use super::*;
        use crate::dbus::testing::{
            DEADLINE, PATH, ZONE, fixture, get_bool, get_f64, get_i64, get_property, get_string,
        };

        /// A `ZoneChanged` for `zone` with every field set to something other than the
        /// default of `PlayerState`.
        fn zone_changed(zone: usize, playback: &str) -> Notification {
            Notification::ZoneChanged {
                zone,
                playback: playback.to_owned(),
                source: "subsonic".to_owned(),
                shuffle: true,
                repeat: snapdog_common::RepeatMode::Playlist,
                title: "Title".to_owned(),
                artist: "Artist".to_owned(),
                album: "Album".to_owned(),
                album_artist: None,
                genre: None,
                year: None,
                track_number: None,
                disc_number: None,
                duration_ms: 200_000,
                position_ms: 5_000,
                seekable: true,
                cover_url: Some("/api/v1/zones/1/cover".to_owned()),
                bitrate_kbps: None,
                content_type: None,
                track_index: None,
                track_count: None,
                playlist: None,
                playlist_name: None,
                playlist_total: None,
                can_playlist_next: false,
                can_playlist_prev: false,
                can_next: true,
                can_prev: true,
                volume: 40,
                muted: false,
            }
        }

        fn publish(tx: &broadcast::Sender<Arc<str>>, notification: &Notification) {
            let json = serde_json::to_string(notification).expect("serialize");
            tx.send(json.into()).expect("the updater is subscribed");
        }

        /// Poll until the property equals `expected`: the updater runs on its own task.
        async fn wait_for_string(caller: &zbus::Connection, name: &str, expected: &str) {
            let deadline = tokio::time::Instant::now() + DEADLINE;
            loop {
                let got = get_string(caller, name).await;
                if got == expected {
                    return;
                }
                assert!(
                    tokio::time::Instant::now() < deadline,
                    "{name} is still {got:?}, expected {expected:?}"
                );
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }

        async fn wait_for_volume(caller: &zbus::Connection, volume: f64) {
            let deadline = tokio::time::Instant::now() + DEADLINE;
            loop {
                if (get_f64(caller, "Volume").await - volume).abs() < 1e-9 {
                    return;
                }
                assert!(
                    tokio::time::Instant::now() < deadline,
                    "the volume never reached {volume}"
                );
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }

        async fn wait_for_position(caller: &zbus::Connection, micros: i64) {
            let deadline = tokio::time::Instant::now() + DEADLINE;
            while get_i64(caller, "Position").await != micros {
                assert!(
                    tokio::time::Instant::now() < deadline,
                    "the position never reached {micros}"
                );
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }

        async fn updater_fixture() -> (crate::dbus::testing::Fixture, broadcast::Sender<Arc<str>>) {
            let f = fixture().await;
            let iface_ref: InterfaceRef<PlayerInterface> = f
                .server
                .object_server()
                .interface(PATH)
                .await
                .expect("interface is served");
            let (tx, rx) = broadcast::channel(16);
            tokio::spawn(notification_updater(
                ZONE,
                Arc::clone(&f.state),
                iface_ref,
                rx,
            ));
            (f, tx)
        }

        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn playback_maps_to_the_mpris_playback_status() {
            let (f, tx) = updater_fixture().await;
            assert_eq!(get_string(&f.caller, "PlaybackStatus").await, "Stopped");
            for (playback, status) in [
                ("playing", "Playing"),
                ("paused", "Paused"),
                ("stopped", "Stopped"),
                ("playing", "Playing"),
                ("buffering", "Stopped"),
            ] {
                publish(&tx, &zone_changed(ZONE, playback));
                wait_for_string(&f.caller, "PlaybackStatus", status).await;
            }
        }

        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn zone_changed_reaches_the_state_and_the_properties() {
            let (f, tx) = updater_fixture().await;
            publish(&tx, &zone_changed(ZONE, "playing"));
            wait_for_string(&f.caller, "PlaybackStatus", "Playing").await;

            assert!((get_f64(&f.caller, "Volume").await - 0.4).abs() < 1e-9);
            assert!(get_bool(&f.caller, "Shuffle").await);
            assert_eq!(get_string(&f.caller, "LoopStatus").await, "Playlist");
            assert!(get_bool(&f.caller, "CanSeek").await);
            assert!(get_bool(&f.caller, "CanGoNext").await);
            assert!(get_bool(&f.caller, "CanGoPrevious").await);
            assert_eq!(get_i64(&f.caller, "Position").await, 5_000_000);
            let metadata: HashMap<String, zbus::OwnedVariant> = get_property(&f.caller, "Metadata")
                .await
                .try_into()
                .expect("a{sv}");
            assert_eq!(
                String::try_from(metadata["xesam:title"].try_clone().expect("clone")).ok(),
                Some("Title".to_owned())
            );
            assert_eq!(
                Vec::<String>::try_from(metadata["xesam:artist"].try_clone().expect("clone")).ok(),
                Some(vec!["Artist".to_owned()])
            );
            assert_eq!(
                String::try_from(metadata["xesam:album"].try_clone().expect("clone")).ok(),
                Some("Album".to_owned())
            );
            assert_eq!(
                i64::try_from(metadata["mpris:length"].try_clone().expect("clone")).ok(),
                Some(200_000_000)
            );
            assert_eq!(
                String::try_from(metadata["mpris:artUrl"].try_clone().expect("clone")).ok(),
                Some("http://localhost:5555/api/v1/zones/1/cover".to_owned())
            );

            // Repeat modes: Track and Off, besides the Playlist above.
            let mut track = zone_changed(ZONE, "playing");
            if let Notification::ZoneChanged { repeat, .. } = &mut track {
                *repeat = snapdog_common::RepeatMode::Track;
            }
            publish(&tx, &track);
            wait_for_string(&f.caller, "LoopStatus", "Track").await;
            let mut off = zone_changed(ZONE, "playing");
            if let Notification::ZoneChanged { repeat, .. } = &mut off {
                *repeat = snapdog_common::RepeatMode::Off;
            }
            publish(&tx, &off);
            wait_for_string(&f.caller, "LoopStatus", "None").await;

            // A muted zone reads as volume 0.0, whatever the volume is.
            let mut muted = zone_changed(ZONE, "playing");
            if let Notification::ZoneChanged { muted: m, .. } = &mut muted {
                *m = true;
            }
            publish(&tx, &muted);
            wait_for_volume(&f.caller, 0.0).await;
        }

        /// Notifications for another zone must change nothing. The updater handles them in
        /// order, so once a marker from the zone itself has arrived, the other zone's
        /// notifications before it are done. The marker must not overwrite what the
        /// notification under test would change, hence one round per kind.
        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn notifications_for_other_zones_are_ignored() {
            let (f, tx) = updater_fixture().await;
            publish(&tx, &zone_changed(ZONE, "playing"));
            wait_for_string(&f.caller, "PlaybackStatus", "Playing").await;
            let progress = |zone, position_ms| Notification::ZoneProgress {
                zone,
                position_ms,
                duration_ms: 200_000,
                buffered_ms: None,
            };
            publish(&tx, &progress(ZONE, 7_000));
            wait_for_position(&f.caller, 7_000_000).await;

            // Round 1: full state and volume of another zone, marker is a position.
            let mut other = zone_changed(ZONE + 1, "paused");
            if let Notification::ZoneChanged { volume, muted, .. } = &mut other {
                *volume = 5;
                *muted = true;
            }
            publish(&tx, &other);
            publish(
                &tx,
                &Notification::ZoneVolumeChanged {
                    zone: ZONE + 1,
                    volume: 1,
                    muted: true,
                },
            );
            publish(&tx, &progress(ZONE, 8_000));
            wait_for_position(&f.caller, 8_000_000).await;
            assert_eq!(get_string(&f.caller, "PlaybackStatus").await, "Playing");
            assert!((get_f64(&f.caller, "Volume").await - 0.4).abs() < 1e-9);

            // Round 2: another zone's progress, marker is a volume.
            publish(&tx, &progress(ZONE + 1, 1));
            publish(
                &tx,
                &Notification::ZoneVolumeChanged {
                    zone: ZONE,
                    volume: 60,
                    muted: false,
                },
            );
            wait_for_volume(&f.caller, 0.6).await;
            assert_eq!(get_i64(&f.caller, "Position").await, 8_000_000);
        }

        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn volume_and_progress_notifications_update_the_state() {
            let (f, tx) = updater_fixture().await;
            publish(
                &tx,
                &Notification::ZoneVolumeChanged {
                    zone: ZONE,
                    volume: 70,
                    muted: false,
                },
            );
            publish(
                &tx,
                &Notification::ZoneProgress {
                    zone: ZONE,
                    position_ms: 12_000,
                    duration_ms: 20_000,
                    buffered_ms: None,
                },
            );
            wait_for_position(&f.caller, 12_000_000).await;
            assert!((get_f64(&f.caller, "Volume").await - 0.7).abs() < 1e-9);
            publish(
                &tx,
                &Notification::ZoneVolumeChanged {
                    zone: ZONE,
                    volume: 70,
                    muted: true,
                },
            );
            wait_for_volume(&f.caller, 0.0).await;
        }
    }
}
