// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! Inbound commands: topic and payload to zone or Snapcast command.
//!
//! Zone commands go through the per-zone `ZonePlayer` channel, client commands
//! to the Snapcast command channel. Nothing here blocks: a full channel drops
//! the command, so a stalled zone cannot hold up the MQTT event loop (and with
//! it the keep-alive).

use std::collections::HashMap;

use anyhow::Result;
use tokio::sync::mpsc;

use super::topics::{Route, TopicError, Topics};
use crate::player::{ClientAction, SnapcastCmd, ZoneCommand, ZoneCommandSender};
use crate::state;

/// Turns incoming MQTT messages into commands.
pub struct CommandHandler {
    topics: Topics,
    zone_commands: HashMap<usize, ZoneCommandSender>,
    state: state::SharedState,
    snap_tx: mpsc::Sender<SnapcastCmd>,
}

impl CommandHandler {
    /// A handler for the topics below `topics`.
    pub const fn new(
        topics: Topics,
        zone_commands: HashMap<usize, ZoneCommandSender>,
        state: state::SharedState,
        snap_tx: mpsc::Sender<SnapcastCmd>,
    ) -> Self {
        Self {
            topics,
            zone_commands,
            state,
            snap_tx,
        }
    }

    /// Handle one incoming message.
    ///
    /// A topic that is below the base but not a command is ignored (`Ok`).
    ///
    /// # Errors
    ///
    /// Returns an error for a topic outside the base or with an invalid index,
    /// an unparsable payload, or a command that could not be delivered.
    #[allow(clippy::too_many_lines)] // Match dispatcher for MQTT topics — splitting would obscure control flow
    pub async fn handle(&self, topic: &str, payload: &str) -> Result<()> {
        let parsed = match self.topics.parse_command(topic) {
            Ok(parsed) => parsed,
            Err(TopicError::UnknownRoute) => {
                tracing::debug!(topic = ?topic, "Unhandled MQTT command topic");
                return Ok(());
            }
            Err(e) => return Err(e.into()),
        };
        let index = parsed.index;

        match parsed.route {
            // Zone commands → routed through ZonePlayer
            Route::ZoneVolume => {
                let volume: f64 = payload.parse()?;
                // Accept both 0.0-1.0 (HA) and 0-100 (legacy)
                let volume_int = if volume <= 1.0 {
                    (volume * 100.0).round() as i32
                } else {
                    volume as i32
                };
                self.send_zone(index, ZoneCommand::SetVolume(volume_int.clamp(0, 100)))?;
            }
            Route::ZoneMute => {
                let muted = matches!(payload.trim(), "true" | "1" | "on");
                self.send_zone(index, ZoneCommand::SetMute(muted))?;
            }
            Route::ZoneControl => {
                let cmd = match payload.to_lowercase().as_str() {
                    "play" => Some(ZoneCommand::Play),
                    "pause" => Some(ZoneCommand::Pause),
                    "stop" => Some(ZoneCommand::Stop),
                    "next" => Some(ZoneCommand::Next),
                    "previous" => Some(ZoneCommand::Previous),
                    _ => {
                        tracing::debug!(zone = index, "Unknown control command");
                        None
                    }
                };
                if let Some(cmd) = cmd {
                    self.send_zone(index, cmd)?;
                }
            }
            Route::ZonePlaylist => {
                let playlist: usize = payload.parse()?;
                self.send_zone(index, ZoneCommand::SetPlaylist(playlist, 0))?;
            }
            Route::ZoneTrack => {
                let track: usize = payload.parse()?;
                self.send_zone(index, ZoneCommand::SetTrack(track))?;
            }
            Route::ZonePosition => {
                let pos_secs: f64 = payload.parse()?;
                let pos_ms = (pos_secs * 1000.0) as i64;
                self.send_zone(index, ZoneCommand::Seek(pos_ms))?;
            }
            Route::ZoneShuffle => {
                let shuffle = matches!(payload.trim(), "true" | "1" | "on");
                self.send_zone(index, ZoneCommand::SetShuffle(shuffle))?;
            }
            Route::ZoneRepeat => {
                let mode = match payload.trim() {
                    "one" | "track" => snapdog_common::RepeatMode::Track,
                    "all" | "playlist" => snapdog_common::RepeatMode::Playlist,
                    _ => snapdog_common::RepeatMode::Off,
                };
                self.send_zone(index, ZoneCommand::SetRepeat(mode))?;
            }
            Route::ZonePresence => {
                let present: bool = payload.parse()?;
                self.send_zone(index, ZoneCommand::SetPresence(present))?;
            }
            Route::ZonePresenceEnable => {
                let enabled: bool = payload.parse()?;
                self.send_zone(index, ZoneCommand::SetPresenceEnabled(enabled))?;
            }
            Route::ZonePresenceTimeout => {
                let delay: u16 = payload.parse()?;
                self.send_zone(index, ZoneCommand::SetAutoOffDelay(delay))?;
            }

            // Client commands → Snapcast commands or direct state mutation
            Route::ClientVolume => {
                let volume: i32 = payload.parse()?;
                self.send_client(index, ClientAction::Volume(volume.clamp(0, 100)))
                    .await?;
            }
            Route::ClientMute => {
                let muted: bool = payload.parse()?;
                self.send_client(index, ClientAction::Mute(muted)).await?;
            }
            Route::ClientLatency => {
                let latency: i32 = payload.parse()?;
                self.send_client(index, ClientAction::Latency(latency))
                    .await?;
            }
            Route::ClientZone => {
                let zone: usize = payload.parse()?;
                let mut store = self.state.write().await;
                if zone == 0 || !store.zones.contains_key(&zone) {
                    tracing::warn!(
                        client = index,
                        zone,
                        "Ignoring MQTT client zone change to unknown zone"
                    );
                    return Ok(());
                }
                let Some(client) = store.clients.get_mut(&index) else {
                    tracing::warn!(
                        client = index,
                        "Ignoring MQTT zone change for unknown client"
                    );
                    return Ok(());
                };
                client.zone_index = zone;
                store.dirty = true;
                drop(store);
                self.snap_tx.try_send(SnapcastCmd::ReconcileZones)?;
            }
        }

        Ok(())
    }

    /// Queue a command for zone `index`'s player without waiting.
    fn send_zone(&self, index: usize, cmd: ZoneCommand) -> Result<()> {
        let Some(tx) = self.zone_commands.get(&index) else {
            anyhow::bail!("no zone {index} to send the command to");
        };
        tx.try_send(cmd)
            .map_err(|e| anyhow::anyhow!("zone {index} command channel: {e}"))
    }

    /// Queue `action` for client `index` if it is known to Snapcast. A client
    /// that is not connected (no Snapcast id yet) has nothing to receive it.
    async fn send_client(&self, index: usize, action: ClientAction) -> Result<()> {
        let snap_id = {
            let store = self.state.read().await;
            store
                .clients
                .get(&index)
                .and_then(|c| c.snapcast_id.clone())
        };
        let Some(client_id) = snap_id else {
            tracing::debug!(
                client = index,
                "MQTT client command for a client without Snapcast id"
            );
            return Ok(());
        };
        self.snap_tx
            .try_send(SnapcastCmd::Client { client_id, action })
            .map_err(|e| anyhow::anyhow!("Snapcast command channel: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mqtt::test_support::{snap_channel, test_state_with_client, zone_channels};

    /// A handler on `base` plus the receivers of its channels.
    fn handler(
        base: &str,
    ) -> (
        CommandHandler,
        mpsc::Receiver<ZoneCommand>,
        mpsc::Receiver<SnapcastCmd>,
        state::SharedState,
    ) {
        let (snap_tx, snap_rx) = snap_channel();
        let (cmds, zone_rx) = zone_channels();
        let state = test_state_with_client();
        (
            CommandHandler::new(Topics::new(base), cmds, state.clone(), snap_tx),
            zone_rx,
            snap_rx,
            state,
        )
    }

    #[tokio::test]
    async fn routes_zone_volume() {
        let (h, mut rx, _snap, _state) = handler("snapdog/");
        h.handle("snapdog/zones/1/volume/set", "75").await.unwrap();
        assert!(matches!(rx.recv().await, Some(ZoneCommand::SetVolume(75))));
    }

    #[tokio::test]
    async fn routes_zone_mute() {
        let (h, mut rx, _snap, _state) = handler("snapdog/");
        h.handle("snapdog/zones/1/mute/set", "true").await.unwrap();
        assert!(matches!(rx.recv().await, Some(ZoneCommand::SetMute(true))));
    }

    #[tokio::test]
    async fn routes_zone_control_commands() {
        let (h, mut rx, _snap, _state) = handler("snapdog/");
        for (payload, expected) in [
            ("play", ZoneCommand::Play),
            ("pause", ZoneCommand::Pause),
            ("stop", ZoneCommand::Stop),
            ("next", ZoneCommand::Next),
            ("previous", ZoneCommand::Previous),
        ] {
            h.handle("snapdog/zones/1/control/set", payload)
                .await
                .unwrap();
            let cmd = rx.recv().await.unwrap();
            assert_eq!(
                std::mem::discriminant(&cmd),
                std::mem::discriminant(&expected)
            );
        }
    }

    #[tokio::test]
    async fn routes_zone_playlist() {
        let (h, mut rx, _snap, _state) = handler("snapdog/");
        h.handle("snapdog/zones/1/playlist/set", "3").await.unwrap();
        assert!(matches!(
            rx.recv().await,
            Some(ZoneCommand::SetPlaylist(3, 0))
        ));
    }

    #[tokio::test]
    async fn routes_zone_seek() {
        let (h, mut rx, _snap, _state) = handler("snapdog/");
        h.handle("snapdog/zones/1/track/position/set", "30")
            .await
            .unwrap();
        assert!(matches!(rx.recv().await, Some(ZoneCommand::Seek(30000))));
    }

    #[tokio::test]
    async fn routes_client_volume() {
        let (h, _rx, mut snap_rx, _state) = handler("snapdog/");
        h.handle("snapdog/clients/1/volume/set", "80")
            .await
            .unwrap();
        let cmd = snap_rx.recv().await.unwrap();
        assert!(
            matches!(cmd, SnapcastCmd::Client { client_id, action: ClientAction::Volume(80) } if client_id == "snap-1")
        );
    }

    #[tokio::test]
    async fn routes_client_mute() {
        let (h, _rx, mut snap_rx, _state) = handler("snapdog/");
        h.handle("snapdog/clients/1/mute/set", "true")
            .await
            .unwrap();
        let cmd = snap_rx.recv().await.unwrap();
        assert!(
            matches!(cmd, SnapcastCmd::Client { client_id, action: ClientAction::Mute(true) } if client_id == "snap-1")
        );
    }

    #[tokio::test]
    async fn routes_client_zone_change() {
        let (h, _rx, mut snap_rx, state) = handler("snapdog/");
        h.handle("snapdog/clients/1/zone/set", "2").await.unwrap();
        assert_eq!(state.read().await.clients[&1].zone_index, 2);
        let cmd = snap_rx.recv().await.unwrap();
        assert!(matches!(cmd, SnapcastCmd::ReconcileZones));
    }

    #[tokio::test]
    async fn rejects_client_zone_change_to_unknown_zone() {
        let (h, _rx, mut snap_rx, state) = handler("snapdog/");
        h.handle("snapdog/clients/1/zone/set", "99").await.unwrap();
        assert_eq!(state.read().await.clients[&1].zone_index, 1);
        assert!(snap_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn clamps_client_volume() {
        let (h, _rx, mut snap_rx, _state) = handler("snapdog/");
        h.handle("snapdog/clients/1/volume/set", "200")
            .await
            .unwrap();
        let cmd = snap_rx.recv().await.unwrap();
        assert!(matches!(
            cmd,
            SnapcastCmd::Client {
                action: ClientAction::Volume(100),
                ..
            }
        ));
    }

    // ── Regression: command topics under the real, normalised base topic ──
    //
    // `config` appends the trailing slash ("snapdog" becomes "snapdog/"). The bridge
    // once re-derived the base by hand when parsing, so every inbound command was
    // dropped in production while the tests (which used "snapdog" without slash)
    // passed. These tests go through the config loader and take the topics from the
    // function that builds the subscriptions.

    /// The `[mqtt]` section as the config loader resolves it.
    fn mqtt_config(base_topic: Option<&str>) -> crate::config::MqttConfig {
        let base_line = base_topic.map_or(String::new(), |b| format!("base_topic = \"{b}\""));
        let toml = format!(
            r#"
            [mqtt]
            broker = "localhost:1883"
            {base_line}

            [[zone]]
            name = "Z1"
            [[zone]]
            name = "Z2"
            [[client]]
            name = "C1"
            mac = "00:00:00:00:00:01"
            zone = "Z1"
            "#
        );
        crate::config::load_raw(toml::from_str(&toml).unwrap())
            .unwrap()
            .mqtt
            .unwrap()
    }

    /// A payload every command accepts.
    fn valid_payload(tail: &str) -> &'static str {
        match tail {
            "volume/set" => "50",
            "mute/set" | "shuffle/set" | "presence/set" | "presence/enable/set" => "true",
            "control/set" => "play",
            "repeat/set" => "all",
            "position/set" | "track/position/set" => "30",
            "track/set" | "zone/set" => "2",
            "playlist/set" => "1",
            "presence/timeout/set" => "60",
            "latency/set" => "20",
            other => panic!("no sample payload for {other}: extend valid_payload"),
        }
    }

    #[tokio::test]
    async fn command_on_default_normalised_base_is_not_dropped() {
        // The exact topic from the bug report, with the base the config really produces.
        let mqtt = mqtt_config(None);
        assert_eq!(mqtt.base_topic, "snapdog/");
        let (h, mut rx, _snap, _state) = handler(&mqtt.base_topic);
        h.handle("snapdog/zones/1/volume/set", "75").await.unwrap();
        assert!(matches!(rx.recv().await, Some(ZoneCommand::SetVolume(75))));
    }

    #[tokio::test]
    async fn every_subscribed_topic_is_handled_under_the_real_config_base() {
        for (configured, effective) in [
            (None, "snapdog/"),
            (Some("snapdog"), "snapdog/"),
            (Some("snapdog/"), "snapdog/"),
            (Some("home/audio"), "home/audio/"),
            (Some("home/audio/"), "home/audio/"),
        ] {
            let mqtt = mqtt_config(configured);
            assert_eq!(mqtt.base_topic, effective, "config normalisation");
            // The same function the bridge subscribes with.
            let filters: Vec<String> = Topics::new(&mqtt.base_topic).command_filters().collect();
            assert_eq!(
                filters.len(),
                16,
                "documented command set, base {effective}"
            );

            for filter in filters {
                assert!(filter.starts_with(effective), "{filter}");
                // A concrete topic that this very subscription delivers.
                let topic = filter.replace('+', "1");
                let tail = topic
                    .strip_prefix(effective)
                    .and_then(|t| t.split_once("/1/"))
                    .map(|(_, tail)| tail)
                    .expect("scope/1/tail");

                let (h, mut zone_rx, mut snap_rx, _state) = handler(&mqtt.base_topic);
                h.handle(&topic, valid_payload(tail))
                    .await
                    .unwrap_or_else(|e| panic!("{topic} was rejected: {e}"));
                assert!(
                    zone_rx.try_recv().is_ok() || snap_rx.try_recv().is_ok(),
                    "{topic} (subscribed) produced no command"
                );
            }
        }
    }

    #[tokio::test]
    async fn ignores_unknown_topic_below_the_base() {
        let (h, mut rx, mut snap_rx, _state) = handler("snapdog/");
        h.handle("snapdog/unknown/topic", "x").await.unwrap();
        assert!(rx.try_recv().is_err() && snap_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn rejects_topic_outside_the_base() {
        let (h, _rx, _snap, _state) = handler("snapdog/");
        for topic in ["other/zones/1/volume/set", "snapdogs/zones/1/volume/set"] {
            assert!(h.handle(topic, "50").await.is_err(), "{topic}");
        }
    }

    #[tokio::test]
    async fn a_full_zone_channel_drops_the_command_instead_of_blocking() {
        let (snap_tx, _snap_rx) = snap_channel();
        let (cmds, mut rx) = crate::mqtt::test_support::zone_channels_with(1);
        let h = CommandHandler::new(
            Topics::new("snapdog/"),
            cmds,
            test_state_with_client(),
            snap_tx,
        );
        h.handle("snapdog/zones/1/volume/set", "10").await.unwrap();
        // The channel holds one command and nobody receives: this must return, not hang.
        let second = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            h.handle("snapdog/zones/1/volume/set", "20"),
        )
        .await
        .expect("handle must not block on a full channel");
        assert!(second.is_err());
        assert!(matches!(rx.try_recv(), Ok(ZoneCommand::SetVolume(10))));
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn unknown_zone_is_an_error() {
        let (h, _rx, _snap, _state) = handler("snapdog/");
        assert!(h.handle("snapdog/zones/2/volume/set", "10").await.is_err());
    }
}
