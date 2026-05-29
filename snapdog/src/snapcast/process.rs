// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! Process-based Snapcast backend — external snapserver binary + JSON-RPC.

use std::collections::HashMap;

use anyhow::{Context, Result};
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio::sync::RwLock;

use super::backend::{BoxFuture, SnapcastBackend};
use super::{SnapcastClient, open_audio_source};
use crate::audio;
use crate::config::AppConfig;
use crate::player::{ClientAction, GroupAction, SnapcastCmd};

/// Process-based Snapcast backend wrapping JSON-RPC + TCP sinks.
pub struct ProcessBackend {
    snap: SnapcastClient,
    /// Zone index → TCP sink (one per zone).
    sinks: RwLock<HashMap<usize, TcpStream>>,
    bit_depth: u16,
    /// Zone index → TCP source port (for reconnect).
    ports: HashMap<usize, u16>,
    store: crate::state::SharedState,
    volume_modes: HashMap<usize, crate::config::GroupVolumeMode>,
}

impl ProcessBackend {
    /// Connect to an existing snapserver process.
    ///
    /// # Errors
    /// Returns an error if the connection fails.
    pub async fn start(
        config: &AppConfig,
        snap: SnapcastClient,
        store: crate::state::SharedState,
    ) -> Result<Self> {
        let mut sinks = HashMap::new();
        let mut ports = HashMap::new();
        for zone in &config.zones {
            let tcp = open_audio_source(zone.tcp_source_port).await?;
            sinks.insert(zone.index, tcp);
            ports.insert(zone.index, zone.tcp_source_port);
        }
        Ok(Self {
            snap,
            sinks: RwLock::new(sinks),
            bit_depth: config.audio.bit_depth,
            ports,
            store,
            volume_modes: config
                .zones
                .iter()
                .map(|z| (z.index, z.group_volume_mode))
                .collect(),
        })
    }

    /// Get a reference to the underlying JSON-RPC client.
    #[must_use]
    pub const fn client(&self) -> &SnapcastClient {
        &self.snap
    }
}

impl SnapcastBackend for ProcessBackend {
    fn send_audio(
        &self,
        zone_index: usize,
        samples: &[f32],
        _sample_rate: u32,
        _channels: u16,
    ) -> BoxFuture<'_, Result<()>> {
        let pcm = audio::resample::f32_to_pcm(samples, self.bit_depth);
        Box::pin(async move {
            if let Some(tcp) = self.sinks.write().await.get_mut(&zone_index) {
                if let Err(e) = tcp.write_all(&pcm).await {
                    tracing::error!(zone = zone_index, error = %e, "TCP write failed");
                    // Reconnect
                    if let Some(&port) = self.ports.get(&zone_index) {
                        match open_audio_source(port).await {
                            Ok(new_tcp) => {
                                *tcp = new_tcp;
                                tracing::info!(zone = zone_index, "TCP audio source reconnected");
                            }
                            Err(e) => {
                                tracing::warn!(zone = zone_index, error = %e, "TCP audio source reconnect failed");
                            }
                        }
                    }
                }
            }
            Ok(())
        })
    }

    fn execute(&self, cmd: SnapcastCmd) -> BoxFuture<'_, Result<()>> {
        Box::pin(async move {
            match cmd {
                SnapcastCmd::Group { group_id, action } => match action {
                    GroupAction::Stream(stream_id) => {
                        self.snap.group_set_stream(&group_id, &stream_id).await
                    }
                    GroupAction::Clients(clients) => {
                        self.snap.group_set_clients(&group_id, clients).await
                    }
                    GroupAction::Name(name) => self.snap.group_set_name(&group_id, &name).await,
                    GroupAction::Volume(percent) => {
                        let s = self.store.read().await;
                        let zone_index = s
                            .zones
                            .iter()
                            .find(|(_, z)| z.snapcast_group_id.as_deref() == Some(&group_id))
                            .map(|(&zi, _)| zi);
                        let mode = zone_index
                            .and_then(|zi| self.volume_modes.get(&zi).copied())
                            .unwrap_or_default();
                        let clients: Vec<_> = s
                            .clients
                            .values()
                            .filter(|c| zone_index.is_some_and(|zi| c.zone_index == zi))
                            .filter_map(|c| {
                                c.snapcast_id
                                    .as_ref()
                                    .map(|sid| (sid.clone(), c.base_volume, c.muted, c.max_volume))
                            })
                            .collect();
                        drop(s);
                        for (cid, base, _muted, max_vol) in clients {
                            let vol = mode.effective(base, percent, max_vol) as u8;
                            self.snap.client_set_volume(&cid, vol).await?;
                        }
                        Ok(())
                    }
                    GroupAction::Mute(muted) => self.snap.group_set_mute(&group_id, muted).await,
                },
                SnapcastCmd::Client { client_id, action } => match action {
                    ClientAction::Volume(v) => {
                        let mut s = self.store.write().await;
                        let info = s
                            .clients
                            .values()
                            .find(|c| c.snapcast_id.as_deref() == Some(&client_id))
                            .map(|c| (c.zone_index, c.max_volume));
                        let (zone_vol, mode, max_vol) = info
                            .map(|(zi, mv)| {
                                let zv = s.zones.get(&zi).map(|z| z.volume).unwrap_or(100);
                                let m = self.volume_modes.get(&zi).copied().unwrap_or_default();
                                (zv, m, mv)
                            })
                            .unwrap_or((100, Default::default(), 100));
                        let base = v.clamp(0, max_vol);
                        if let Some(c) = s
                            .clients
                            .values_mut()
                            .find(|c| c.snapcast_id.as_deref() == Some(&client_id))
                        {
                            c.base_volume = base;
                        }
                        drop(s);
                        self.snap
                            .client_set_volume(
                                &client_id,
                                mode.effective(base, zone_vol, max_vol) as u8,
                            )
                            .await
                    }
                    ClientAction::Mute(muted) => self.snap.client_set_mute(&client_id, muted).await,
                    ClientAction::Latency(ms) => self.snap.client_set_latency(&client_id, ms).await,
                    ClientAction::SendCustom { .. } => {
                        tracing::warn!("custom-protocol not supported in process mode");
                        Ok(())
                    }
                    ClientAction::AdjustVolume(_) => {
                        // Converted to absolute Volume in main loop before reaching backend
                        Ok(())
                    }
                },
                SnapcastCmd::ReconcileZones => Ok(()), // handled at higher level
            }
        })
    }

    fn stop(&self) -> BoxFuture<'_, Result<()>> {
        Box::pin(async { Ok(()) }) // snapserver process stopped separately
    }

    fn get_status(&self) -> BoxFuture<'_, Result<serde_json::Value>> {
        Box::pin(async move {
            let status = self.snap.server_get_status().await?;
            serde_json::to_value(&status).context("Failed to serialize status")
        })
    }

    fn delete_client(&self, id: &str) -> BoxFuture<'_, Result<()>> {
        let id = id.to_string();
        Box::pin(async move { self.snap.server_delete_client(&id).await })
    }
}
