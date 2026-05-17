// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! MQTT integration via rumqttc.
//!
//! Bidirectional: subscribes to command topics (`*/set`), publishes status updates.
//! All incoming commands are routed through ZonePlayer command channels.

use std::collections::HashMap;

use anyhow::{Context, Result};
use rumqttc::{AsyncClient, Event, LastWill, MqttOptions, Packet, QoS};

use crate::config::MqttConfig;
use crate::player::{ClientAction, SnapcastCmd, ZoneCommand, ZoneCommandSender};
use crate::state;

/// MQTT keep-alive interval.
const MQTT_KEEP_ALIVE: std::time::Duration = std::time::Duration::from_secs(60);
/// Delay before MQTT reconnection attempt.
const MQTT_RECONNECT_DELAY: std::time::Duration = std::time::Duration::from_secs(5);
/// Event loop channel capacity for MQTT messages.
const MQTT_EVENT_CAPACITY: usize = 64;

/// MQTT bridge: receives commands, publishes status.
pub struct MqttBridge {
    client: AsyncClient,
    eventloop: rumqttc::EventLoop,
    base_topic: String,
    base_url: String,
}

impl MqttBridge {
    /// Connect to MQTT broker.
    #[tracing::instrument(skip_all, fields(broker = %config.broker))]
    pub async fn connect(config: &MqttConfig, base_url: &str) -> Result<Self> {
        let mut opts = MqttOptions::new(
            &config.client_id,
            parse_host(&config.broker),
            parse_port(&config.broker)?,
        );
        opts.set_keep_alive(MQTT_KEEP_ALIVE);
        if !config.username.is_empty() {
            opts.set_credentials(&config.username, &*config.password);
        }

        let status_topic = format!("{}/status", config.base_topic.trim_end_matches('/'));
        opts.set_last_will(LastWill::new(
            &status_topic,
            "offline",
            QoS::AtLeastOnce,
            true,
        ));

        let (client, eventloop) = AsyncClient::new(opts, MQTT_EVENT_CAPACITY);
        tracing::info!("MQTT connected");

        Ok(Self {
            client,
            eventloop,
            base_topic: config.base_topic.trim_end_matches('/').to_string(),
            base_url: base_url.trim_end_matches('/').to_string(),
        })
    }

    /// Create a disconnected bridge for testing (command routing only).
    #[cfg(test)]
    pub(crate) fn test_bridge(base_topic: &str) -> Self {
        let opts = MqttOptions::new("test", "localhost", 1883);
        let (client, eventloop) = AsyncClient::new(opts, 4);
        Self {
            client,
            eventloop,
            base_topic: base_topic.to_string(),
            base_url: "http://localhost:5555".to_string(),
        }
    }

    /// Subscribe to all command topics.
    pub async fn subscribe_commands(&self) -> Result<()> {
        let topics = [
            "zones/+/volume/set",
            "zones/+/mute/set",
            "zones/+/control/set",
            "zones/+/shuffle/set",
            "zones/+/repeat/set",
            "zones/+/position/set",
            "zones/+/track/set",
            "zones/+/track/position/set",
            "zones/+/playlist/set",
            "zones/+/presence/set",
            "zones/+/presence/enable/set",
            "zones/+/presence/timeout/set",
            "clients/+/volume/set",
            "clients/+/mute/set",
            "clients/+/latency/set",
            "clients/+/zone/set",
        ];

        for topic in &topics {
            self.client
                .subscribe(format!("{}/{topic}", self.base_topic), QoS::AtLeastOnce)
                .await
                .with_context(|| format!("Failed to subscribe to {topic}"))?;
        }

        tracing::info!(topics = topics.len(), "MQTT subscribed");
        Ok(())
    }

    /// Publish Home Assistant MQTT Discovery messages for all zones.
    pub async fn publish_ha_discovery(&self, zones: &[crate::config::ZoneConfig]) -> Result<()> {
        for zone in zones {
            let idx = zone.index;
            let unique_id = format!("snapdog_zone_{idx}");
            let base = &self.base_topic;

            let discovery = serde_json::json!({
                "name": zone.name,
                "unique_id": &unique_id,
                "object_id": &unique_id,
                "icon": "mdi:speaker-group",
                "state_topic": format!("{base}/zones/{idx}/state"),
                "volume_command_topic": format!("{base}/zones/{idx}/volume/set"),
                "mute_command_topic": format!("{base}/zones/{idx}/mute/set"),
                "media_position_command_topic": format!("{base}/zones/{idx}/position/set"),
                "payload_play": "play",
                "payload_pause": "pause",
                "payload_stop": "stop",
                "payload_next": "next",
                "payload_previous": "previous",
                "command_topic": format!("{base}/zones/{idx}/control/set"),
                "shuffle_command_topic": format!("{base}/zones/{idx}/shuffle/set"),
                "repeat_command_topic": format!("{base}/zones/{idx}/repeat/set"),
                "volume_level_template": "{{ value_json.volume_level }}",
                "is_volume_muted_template": "{{ value_json.is_volume_muted }}",
                "state_template": "{{ value_json.state }}",
                "media_title_template": "{{ value_json.media_title | default('') }}",
                "media_artist_template": "{{ value_json.media_artist | default('') }}",
                "media_album_name_template": "{{ value_json.media_album_name | default('') }}",
                "media_duration_template": "{{ value_json.media_duration | default(0) }}",
                "media_position_template": "{{ value_json.media_position | default(0) }}",
                "media_image_url_template": "{{ value_json.media_image_url | default('') }}",
                "shuffle_state_template": "{{ value_json.shuffle }}",
                "repeat_state_template": "{{ value_json.repeat }}",
                "supported_features": ["play", "pause", "stop", "next_track", "previous_track", "volume_set", "volume_mute", "shuffle_set", "repeat_set", "seek"],
                "device": {
                    "identifiers": ["snapdog"],
                    "name": "SnapDog",
                    "manufacturer": "metaneutrons",
                    "model": "Multi-zone Audio Controller",
                    "sw_version": env!("CARGO_PKG_VERSION"),
                },
                "availability_topic": format!("{base}/status"),
                "payload_available": "online",
                "payload_not_available": "offline",
            });

            self.client
                .publish(
                    format!("homeassistant/media_player/{unique_id}/config"),
                    QoS::AtLeastOnce,
                    true,
                    discovery.to_string().as_bytes(),
                )
                .await
                .with_context(|| format!("Failed to publish HA discovery for zone {idx}"))?;
        }

        // Publish availability
        self.client
            .publish(
                format!("{}/status", self.base_topic),
                QoS::AtLeastOnce,
                true,
                b"online",
            )
            .await?;

        tracing::info!(zones = zones.len(), "HA MQTT Discovery published");
        Ok(())
    }

    /// Publish a status value (retained).
    pub async fn publish(&self, topic: &str, payload: &str) -> Result<()> {
        self.client
            .publish(
                format!("{}/{topic}", self.base_topic),
                QoS::AtLeastOnce,
                true,
                payload.as_bytes(),
            )
            .await
            .with_context(|| format!("Failed to publish to {topic}"))
    }

    /// Publish zone state as a single retained JSON object.
    pub async fn publish_zone_state(&self, index: usize, zone: &state::ZoneState) -> Result<()> {
        let state_str = match zone.playback {
            state::PlaybackState::Playing => "playing",
            state::PlaybackState::Paused => "paused",
            state::PlaybackState::Stopped => {
                if zone.source == state::SourceType::Idle {
                    "idle"
                } else {
                    "stopped"
                }
            }
        };

        let mut payload = serde_json::json!({
            "state": state_str,
            "volume_level": f64::from(zone.volume) / 100.0,
            "is_volume_muted": zone.muted,
            "shuffle": zone.shuffle,
            "repeat": if zone.track_repeat { "one" } else if zone.repeat { "all" } else { "off" },
            "source": zone.source.to_string(),
            "presence": zone.presence,
        });

        if let Some(track) = &zone.track {
            payload["media_title"] = serde_json::json!(track.title);
            payload["media_artist"] = serde_json::json!(track.artist);
            payload["media_album_name"] = serde_json::json!(track.album);
            payload["media_duration"] = serde_json::json!(track.duration_ms / 1000);
            payload["media_position"] = serde_json::json!(track.position_ms / 1000);
            payload["media_content_type"] = serde_json::json!("music");
        }

        if let Some(ref cover_url) = zone.cover_url {
            let absolute = if cover_url.starts_with('/') {
                format!("{}{cover_url}", self.base_url)
            } else {
                cover_url.clone()
            };
            payload["media_image_url"] = serde_json::json!(absolute);
        }

        self.publish(&format!("zones/{index}/state"), &payload.to_string())
            .await
    }

    /// Publish client state as a single retained JSON object.
    pub async fn publish_client_state(
        &self,
        index: usize,
        client: &state::ClientState,
    ) -> Result<()> {
        let payload = serde_json::json!({
            "volume": client.base_volume,
            "muted": client.muted,
            "connected": client.connected,
            "zone": client.zone_index,
            "latency": client.latency_ms,
        });
        self.publish(&format!("clients/{index}/state"), &payload.to_string())
            .await
    }

    /// Run the event loop, dispatching incoming commands via ZonePlayer channels.
    pub async fn run(
        &mut self,
        zone_commands: HashMap<usize, ZoneCommandSender>,
        state: state::SharedState,
        snap_tx: tokio::sync::mpsc::Sender<SnapcastCmd>,
    ) -> Result<()> {
        loop {
            self.poll_once(&zone_commands, &state, &snap_tx).await;
        }
    }

    /// Poll for a single MQTT event. Returns when one event is processed.
    pub async fn poll_once(
        &mut self,
        zone_commands: &HashMap<usize, ZoneCommandSender>,
        state: &state::SharedState,
        snap_tx: &tokio::sync::mpsc::Sender<SnapcastCmd>,
    ) {
        match self.eventloop.poll().await {
            Ok(Event::Incoming(Packet::Publish(msg))) => {
                let topic = msg.topic.clone();
                let payload = String::from_utf8_lossy(&msg.payload).to_string();
                tracing::debug!(topic = %topic, payload = %payload, "MQTT message received");
                if let Err(e) = self
                    .handle_command(&topic, &payload, zone_commands, state, snap_tx)
                    .await
                {
                    tracing::warn!(error = %e, topic = %topic, "Failed to handle MQTT command");
                }
            }
            Ok(_) => {}
            Err(e) => {
                tracing::warn!(error = %e, "MQTT connection error, retrying");
                tokio::time::sleep(MQTT_RECONNECT_DELAY).await;
            }
        }
    }

    pub(crate) async fn handle_command(
        &self,
        topic: &str,
        payload: &str,
        zone_commands: &HashMap<usize, ZoneCommandSender>,
        state: &state::SharedState,
        snap_tx: &tokio::sync::mpsc::Sender<SnapcastCmd>,
    ) -> Result<()> {
        let stripped = topic
            .strip_prefix(&self.base_topic)
            .and_then(|t| t.strip_prefix('/'))
            .context("Topic doesn't match base")?;

        let parts: Vec<&str> = stripped.split('/').collect();

        match parts.as_slice() {
            // Zone commands → routed through ZonePlayer
            ["zones", idx, "volume", "set"] => {
                let index: usize = idx.parse()?;
                let volume: f64 = payload.parse()?;
                // Accept both 0.0-1.0 (HA) and 0-100 (legacy)
                let volume_int = if volume <= 1.0 {
                    (volume * 100.0).round() as i32
                } else {
                    volume as i32
                };
                send_zone_cmd(
                    zone_commands,
                    index,
                    ZoneCommand::SetVolume(volume_int.clamp(0, 100)),
                )
                .await;
            }
            ["zones", idx, "mute", "set"] => {
                let index: usize = idx.parse()?;
                let muted = matches!(payload.trim(), "true" | "1" | "on");
                send_zone_cmd(zone_commands, index, ZoneCommand::SetMute(muted)).await;
            }
            ["zones", idx, "control", "set"] => {
                let index: usize = idx.parse()?;
                let cmd = match payload.to_lowercase().as_str() {
                    "play" => Some(ZoneCommand::Play),
                    "pause" => Some(ZoneCommand::Pause),
                    "stop" => Some(ZoneCommand::Stop),
                    "next" => Some(ZoneCommand::Next),
                    "previous" => Some(ZoneCommand::Previous),
                    _ => {
                        tracing::debug!(zone = index, command = payload, "Unknown control command");
                        None
                    }
                };
                if let Some(cmd) = cmd {
                    send_zone_cmd(zone_commands, index, cmd).await;
                }
            }
            ["zones", idx, "playlist", "set"] => {
                let index: usize = idx.parse()?;
                let playlist: usize = payload.parse()?;
                send_zone_cmd(zone_commands, index, ZoneCommand::SetPlaylist(playlist, 0)).await;
            }
            ["zones", idx, "track", "set"] => {
                let index: usize = idx.parse()?;
                let track: usize = payload.parse()?;
                send_zone_cmd(zone_commands, index, ZoneCommand::SetTrack(track)).await;
            }
            ["zones", idx, "position", "set"] | ["zones", idx, "track", "position", "set"] => {
                let index: usize = idx.parse()?;
                let pos_secs: f64 = payload.parse()?;
                let pos_ms = (pos_secs * 1000.0) as i64;
                send_zone_cmd(zone_commands, index, ZoneCommand::Seek(pos_ms)).await;
            }
            ["zones", idx, "shuffle", "set"] => {
                let index: usize = idx.parse()?;
                let shuffle = matches!(payload.trim(), "true" | "1" | "on");
                send_zone_cmd(zone_commands, index, ZoneCommand::SetShuffle(shuffle)).await;
            }
            ["zones", idx, "repeat", "set"] => {
                let index: usize = idx.parse()?;
                match payload.trim() {
                    "off" => {
                        send_zone_cmd(zone_commands, index, ZoneCommand::SetRepeat(false)).await;
                        send_zone_cmd(zone_commands, index, ZoneCommand::SetTrackRepeat(false))
                            .await;
                    }
                    "one" => {
                        send_zone_cmd(zone_commands, index, ZoneCommand::SetTrackRepeat(true))
                            .await;
                    }
                    "all" => {
                        send_zone_cmd(zone_commands, index, ZoneCommand::SetTrackRepeat(false))
                            .await;
                        send_zone_cmd(zone_commands, index, ZoneCommand::SetRepeat(true)).await;
                    }
                    _ => {}
                }
            }
            ["zones", idx, "presence", "set"] => {
                let index: usize = idx.parse()?;
                let present: bool = payload.parse()?;
                send_zone_cmd(zone_commands, index, ZoneCommand::SetPresence(present)).await;
            }
            ["zones", idx, "presence", "enable", "set"] => {
                let index: usize = idx.parse()?;
                let enabled: bool = payload.parse()?;
                send_zone_cmd(
                    zone_commands,
                    index,
                    ZoneCommand::SetPresenceEnabled(enabled),
                )
                .await;
            }
            ["zones", idx, "presence", "timeout", "set"] => {
                let index: usize = idx.parse()?;
                let delay: u16 = payload.parse()?;
                send_zone_cmd(zone_commands, index, ZoneCommand::SetAutoOffDelay(delay)).await;
            }

            // Client commands → direct state mutation (no ZonePlayer involvement)
            ["clients", idx, "volume", "set"] => {
                let index: usize = idx.parse()?;
                let volume: i32 = payload.parse()?;
                let snap_id = {
                    let store = state.read().await;
                    store
                        .clients
                        .get(&index)
                        .and_then(|c| c.snapcast_id.clone())
                };
                if let Some(snap_id) = snap_id {
                    let _ = snap_tx
                        .send(SnapcastCmd::Client {
                            client_id: snap_id,
                            action: ClientAction::Volume(volume.clamp(0, 100)),
                        })
                        .await;
                }
            }
            ["clients", idx, "mute", "set"] => {
                let index: usize = idx.parse()?;
                let muted: bool = payload.parse()?;
                let snap_id = {
                    let store = state.read().await;
                    store
                        .clients
                        .get(&index)
                        .and_then(|c| c.snapcast_id.clone())
                };
                if let Some(snap_id) = snap_id {
                    let _ = snap_tx
                        .send(SnapcastCmd::Client {
                            client_id: snap_id,
                            action: ClientAction::Mute(muted),
                        })
                        .await;
                }
            }
            ["clients", idx, "latency", "set"] => {
                let index: usize = idx.parse()?;
                let latency: i32 = payload.parse()?;
                let snap_id = {
                    let store = state.read().await;
                    store
                        .clients
                        .get(&index)
                        .and_then(|c| c.snapcast_id.clone())
                };
                if let Some(snap_id) = snap_id {
                    let _ = snap_tx
                        .send(SnapcastCmd::Client {
                            client_id: snap_id,
                            action: ClientAction::Latency(latency),
                        })
                        .await;
                }
            }
            ["clients", idx, "zone", "set"] => {
                let index: usize = idx.parse()?;
                let zone: usize = payload.parse()?;
                let mut store = state.write().await;
                if let Some(client) = store.clients.get_mut(&index) {
                    client.zone_index = zone;
                }
                drop(store);
                let _ = snap_tx.send(SnapcastCmd::ReconcileZones).await;
            }
            _ => {
                tracing::debug!(topic = stripped, "Unhandled MQTT command topic");
            }
        }

        Ok(())
    }
}

async fn send_zone_cmd(
    zone_commands: &HashMap<usize, ZoneCommandSender>,
    index: usize,
    cmd: ZoneCommand,
) {
    if let Some(tx) = zone_commands.get(&index) {
        if let Err(e) = tx.send(cmd).await {
            tracing::warn!(zone = index, error = %e, "Failed to send zone command from MQTT");
        }
    } else {
        tracing::warn!(zone = index, "No ZonePlayer for MQTT command");
    }
}

fn parse_host(broker: &str) -> &str {
    broker.rsplit_once(':').map_or(broker, |(h, _)| h)
}

fn parse_port(broker: &str) -> Result<u16> {
    broker
        .rsplit_once(':')
        .and_then(|(_, p)| p.parse().ok())
        .context("Invalid broker address — expected host:port")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tokio::sync::{RwLock, mpsc};

    fn test_state_with_client() -> state::SharedState {
        let store: state::Store = serde_json::from_value(serde_json::json!({
            "zones": {},
            "clients": {
                "1": {
                    "name": "Test", "icon": "", "mac": "", "zone_index": 1,
                    "volume": 50, "base_volume": 50, "muted": false,
                    "latency_ms": 0, "connected": true, "snapcast_id": "snap-1"
                }
            }
        }))
        .unwrap();
        Arc::new(RwLock::new(store))
    }

    fn zone_channels() -> (
        HashMap<usize, ZoneCommandSender>,
        mpsc::Receiver<ZoneCommand>,
    ) {
        let (tx, rx) = mpsc::channel(16);
        let mut map = HashMap::new();
        map.insert(1, tx);
        (map, rx)
    }

    fn snap_channel() -> (mpsc::Sender<SnapcastCmd>, mpsc::Receiver<SnapcastCmd>) {
        mpsc::channel(16)
    }

    #[test]
    fn parses_broker_port() {
        assert_eq!(parse_port("mqtt:1883").unwrap(), 1883);
        assert_eq!(parse_port("localhost:1883").unwrap(), 1883);
        assert!(parse_port("no-port").is_err());
    }

    #[tokio::test]
    async fn routes_zone_volume() {
        let bridge = MqttBridge::test_bridge("snapdog");
        let (snap_tx, _snap_rx) = snap_channel();
        let (cmds, mut rx) = zone_channels();
        let state = test_state_with_client();
        bridge
            .handle_command("snapdog/zones/1/volume/set", "75", &cmds, &state, &snap_tx)
            .await
            .unwrap();
        assert!(matches!(rx.recv().await, Some(ZoneCommand::SetVolume(75))));
    }

    #[tokio::test]
    async fn routes_zone_mute() {
        let bridge = MqttBridge::test_bridge("snapdog");
        let (snap_tx, _snap_rx) = snap_channel();
        let (cmds, mut rx) = zone_channels();
        let state = test_state_with_client();
        bridge
            .handle_command("snapdog/zones/1/mute/set", "true", &cmds, &state, &snap_tx)
            .await
            .unwrap();
        assert!(matches!(rx.recv().await, Some(ZoneCommand::SetMute(true))));
    }

    #[tokio::test]
    async fn routes_zone_control_commands() {
        let bridge = MqttBridge::test_bridge("snapdog");
        let (snap_tx, _snap_rx) = snap_channel();
        let (cmds, mut rx) = zone_channels();
        let state = test_state_with_client();
        for (payload, expected) in [
            ("play", ZoneCommand::Play),
            ("pause", ZoneCommand::Pause),
            ("stop", ZoneCommand::Stop),
            ("next", ZoneCommand::Next),
            ("previous", ZoneCommand::Previous),
        ] {
            bridge
                .handle_command(
                    "snapdog/zones/1/control/set",
                    payload,
                    &cmds,
                    &state,
                    &snap_tx,
                )
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
        let bridge = MqttBridge::test_bridge("snapdog");
        let (snap_tx, _snap_rx) = snap_channel();
        let (cmds, mut rx) = zone_channels();
        let state = test_state_with_client();
        bridge
            .handle_command("snapdog/zones/1/playlist/set", "3", &cmds, &state, &snap_tx)
            .await
            .unwrap();
        assert!(matches!(
            rx.recv().await,
            Some(ZoneCommand::SetPlaylist(3, 0))
        ));
    }

    #[tokio::test]
    async fn routes_zone_seek() {
        let bridge = MqttBridge::test_bridge("snapdog");
        let (snap_tx, _snap_rx) = snap_channel();
        let (cmds, mut rx) = zone_channels();
        let state = test_state_with_client();
        bridge
            .handle_command(
                "snapdog/zones/1/track/position/set",
                "30",
                &cmds,
                &state,
                &snap_tx,
            )
            .await
            .unwrap();
        assert!(matches!(rx.recv().await, Some(ZoneCommand::Seek(30000))));
    }

    #[tokio::test]
    async fn routes_client_volume() {
        let bridge = MqttBridge::test_bridge("snapdog");
        let (snap_tx, mut snap_rx) = snap_channel();
        let (cmds, _rx) = zone_channels();
        let state = test_state_with_client();
        bridge
            .handle_command(
                "snapdog/clients/1/volume/set",
                "80",
                &cmds,
                &state,
                &snap_tx,
            )
            .await
            .unwrap();
        let cmd = snap_rx.recv().await.unwrap();
        assert!(
            matches!(cmd, SnapcastCmd::Client { client_id, action: ClientAction::Volume(80) } if client_id == "snap-1")
        );
    }

    #[tokio::test]
    async fn routes_client_mute() {
        let bridge = MqttBridge::test_bridge("snapdog");
        let (snap_tx, mut snap_rx) = snap_channel();
        let (cmds, _rx) = zone_channels();
        let state = test_state_with_client();
        bridge
            .handle_command(
                "snapdog/clients/1/mute/set",
                "true",
                &cmds,
                &state,
                &snap_tx,
            )
            .await
            .unwrap();
        let cmd = snap_rx.recv().await.unwrap();
        assert!(
            matches!(cmd, SnapcastCmd::Client { client_id, action: ClientAction::Mute(true) } if client_id == "snap-1")
        );
    }

    #[tokio::test]
    async fn routes_client_zone_change() {
        let bridge = MqttBridge::test_bridge("snapdog");
        let (snap_tx, mut snap_rx) = snap_channel();
        let (cmds, _rx) = zone_channels();
        let state = test_state_with_client();
        bridge
            .handle_command("snapdog/clients/1/zone/set", "2", &cmds, &state, &snap_tx)
            .await
            .unwrap();
        let s = state.read().await;
        assert_eq!(s.clients[&1].zone_index, 2);
        drop(s);
        let cmd = snap_rx.recv().await.unwrap();
        assert!(matches!(cmd, SnapcastCmd::ReconcileZones));
    }

    #[tokio::test]
    async fn clamps_client_volume() {
        let bridge = MqttBridge::test_bridge("snapdog");
        let (snap_tx, mut snap_rx) = snap_channel();
        let (cmds, _rx) = zone_channels();
        let state = test_state_with_client();
        bridge
            .handle_command(
                "snapdog/clients/1/volume/set",
                "200",
                &cmds,
                &state,
                &snap_tx,
            )
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

    #[tokio::test]
    async fn rejects_unknown_topic() {
        let bridge = MqttBridge::test_bridge("snapdog");
        let (snap_tx, _snap_rx) = snap_channel();
        let (cmds, _rx) = zone_channels();
        let state = test_state_with_client();
        // Should not error, just silently ignore
        bridge
            .handle_command("snapdog/unknown/topic", "x", &cmds, &state, &snap_tx)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn rejects_wrong_base_topic() {
        let bridge = MqttBridge::test_bridge("snapdog");
        let (snap_tx, _snap_rx) = snap_channel();
        let (cmds, _rx) = zone_channels();
        let state = test_state_with_client();
        let result = bridge
            .handle_command("other/zones/1/volume/set", "50", &cmds, &state, &snap_tx)
            .await;
        assert!(result.is_err());
    }
}
