// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! MQTT integration via rumqttc.
//!
//! Bidirectional: subscribes to command topics (`*/set`), publishes status updates.
//! All incoming commands are routed through `ZonePlayer` command channels.

use std::collections::HashMap;

use anyhow::{Context, Result};
use rumqttc::{AsyncClient, Event, LastWill, MqttOptions, Packet, QoS};

use crate::config::MqttConfig;
use crate::player::{ClientAction, SnapcastCmd, ZoneCommand, ZoneCommandSender};
use crate::state;

mod topics;

use topics::{Route, TopicError, Topics};

/// MQTT keep-alive interval.
const MQTT_KEEP_ALIVE: std::time::Duration = std::time::Duration::from_mins(1);
/// Delay before MQTT reconnection attempt.
const MQTT_RECONNECT_DELAY: std::time::Duration = std::time::Duration::from_secs(5);
/// Event loop channel capacity for MQTT messages.
const MQTT_EVENT_CAPACITY: usize = 64;

/// MQTT bridge: receives commands, publishes status.
pub struct MqttBridge {
    client: AsyncClient,
    eventloop: rumqttc::EventLoop,
    topics: Topics,
    base_url: String,
}

impl MqttBridge {
    /// Connect to MQTT broker.
    ///
    /// # Errors
    ///
    /// Returns an error if the broker connection fails.
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

        let topics = Topics::new(&config.base_topic);
        opts.set_last_will(LastWill::new(
            topics.availability(),
            "offline",
            QoS::AtLeastOnce,
            true,
        ));

        let (client, eventloop) = AsyncClient::new(opts, MQTT_EVENT_CAPACITY);
        tracing::info!("MQTT connected");

        Ok(Self {
            client,
            eventloop,
            topics,
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
            topics: Topics::new(base_topic),
            base_url: "http://localhost:5555".to_string(),
        }
    }

    /// Subscribe to all command topics.
    ///
    /// # Errors
    ///
    /// Returns an error if any subscription fails.
    pub async fn subscribe_commands(&self) -> Result<()> {
        let mut count = 0_usize;
        for filter in self.subscription_topics() {
            self.client
                .subscribe(&filter, QoS::AtLeastOnce)
                .await
                .with_context(|| format!("Failed to subscribe to {filter}"))?;
            count += 1;
        }

        tracing::info!(topics = count, "MQTT subscribed");
        Ok(())
    }

    /// The subscription filters, generated from the same route table that
    /// [`Self::handle_command`] parses incoming topics with.
    pub(crate) fn subscription_topics(&self) -> impl Iterator<Item = String> + '_ {
        self.topics.command_filters()
    }

    /// Announce availability and clear what earlier releases left on the broker.
    ///
    /// Publishes the retained `online` on the availability topic (the last will is
    /// `offline`) and clears two kinds of stale retained message:
    ///
    /// - `{base}/status` (double slash), `online` from a past topic bug;
    /// - `homeassistant/media_player/snapdog_zone_{n}/config`, the discovery
    ///   messages of a platform that Home Assistant's MQTT integration does not
    ///   have, so they never produced an entity.
    ///
    /// A zero-length retained publish deletes a retained message (MQTT-3.3.1-10), so
    /// the cleanup is idempotent. `SnapDog` publishes no discovery message at the
    /// moment; see `docs/rfcs/0007-home-assistant-integration.md`.
    ///
    /// # Errors
    ///
    /// Returns an error if a publish cannot be queued.
    pub async fn announce_online(&self, zones: usize) -> Result<()> {
        self.client
            .publish(
                self.topics.availability(),
                QoS::AtLeastOnce,
                true,
                b"online",
            )
            .await?;

        let stale = std::iter::once(self.topics.legacy_double_slash_status()).chain(
            (1..=zones)
                .map(|zone| format!("homeassistant/media_player/snapdog_zone_{zone}/config")),
        );
        for topic in stale {
            self.client
                .publish(topic, QoS::AtLeastOnce, true, Vec::<u8>::new())
                .await?;
        }

        tracing::info!(
            zones,
            "MQTT online announced, stale retained messages cleared"
        );
        Ok(())
    }

    /// Publish a status value (retained).
    ///
    /// # Errors
    ///
    /// Returns an error if the MQTT publish fails.
    pub async fn publish(&self, topic: &str, payload: &str) -> Result<()> {
        self.publish_to(self.topics.join(topic), payload).await
    }

    /// Publish `payload` retained to the full `topic`.
    async fn publish_to(&self, topic: String, payload: &str) -> Result<()> {
        self.client
            .publish(&topic, QoS::AtLeastOnce, true, payload.as_bytes())
            .await
            .with_context(|| format!("Failed to publish to {topic}"))
    }

    /// Publish zone state as a single retained JSON object.
    ///
    /// # Errors
    ///
    /// Returns an error if the MQTT publish fails.
    pub async fn publish_zone_state(&self, index: usize, zone: &state::ZoneState) -> Result<()> {
        let payload = zone_state_payload(&self.base_url, zone);
        self.publish_to(self.topics.zone_state(index), &payload.to_string())
            .await
    }

    /// Publish client state as a single retained JSON object.
    ///
    /// # Errors
    ///
    /// Returns an error if the MQTT publish fails.
    pub async fn publish_client_state(
        &self,
        index: usize,
        client: &state::ClientState,
    ) -> Result<()> {
        self.publish_to(
            self.topics.client_state(index),
            &client_state_payload(client).to_string(),
        )
        .await
    }

    /// Run the event loop, dispatching incoming commands via `ZonePlayer` channels.
    ///
    /// # Errors
    ///
    /// Returns an error if the event loop encounters a fatal MQTT error.
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

    #[allow(clippy::too_many_lines)] // Match dispatcher for MQTT topics — splitting would obscure control flow
    pub(crate) async fn handle_command(
        &self,
        topic: &str,
        payload: &str,
        zone_commands: &HashMap<usize, ZoneCommandSender>,
        state: &state::SharedState,
        snap_tx: &tokio::sync::mpsc::Sender<SnapcastCmd>,
    ) -> Result<()> {
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
                send_zone_cmd(
                    zone_commands,
                    index,
                    ZoneCommand::SetVolume(volume_int.clamp(0, 100)),
                )
                .await;
            }
            Route::ZoneMute => {
                let muted = matches!(payload.trim(), "true" | "1" | "on");
                send_zone_cmd(zone_commands, index, ZoneCommand::SetMute(muted)).await;
            }
            Route::ZoneControl => {
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
            Route::ZonePlaylist => {
                let playlist: usize = payload.parse()?;
                send_zone_cmd(zone_commands, index, ZoneCommand::SetPlaylist(playlist, 0)).await;
            }
            Route::ZoneTrack => {
                let track: usize = payload.parse()?;
                send_zone_cmd(zone_commands, index, ZoneCommand::SetTrack(track)).await;
            }
            Route::ZonePosition => {
                let pos_secs: f64 = payload.parse()?;
                let pos_ms = (pos_secs * 1000.0) as i64;
                send_zone_cmd(zone_commands, index, ZoneCommand::Seek(pos_ms)).await;
            }
            Route::ZoneShuffle => {
                let shuffle = matches!(payload.trim(), "true" | "1" | "on");
                send_zone_cmd(zone_commands, index, ZoneCommand::SetShuffle(shuffle)).await;
            }
            Route::ZoneRepeat => {
                let mode = match payload.trim() {
                    "one" | "track" => snapdog_common::RepeatMode::Track,
                    "all" | "playlist" => snapdog_common::RepeatMode::Playlist,
                    _ => snapdog_common::RepeatMode::Off,
                };
                send_zone_cmd(zone_commands, index, ZoneCommand::SetRepeat(mode)).await;
            }
            Route::ZonePresence => {
                let present: bool = payload.parse()?;
                send_zone_cmd(zone_commands, index, ZoneCommand::SetPresence(present)).await;
            }
            Route::ZonePresenceEnable => {
                let enabled: bool = payload.parse()?;
                send_zone_cmd(
                    zone_commands,
                    index,
                    ZoneCommand::SetPresenceEnabled(enabled),
                )
                .await;
            }
            Route::ZonePresenceTimeout => {
                let delay: u16 = payload.parse()?;
                send_zone_cmd(zone_commands, index, ZoneCommand::SetAutoOffDelay(delay)).await;
            }

            // Client commands → Snapcast commands or direct state mutation
            Route::ClientVolume => {
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
            Route::ClientMute => {
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
            Route::ClientLatency => {
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
            Route::ClientZone => {
                let zone: usize = payload.parse()?;
                let mut store = state.write().await;
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
                let _ = snap_tx.send(SnapcastCmd::ReconcileZones).await;
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

/// Build the retained zone-state JSON (Home Assistant `media_player` schema).
///
/// Pure: extracted from `publish_zone_state` so the wire schema is unit-testable
/// without a broker. Note `source` uses the `Display` form (e.g. `subsonic_playlist`,
/// with underscore — differs from the serde form), and `repeat` maps to HA's
/// `off`/`one`/`all`.
fn zone_state_payload(base_url: &str, zone: &state::ZoneState) -> serde_json::Value {
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
        "repeat": match zone.repeat { snapdog_common::RepeatMode::Track => "one", snapdog_common::RepeatMode::Playlist => "all", snapdog_common::RepeatMode::Off => "off" },
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
            format!("{base_url}{cover_url}")
        } else {
            cover_url.clone()
        };
        payload["media_image_url"] = serde_json::json!(absolute);
    }

    payload
}

/// Build the retained client-state JSON. Pure (see [`zone_state_payload`]).
fn client_state_payload(client: &state::ClientState) -> serde_json::Value {
    serde_json::json!({
        "volume": client.base_volume,
        "muted": client.muted,
        "connected": client.connected,
        "zone": client.zone_index,
        "latency": client.latency_ms,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tokio::sync::{RwLock, mpsc};

    fn test_state_with_client() -> state::SharedState {
        let store: state::Store = serde_json::from_value(serde_json::json!({
            "zones": {
                "1": { "name": "Zone 1", "icon": "", "volume": 50, "muted": false, "playback": "stopped", "shuffle": false, "repeat": "off", "track": null, "playlist_index": null, "playlist_name": null, "playlist_total": null, "playlist_track_index": null, "playlist_track_count": null, "source": "idle", "cover_url": null, "snapcast_group_id": null, "presence": false, "presence_enabled": true, "presence_source": false, "auto_off_active": false, "auto_off_delay": 0 },
                "2": { "name": "Zone 2", "icon": "", "volume": 50, "muted": false, "playback": "stopped", "shuffle": false, "repeat": "off", "track": null, "playlist_index": null, "playlist_name": null, "playlist_total": null, "playlist_track_index": null, "playlist_track_count": null, "source": "idle", "cover_url": null, "snapcast_group_id": null, "presence": false, "presence_enabled": true, "presence_source": false, "auto_off_active": false, "auto_off_delay": 0 }
            },
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

    // ── IT-T31: retained-state JSON schema ────────────────────────

    #[tokio::test]
    async fn zone_state_payload_idle() {
        let state = test_state_with_client();
        let z = state.read().await.zones[&1].clone();
        let p = zone_state_payload("http://host:8000", &z);
        assert_eq!(p["state"], "idle"); // Stopped + Idle source => "idle"
        assert_eq!(p["volume_level"], 0.5); // 50 / 100
        assert_eq!(p["is_volume_muted"], false);
        assert_eq!(p["shuffle"], false);
        assert_eq!(p["repeat"], "off");
        assert_eq!(p["source"], "idle");
        assert_eq!(p["presence"], false);
        assert!(p.get("media_title").is_none(), "no track fields when idle");
        assert!(p.get("media_image_url").is_none(), "no cover when none");
    }

    #[tokio::test]
    async fn zone_state_payload_stopped_non_idle() {
        let state = test_state_with_client();
        let mut z = state.read().await.zones[&1].clone();
        z.source = state::SourceType::Radio; // Stopped + non-Idle source => "stopped"
        let p = zone_state_payload("http://h", &z);
        assert_eq!(p["state"], "stopped");
        assert_eq!(p["source"], "radio");
    }

    #[tokio::test]
    async fn zone_state_payload_playing_with_track_and_cover() {
        let state = test_state_with_client();
        let mut z = state.read().await.zones[&1].clone();
        z.volume = 80;
        z.muted = true;
        z.shuffle = true;
        z.playback = state::PlaybackState::Playing;
        z.repeat = snapdog_common::RepeatMode::Playlist;
        z.source = state::SourceType::SubsonicPlaylist;
        z.cover_url = Some("/cover/1.jpg".into());
        z.track = Some(
            serde_json::from_value(serde_json::json!({
                "title": "T", "artist": "A", "album": "Al",
                "album_artist": null, "genre": null, "year": null,
                "track_number": null, "disc_number": null,
                "duration_ms": 200_000, "position_ms": 5_000, "seekable": true,
                "source": "subsonicplaylist", "bitrate_kbps": null,
                "content_type": null, "sample_rate": null
            }))
            .unwrap(),
        );

        let p = zone_state_payload("http://host:8000", &z);
        assert_eq!(p["state"], "playing");
        assert_eq!(p["volume_level"], 0.8);
        assert_eq!(p["is_volume_muted"], true);
        assert_eq!(p["shuffle"], true);
        assert_eq!(p["repeat"], "all"); // RepeatMode::Playlist => HA "all"
        // `source` is the Display form (underscore), NOT the serde form "subsonicplaylist".
        assert_eq!(p["source"], "subsonic_playlist");
        assert_eq!(p["media_title"], "T");
        assert_eq!(p["media_duration"], 200); // ms -> s
        assert_eq!(p["media_position"], 5);
        assert_eq!(p["media_content_type"], "music");
        // Relative cover URL is absolutized against base_url.
        assert_eq!(p["media_image_url"], "http://host:8000/cover/1.jpg");
    }

    #[tokio::test]
    async fn client_state_payload_schema() {
        let state = test_state_with_client();
        let c = state.read().await.clients[&1].clone();
        let p = client_state_payload(&c);
        assert_eq!(p["volume"], 50); // base_volume
        assert_eq!(p["muted"], false);
        assert_eq!(p["connected"], true);
        assert_eq!(p["zone"], 1);
        assert_eq!(p["latency"], 0);
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
    async fn rejects_client_zone_change_to_unknown_zone() {
        let bridge = MqttBridge::test_bridge("snapdog");
        let (snap_tx, mut snap_rx) = snap_channel();
        let (cmds, _rx) = zone_channels();
        let state = test_state_with_client();
        bridge
            .handle_command("snapdog/clients/1/zone/set", "99", &cmds, &state, &snap_tx)
            .await
            .unwrap();
        let s = state.read().await;
        assert_eq!(s.clients[&1].zone_index, 1);
        drop(s);
        assert!(snap_rx.try_recv().is_err());
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

    // ── Regression: command topics under the real, normalised base topic ──
    //
    // `config` appends the trailing slash ("snapdog" becomes "snapdog/"). The bridge
    // once re-derived the base by hand when parsing, so every inbound command was
    // dropped in production while the tests (which used "snapdog" without slash)
    // passed. These tests go through the config deserializer and take the topics
    // from the function that builds the subscriptions.

    /// A bridge built the way `main` builds it: TOML, config loader, `connect`.
    /// `connect` is lazy, nothing is dialled until the event loop is polled.
    async fn bridge_from_toml(base_topic: Option<&str>) -> (MqttBridge, MqttConfig) {
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
        let cfg = crate::config::load_raw(toml::from_str(&toml).unwrap()).unwrap();
        let mqtt = cfg.mqtt.unwrap();
        let bridge = MqttBridge::connect(&mqtt, "http://localhost:5555")
            .await
            .unwrap();
        (bridge, mqtt)
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
        let bridge = MqttBridge::test_bridge("snapdog/");
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
    async fn every_subscribed_topic_is_handled_under_the_real_config_base() {
        for (configured, effective) in [
            (None, "snapdog/"),
            (Some("snapdog"), "snapdog/"),
            (Some("snapdog/"), "snapdog/"),
            (Some("home/audio"), "home/audio/"),
            (Some("home/audio/"), "home/audio/"),
        ] {
            let (bridge, mqtt) = bridge_from_toml(configured).await;
            assert_eq!(mqtt.base_topic, effective, "config normalisation");

            let filters: Vec<String> = bridge.subscription_topics().collect();
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

                let (snap_tx, mut snap_rx) = snap_channel();
                let (cmds, mut zone_rx) = zone_channels();
                let state = test_state_with_client();
                bridge
                    .handle_command(&topic, valid_payload(tail), &cmds, &state, &snap_tx)
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
    async fn rejects_topic_outside_the_normalised_base() {
        let bridge = MqttBridge::test_bridge("snapdog/");
        let (snap_tx, _snap_rx) = snap_channel();
        let (cmds, _rx) = zone_channels();
        let state = test_state_with_client();
        for topic in ["other/zones/1/volume/set", "snapdogs/zones/1/volume/set"] {
            assert!(
                bridge
                    .handle_command(topic, "50", &cmds, &state, &snap_tx)
                    .await
                    .is_err(),
                "{topic}"
            );
        }
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
