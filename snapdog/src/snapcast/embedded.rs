// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! Embedded Snapcast backend — in-process server via snapcast-server crate.

use std::collections::HashMap;

use anyhow::{Context, Result};
use tokio::sync::mpsc;

use snapcast_server::{ServerCommand, ServerConfig, ServerEvent, SnapServer};

use super::backend::{BoxFuture, SnapcastBackend, SnapcastEvent};
use crate::config::AppConfig;
use crate::player::{ClientAction, GroupAction, SnapcastCmd};
use crate::state;

/// Default audio buffer size in milliseconds for the embedded server.
const DEFAULT_BUFFER_MS: u32 = 1000;

/// Embedded Snapcast server backend.
pub struct EmbeddedBackend {
    cmd_tx: mpsc::Sender<ServerCommand>,
    audio_txs: HashMap<usize, tokio::sync::Mutex<snapcast_server::F32AudioSender>>,
    store: state::SharedState,
    volume_modes: HashMap<usize, crate::config::GroupVolumeMode>,
}

/// Event receiver from the embedded server.
pub struct EmbeddedEventReceiver {
    event_rx: mpsc::Receiver<ServerEvent>,
}

impl EmbeddedBackend {
    /// Start the embedded server. Returns the backend + event receiver.
    ///
    /// # Errors
    ///
    /// Returns an error if the server cannot be started.
    pub fn start(
        config: &AppConfig,
        store: state::SharedState,
    ) -> Result<(Self, EmbeddedEventReceiver)> {
        // Resolve f32lz4e → f32lz4 + encryption
        let (codec, encryption_psk) = if config.snapcast.codec == crate::config::AudioCodec::F32lz4e
        {
            let psk = config.snapcast.encryption_psk.as_ref().map_or_else(
                || snapcast_server::DEFAULT_ENCRYPTION_PSK.into(),
                |s| s.to_string(),
            );
            ("f32lz4".into(), Some(psk))
        } else {
            (
                config.snapcast.codec.as_str().to_string(),
                config
                    .snapcast
                    .encryption_psk
                    .as_ref()
                    .map(|s| s.to_string()),
            )
        };

        // Build client filter based on unknown_clients policy
        let client_filter: Option<std::sync::Arc<dyn snapcast_server::auth::ClientFilter>> =
            if config.snapcast.unknown_clients == crate::config::UnknownClientPolicy::Reject {
                struct MacAllowlist(Vec<String>);
                impl snapcast_server::auth::ClientFilter for MacAllowlist {
                    fn accept(&self, hello: &snapcast_server::Hello) -> bool {
                        self.0.iter().any(|m| m == &hello.mac.to_lowercase())
                    }
                }
                let allowed_macs: Vec<String> = config
                    .clients
                    .iter()
                    .map(|c| c.mac.to_lowercase())
                    .collect();
                Some(std::sync::Arc::new(MacAllowlist(allowed_macs)))
            } else {
                None
            };

        let server_config = ServerConfig {
            stream_port: config.snapcast.streaming_port,
            buffer_ms: DEFAULT_BUFFER_MS,
            codec,
            sample_format: config.audio.sample_format(),
            encryption_psk,
            client_filter,
            state_file: Some(
                std::path::PathBuf::from(&config.system.state_dir).join("snapcast.json"),
            ),
            mdns_service_type: config.snapcast.mdns_service_type.clone(),
            mdns_name: config.snapcast.mdns_name.clone(),
            ..ServerConfig::default()
        };

        let (mut server, event_rx) = SnapServer::new(server_config);

        // One stream per zone
        let mut audio_txs = HashMap::new();
        for zone in &config.zones {
            let tx = server
                .add_f32_stream(&zone.stream_name)
                .map_err(|e| anyhow::anyhow!(e))?;
            audio_txs.insert(zone.index, tokio::sync::Mutex::new(tx));
        }

        let cmd_tx = server.command_sender();

        tokio::spawn(async move {
            if let Err(e) = server.run().await {
                tracing::error!(error = %e, "Embedded Snapcast server error");
            }
        });

        tracing::info!(
            port = config.snapcast.streaming_port,
            zones = audio_txs.len(),
            "Embedded Snapcast server started"
        );

        // mDNS is handled automatically by SnapServer::run()

        Ok((
            Self {
                cmd_tx,
                audio_txs,
                store,
                volume_modes: config
                    .zones
                    .iter()
                    .map(|z| (z.index, z.group_volume_mode))
                    .collect(),
            },
            EmbeddedEventReceiver { event_rx },
        ))
    }

    /// Map a `SnapcastCmd` to one or more `ServerCommand`s.
    fn map_command(cmd: SnapcastCmd) -> Vec<ServerCommand> {
        match cmd {
            SnapcastCmd::Group { group_id, action } => match action {
                GroupAction::Stream(stream_id) => {
                    vec![ServerCommand::SetGroupStream {
                        group_id,
                        stream_id,
                    }]
                }
                GroupAction::Clients(clients) => {
                    vec![ServerCommand::SetGroupClients { group_id, clients }]
                }
                GroupAction::Name(name) => {
                    vec![ServerCommand::SetGroupName { group_id, name }]
                }
                GroupAction::Volume(_) => vec![], // handled in execute()
                GroupAction::Mute(muted) => {
                    vec![ServerCommand::SetGroupMute { group_id, muted }]
                }
            },
            SnapcastCmd::Client { client_id, action } => match action {
                ClientAction::Volume(percent) => {
                    vec![ServerCommand::SetClientVolume {
                        client_id,
                        volume: percent.clamp(0, 100) as u16,
                        muted: false, // preserve current — server merges
                    }]
                }
                ClientAction::Mute(muted) => {
                    vec![ServerCommand::SetClientVolume {
                        client_id,
                        volume: 0, // preserve current — server merges
                        muted,
                    }]
                }
                ClientAction::Latency(ms) => {
                    vec![ServerCommand::SetClientLatency {
                        client_id,
                        latency: ms,
                    }]
                }
                ClientAction::SendCustom { type_id, payload } => {
                    vec![ServerCommand::SendToClient {
                        client_id,
                        message: snapcast_server::CustomMessage::new(type_id, payload),
                    }]
                }
                ClientAction::AdjustVolume(_) => {
                    // Converted to absolute Volume in main loop before reaching backend
                    vec![]
                }
            },
            SnapcastCmd::ReconcileZones => unreachable!("handled in execute"),
        }
    }

    /// Map a `ServerEvent` to a `SnapcastEvent`.
    fn map_event(event: ServerEvent) -> Option<SnapcastEvent> {
        match event {
            ServerEvent::ClientConnected { id, hello } => Some(SnapcastEvent::ClientConnected {
                id,
                hello: super::backend::ClientHello {
                    client_name: hello.client_name,
                    mac: hello.mac,
                    host_name: hello.host_name,
                    version: hello.version,
                },
            }),
            ServerEvent::ClientDisconnected { id } => {
                Some(SnapcastEvent::ClientDisconnected { id })
            }
            ServerEvent::ClientVolumeChanged {
                client_id,
                volume,
                muted,
            } => Some(SnapcastEvent::ClientVolumeChanged {
                id: client_id,
                volume: i32::from(volume),
                muted,
            }),
            ServerEvent::ClientLatencyChanged { client_id, latency } => {
                Some(SnapcastEvent::ClientLatencyChanged {
                    id: client_id,
                    latency,
                })
            }
            ServerEvent::ClientNameChanged { client_id, name } => {
                Some(SnapcastEvent::ClientNameChanged {
                    id: client_id,
                    name,
                })
            }
            ServerEvent::GroupStreamChanged { .. }
            | ServerEvent::GroupMuteChanged { .. }
            | ServerEvent::GroupNameChanged { .. }
            | ServerEvent::StreamStatus { .. }
            | ServerEvent::ServerUpdated => Some(SnapcastEvent::ServerUpdated),
            _ => None,
        }
    }
}

impl EmbeddedEventReceiver {
    /// Receive the next mapped event. Returns `None` when the server shuts down.
    pub async fn recv(&mut self) -> Option<SnapcastEvent> {
        loop {
            let event = self.event_rx.recv().await?;
            if let Some(mapped) = EmbeddedBackend::map_event(event) {
                return Some(mapped);
            }
        }
    }
}

impl SnapcastBackend for EmbeddedBackend {
    fn send_audio(
        &self,
        zone_index: usize,
        samples: &[f32],
        _sample_rate: u32,
        _channels: u16,
    ) -> BoxFuture<'_, Result<()>> {
        let samples = samples.to_vec();
        Box::pin(async move {
            let tx = self
                .audio_txs
                .get(&zone_index)
                .ok_or_else(|| anyhow::anyhow!("No audio stream for zone {zone_index}"))?;
            tx.lock()
                .await
                .send(&samples)
                .await
                .map_err(|_| anyhow::anyhow!("Audio channel closed for zone {zone_index}"))
        })
    }

    #[allow(clippy::too_many_lines)] // Command match dispatcher — splitting would obscure control flow
    fn execute(&self, cmd: SnapcastCmd) -> BoxFuture<'_, Result<()>> {
        Box::pin(async move {
            let commands = match &cmd {
                // Group volume: compute per-client effective volume based on mode
                SnapcastCmd::Group {
                    group_id,
                    action: GroupAction::Volume(percent),
                } => {
                    let s = self.store.read().await;
                    let zone_index = s
                        .zones
                        .iter()
                        .find(|(_, z)| z.snapcast_group_id.as_deref() == Some(group_id))
                        .map(|(&zi, _)| zi);
                    let mode = zone_index
                        .and_then(|zi| self.volume_modes.get(&zi).copied())
                        .unwrap_or_default();
                    s.clients
                        .values()
                        .filter(|c| zone_index.is_some_and(|zi| c.zone_index == zi))
                        .filter_map(|c| {
                            c.snapcast_id
                                .as_ref()
                                .map(|sid| ServerCommand::SetClientVolume {
                                    client_id: sid.clone(),
                                    volume: mode.effective(c.base_volume, *percent, c.max_volume)
                                        as u16,
                                    muted: c.muted,
                                })
                        })
                        .collect()
                }
                // Mute needs current volume — SetClientVolume sets both
                SnapcastCmd::Client {
                    client_id,
                    action: ClientAction::Mute(muted),
                } => {
                    let s = self.store.read().await;
                    let volume = s
                        .clients
                        .values()
                        .find(|c| c.snapcast_id.as_deref() == Some(client_id))
                        .map_or(100, |c| c.volume.clamp(0, 100) as u16);
                    drop(s);
                    vec![ServerCommand::SetClientVolume {
                        client_id: client_id.clone(),
                        volume,
                        muted: *muted,
                    }]
                }
                // Volume: update base_volume, compute effective via zone mode
                SnapcastCmd::Client {
                    client_id,
                    action: ClientAction::Volume(percent),
                } => {
                    let mut s = self.store.write().await;
                    // Read zone info before mutable client borrow
                    let client_zone = s
                        .clients
                        .values()
                        .find(|c| c.snapcast_id.as_deref() == Some(client_id))
                        .map(|c| (c.zone_index, c.muted, c.max_volume));
                    let (zone_vol, mode) = client_zone.map_or_else(
                        || (100, crate::config::GroupVolumeMode::default()),
                        |(zi, _, _)| {
                            let zv = s.zones.get(&zi).map_or(100, |z| z.volume);
                            let m = self.volume_modes.get(&zi).copied().unwrap_or_default();
                            (zv, m)
                        },
                    );
                    let muted = client_zone.is_some_and(|(_, m, _)| m);
                    let max_vol = client_zone.map_or(100, |(_, _, mv)| mv);
                    // Now mutate
                    if let Some(c) = s
                        .clients
                        .values_mut()
                        .find(|c| c.snapcast_id.as_deref() == Some(client_id))
                    {
                        c.base_volume = (*percent).clamp(0, max_vol);
                    }
                    let base = (*percent).clamp(0, max_vol);
                    drop(s);
                    vec![ServerCommand::SetClientVolume {
                        client_id: client_id.clone(),
                        volume: mode.effective(base, zone_vol, max_vol) as u16,
                        muted,
                    }]
                }
                SnapcastCmd::ReconcileZones => {
                    // Move clients to their zone's group. Issue SetGroupClients for
                    // ALL zones that have a known group — including zones losing clients
                    // (empty list removes the client from that group, snapcast-server
                    // creates a new group for evicted clients automatically).
                    // The ServerUpdated event then triggers sync_group_ids which
                    // picks up new groups and fixes stream assignments.
                    let s = self.store.read().await;

                    // Build zone_index → [snapcast_ids] for zones with clients
                    let mut zone_clients: HashMap<usize, Vec<String>> = HashMap::new();
                    for c in s.clients.values() {
                        if let Some(ref sid) = c.snapcast_id {
                            zone_clients
                                .entry(c.zone_index)
                                .or_default()
                                .push(sid.clone());
                        }
                    }

                    let mut cmds = vec![];
                    for (&zi, zone) in &s.zones {
                        if let Some(ref gid) = zone.snapcast_group_id {
                            let clients = zone_clients.remove(&zi).unwrap_or_default();
                            cmds.push(ServerCommand::SetGroupClients {
                                group_id: gid.clone(),
                                clients,
                            });
                        }
                    }
                    drop(s);
                    cmds
                }
                _ => Self::map_command(cmd),
            };
            for c in commands {
                self.cmd_tx
                    .send(c)
                    .await
                    .context("Server command channel closed")?;
            }
            Ok(())
        })
    }

    fn stop(&self) -> BoxFuture<'_, Result<()>> {
        Box::pin(async move {
            self.cmd_tx
                .send(ServerCommand::Stop)
                .await
                .context("Failed to send stop command")
        })
    }

    fn get_status(&self) -> BoxFuture<'_, Result<serde_json::Value>> {
        Box::pin(async move {
            let (tx, rx) = tokio::sync::oneshot::channel();
            self.cmd_tx
                .send(ServerCommand::GetStatus { response_tx: tx })
                .await
                .context("Server command channel closed")?;
            let status = rx.await.context("Status response channel closed")?;
            serde_json::to_value(status).context("Failed to serialize status")
        })
    }

    fn delete_client(&self, id: &str) -> BoxFuture<'_, Result<()>> {
        let client_id = id.to_string();
        Box::pin(async move {
            self.cmd_tx
                .send(ServerCommand::DeleteClient { client_id })
                .await
                .context("Server command channel closed")
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_command_group_stream() {
        let cmd = SnapcastCmd::Group {
            group_id: "g1".into(),
            action: GroupAction::Stream("Zone1".into()),
        };
        let cmds = EmbeddedBackend::map_command(cmd);
        assert_eq!(cmds.len(), 1);
        assert!(
            matches!(&cmds[0], ServerCommand::SetGroupStream { group_id, stream_id } if group_id == "g1" && stream_id == "Zone1")
        );
    }

    #[test]
    fn map_command_group_clients() {
        let cmd = SnapcastCmd::Group {
            group_id: "g1".into(),
            action: GroupAction::Clients(vec!["c1".into(), "c2".into()]),
        };
        let cmds = EmbeddedBackend::map_command(cmd);
        assert_eq!(cmds.len(), 1);
        assert!(
            matches!(&cmds[0], ServerCommand::SetGroupClients { clients, .. } if clients.len() == 2)
        );
    }

    #[test]
    fn map_command_group_mute() {
        let cmd = SnapcastCmd::Group {
            group_id: "g1".into(),
            action: GroupAction::Mute(true),
        };
        let cmds = EmbeddedBackend::map_command(cmd);
        assert!(matches!(
            &cmds[0],
            ServerCommand::SetGroupMute { muted: true, .. }
        ));
    }

    #[test]
    fn map_command_group_volume_noop() {
        let cmd = SnapcastCmd::Group {
            group_id: "g1".into(),
            action: GroupAction::Volume(50),
        };
        assert!(EmbeddedBackend::map_command(cmd).is_empty());
    }

    #[test]
    fn map_command_client_volume() {
        let cmd = SnapcastCmd::Client {
            client_id: "c1".into(),
            action: ClientAction::Volume(75),
        };
        let cmds = EmbeddedBackend::map_command(cmd);
        assert!(matches!(
            &cmds[0],
            ServerCommand::SetClientVolume { volume: 75, .. }
        ));
    }

    #[test]
    fn map_command_client_volume_clamps() {
        let cmd = SnapcastCmd::Client {
            client_id: "c1".into(),
            action: ClientAction::Volume(200),
        };
        let cmds = EmbeddedBackend::map_command(cmd);
        assert!(matches!(
            &cmds[0],
            ServerCommand::SetClientVolume { volume: 100, .. }
        ));
    }

    #[test]
    fn map_command_client_mute() {
        let cmd = SnapcastCmd::Client {
            client_id: "c1".into(),
            action: ClientAction::Mute(true),
        };
        let cmds = EmbeddedBackend::map_command(cmd);
        assert!(matches!(
            &cmds[0],
            ServerCommand::SetClientVolume { muted: true, .. }
        ));
    }

    #[test]
    fn map_command_client_latency() {
        let cmd = SnapcastCmd::Client {
            client_id: "c1".into(),
            action: ClientAction::Latency(50),
        };
        let cmds = EmbeddedBackend::map_command(cmd);
        assert!(matches!(
            &cmds[0],
            ServerCommand::SetClientLatency { latency: 50, .. }
        ));
    }

    #[test]
    fn map_event_client_connected() {
        let event = ServerEvent::ClientConnected {
            id: "c1".into(),
            hello: snapcast_server::Hello {
                mac: "aa:bb:cc:dd:ee:ff".into(),
                host_name: "Kitchen".into(),
                version: "0.1.0".into(),
                client_name: "Snapclient".into(),
                os: "Linux".into(),
                arch: "x86_64".into(),
                instance: 1,
                id: "c1".into(),
                snap_stream_protocol_version: 2,
                auth: None,
            },
        };
        let mapped = EmbeddedBackend::map_event(event).unwrap();
        assert!(
            matches!(mapped, SnapcastEvent::ClientConnected { id, ref hello } if id == "c1" && hello.host_name == "Kitchen")
        );
    }

    #[test]
    fn map_event_client_disconnected() {
        let event = ServerEvent::ClientDisconnected { id: "c1".into() };
        let mapped = EmbeddedBackend::map_event(event).unwrap();
        assert!(matches!(mapped, SnapcastEvent::ClientDisconnected { id } if id == "c1"));
    }

    #[test]
    fn map_event_volume_changed() {
        let event = ServerEvent::ClientVolumeChanged {
            client_id: "c1".into(),
            volume: 80,
            muted: false,
        };
        let mapped = EmbeddedBackend::map_event(event).unwrap();
        assert!(matches!(
            mapped,
            SnapcastEvent::ClientVolumeChanged {
                volume: 80,
                muted: false,
                ..
            }
        ));
    }

    #[test]
    fn map_event_group_events_become_server_updated() {
        let event = ServerEvent::GroupMuteChanged {
            group_id: "g1".into(),
            muted: true,
        };
        assert!(matches!(
            EmbeddedBackend::map_event(event),
            Some(SnapcastEvent::ServerUpdated)
        ));
    }

    #[test]
    fn map_event_group_name_changed() {
        let event = ServerEvent::GroupNameChanged {
            group_id: "g1".into(),
            name: "Living Room".into(),
        };
        assert!(matches!(
            EmbeddedBackend::map_event(event),
            Some(SnapcastEvent::ServerUpdated)
        ));
    }

    #[test]
    fn map_event_server_updated() {
        assert!(matches!(
            EmbeddedBackend::map_event(ServerEvent::ServerUpdated),
            Some(SnapcastEvent::ServerUpdated)
        ));
    }
}
