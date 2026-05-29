// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! Snapcast integration — backend trait and implementations.

pub mod backend;
#[cfg(feature = "snapcast-embedded")]
pub mod events;

#[cfg(feature = "snapcast-embedded")]
pub mod embedded;

#[cfg(feature = "snapcast-process")]
pub mod process;

#[cfg(feature = "snapcast-process")]
pub mod connection;
#[cfg(feature = "snapcast-process")]
pub mod protocol;
#[cfg(feature = "snapcast-process")]
pub mod types;

// Everything below is only needed for the process-based backend.
#[cfg(feature = "snapcast-process")]
mod process_impl {

    use std::collections::HashMap;
    use std::net::SocketAddr;

    use anyhow::{Context, Result};
    use serde_json::json;
    use tokio::net::TcpStream;
    use tokio::sync::broadcast;

    use super::connection::Connection;
    use super::protocol::Notification;
    use super::types;
    use super::types::ServerStatus;
    use crate::api;
    use crate::config::AppConfig;
    use crate::state;

    // ── SnapcastClient ────────────────────────────────────────────

    /// Snapcast controller: JSON-RPC connection + TCP audio sources.
    pub struct SnapcastClient {
        conn: Connection,
    }

    impl SnapcastClient {
        /// Connect to snapserver JSON-RPC.
        /// # Errors
        /// Returns an error if the request fails.
        pub async fn connect(addr: SocketAddr) -> Result<Self> {
            let conn = Connection::connect(addr).await?;
            Ok(Self { conn })
        }

        /// Connect using app config.
        /// # Errors
        /// Returns an error if the request fails.
        pub async fn from_config(config: &AppConfig) -> Result<Self> {
            let tcp_port = config.snapcast.jsonrpc_tcp_port;
            let host = &config.snapcast.address;
            let addr: SocketAddr = tokio::net::lookup_host(format!("{host}:{tcp_port}"))
                .await
                .context("Failed to resolve snapcast address")?
                .find(std::net::SocketAddr::is_ipv4)
                .or(tokio::net::lookup_host(format!("{host}:{tcp_port}"))
                    .await
                    .ok()
                    .and_then(|mut a| a.next()))
                .context("No address found for snapcast host")?;
            Self::connect(addr).await
        }

        /// Subscribe to Snapcast server notifications.
        #[must_use]
        pub fn subscribe(&self) -> broadcast::Receiver<Notification> {
            self.conn.subscribe()
        }

        // ── Server methods ────────────────────────────────────────

        /// Fetch full server status.
        /// # Errors
        /// Returns an error if the request fails.
        pub async fn server_get_status(&self) -> Result<ServerStatus> {
            let result = self.conn.request("Server.GetStatus", json!({})).await?;
            serde_json::from_value(result).context("Failed to parse ServerStatus")
        }

        /// Get JSON-RPC protocol version.
        /// # Errors
        /// Returns an error if the request fails.
        pub async fn server_get_rpc_version(&self) -> Result<serde_json::Value> {
            self.conn.request("Server.GetRPCVersion", json!({})).await
        }

        /// Delete a client from the server.
        /// # Errors
        /// Returns an error if the request fails.
        pub async fn server_delete_client(&self, id: &str) -> Result<()> {
            self.conn
                .request("Server.DeleteClient", json!({ "id": id }))
                .await?;
            Ok(())
        }

        // ── Client methods ────────────────────────────────────────

        /// Get client status.
        /// # Errors
        /// Returns an error if the request fails.
        pub async fn client_get_status(&self, id: &str) -> Result<types::Client> {
            #[derive(serde::Deserialize)]
            struct R {
                client: types::Client,
            }
            let result = self
                .conn
                .request("Client.GetStatus", json!({ "id": id }))
                .await?;
            let r: R = serde_json::from_value(result)?;

            Ok(r.client)
        }

        /// Set client volume (preserves current mute state).
        /// # Errors
        /// Returns an error if the request fails.
        pub async fn client_set_volume(&self, id: &str, percent: u8) -> Result<()> {
            self.conn
                .request(
                    "Client.SetVolume",
                    json!({ "id": id, "volume": { "percent": percent } }),
                )
                .await?;
            Ok(())
        }

        /// Set client mute only (preserves current volume).
        /// # Errors
        /// Returns an error if the request fails.
        pub async fn client_set_mute(&self, id: &str, muted: bool) -> Result<()> {
            self.conn
                .request(
                    "Client.SetVolume",
                    json!({ "id": id, "volume": { "muted": muted } }),
                )
                .await?;
            Ok(())
        }

        /// Set client latency.
        /// # Errors
        /// Returns an error if the request fails.
        pub async fn client_set_latency(&self, id: &str, latency: i32) -> Result<()> {
            self.conn
                .request("Client.SetLatency", json!({ "id": id, "latency": latency }))
                .await?;
            Ok(())
        }

        /// Set client name.
        /// # Errors
        /// Returns an error if the request fails.
        pub async fn client_set_name(&self, id: &str, name: &str) -> Result<()> {
            self.conn
                .request("Client.SetName", json!({ "id": id, "name": name }))
                .await?;
            Ok(())
        }

        // ── Group methods ─────────────────────────────────────────

        /// Get group status.
        /// # Errors
        /// Returns an error if the request fails.
        pub async fn group_get_status(&self, id: &str) -> Result<types::Group> {
            #[derive(serde::Deserialize)]
            struct R {
                group: types::Group,
            }
            let result = self
                .conn
                .request("Group.GetStatus", json!({ "id": id }))
                .await?;
            let r: R = serde_json::from_value(result)?;

            Ok(r.group)
        }

        /// Set group mute.
        /// # Errors
        /// Returns an error if the request fails.
        pub async fn group_set_mute(&self, id: &str, muted: bool) -> Result<()> {
            self.conn
                .request("Group.SetMute", json!({ "id": id, "mute": muted }))
                .await?;
            Ok(())
        }

        /// Set group stream.
        /// # Errors
        /// Returns an error if the request fails.
        pub async fn group_set_stream(&self, id: &str, stream_id: &str) -> Result<()> {
            self.conn
                .request(
                    "Group.SetStream",
                    json!({ "id": id, "stream_id": stream_id }),
                )
                .await?;
            Ok(())
        }

        /// Set group clients.
        /// # Errors
        /// Returns an error if the request fails.
        pub async fn group_set_clients(&self, id: &str, clients: Vec<String>) -> Result<()> {
            self.conn
                .request("Group.SetClients", json!({ "id": id, "clients": clients }))
                .await?;
            Ok(())
        }

        /// Set group name.
        /// # Errors
        /// Returns an error if the request fails.
        pub async fn group_set_name(&self, id: &str, name: &str) -> Result<()> {
            self.conn
                .request("Group.SetName", json!({ "id": id, "name": name }))
                .await?;
            Ok(())
        }

        // ── Stream methods ────────────────────────────────────────

        /// Add a stream.
        /// # Errors
        /// Returns an error if the request fails.
        pub async fn stream_add(&self, stream_uri: &str) -> Result<serde_json::Value> {
            self.conn
                .request("Stream.AddStream", json!({ "streamUri": stream_uri }))
                .await
        }

        /// Remove a stream.
        /// # Errors
        /// Returns an error if the request fails.
        pub async fn stream_remove(&self, id: &str) -> Result<()> {
            self.conn
                .request("Stream.RemoveStream", json!({ "id": id }))
                .await?;
            Ok(())
        }

        /// Control a stream.
        /// # Errors
        /// Returns an error if the request fails.
        pub async fn stream_control(
            &self,
            id: &str,
            command: &str,
            params: serde_json::Value,
        ) -> Result<()> {
            self.conn
                .request(
                    "Stream.Control",
                    json!({ "id": id, "command": command, "params": params }),
                )
                .await?;
            Ok(())
        }

        /// Set stream property.
        /// # Errors
        /// Returns an error if the request fails.
        pub async fn stream_set_property(
            &self,
            id: &str,
            properties: serde_json::Value,
        ) -> Result<()> {
            self.conn
                .request(
                    "Stream.SetProperty",
                    json!({ "id": id, "properties": properties }),
                )
                .await?;
            Ok(())
        }
    }

    // ── TCP Audio Source ───────────────────────────────────────────

    /// Open a TCP audio source connection to snapserver.
    /// # Errors
    /// Returns an error if the request fails.
    pub async fn open_audio_source(port: u16) -> Result<TcpStream> {
        let stream = TcpStream::connect(format!("127.0.0.1:{port}"))
            .await
            .context("Failed to connect audio source")?;
        tracing::debug!(port, "Snapcast TCP source connected");
        Ok(stream)
    }

    // ── State sync helpers (called from main.rs) ──────────────────

    /// Sync initial client state from Snapcast server status.
    pub async fn sync_initial_state(
        status: &ServerStatus,
        config: &AppConfig,
        snap: &SnapcastClient,
        store: &state::SharedState,
    ) {
        let mut s = store.write().await;
        // Set snapcast_id and connected for known clients
        // Prefer connected entries when the same MAC appears multiple times
        for group in &status.server.groups {
            for snap_client in &group.clients {
                let mac = snap_client.host.mac.to_lowercase();
                if let Some(client) = s.clients.values_mut().find(|c| c.mac.to_lowercase() == mac) {
                    if !client.connected || snap_client.connected {
                        client.snapcast_id = Some(snap_client.id.clone());
                        client.connected = snap_client.connected;
                    }
                }
            }
        }
        // Sync zone assignments from group membership
        sync_zones_from_groups(&status.server.groups, config, &mut s);

        // Clean up stale Snapcast clients (disconnected duplicates of connected ones)
        let connected_macs: Vec<String> = status
            .server
            .groups
            .iter()
            .flat_map(|g| &g.clients)
            .filter(|c| c.connected)
            .map(|c| c.host.mac.to_lowercase())
            .collect();
        let stale_ids: Vec<String> = status
            .server
            .groups
            .iter()
            .flat_map(|g| &g.clients)
            .filter(|c| !c.connected && connected_macs.contains(&c.host.mac.to_lowercase()))
            .map(|c| c.id.clone())
            .collect();
        for c in s.clients.values() {
            tracing::info!(client = %c.name, zone = c.zone_index, connected = c.connected, "Client synced");
        }
        drop(s);
        for id in &stale_ids {
            tracing::info!(id = %id, "Deleting stale client");
            let _ = snap.server_delete_client(id).await;
        }
    }

    /// Match Snapcast groups to zones by stream ID (read-only, no commands).
    /// Used at startup and on `ServerOnUpdate`\.
    fn sync_zones_from_groups(groups: &[types::Group], config: &AppConfig, s: &mut state::Store) {
        for zone_cfg in &config.zones {
            let group = groups
                .iter()
                .filter(|g| g.stream_id == zone_cfg.stream_name)
                .max_by_key(|g| g.clients.len());

            if let Some(group) = group {
                if let Some(zone) = s.zones.get_mut(&zone_cfg.index) {
                    if zone.snapcast_group_id.as_deref() != Some(&group.id) {
                        tracing::debug!(zone = zone_cfg.index, new = %group.id, "Zone group ID updated");
                        zone.snapcast_group_id = Some(group.id.clone());
                    }
                    if zone.muted != group.muted {
                        tracing::debug!(
                            zone = zone_cfg.index,
                            muted = group.muted,
                            "Zone mute synced from Snapcast group"
                        );
                        zone.muted = group.muted;
                    }
                }
                for snap_client in &group.clients {
                    if let Some(client) = s
                        .clients
                        .values_mut()
                        .find(|c| c.snapcast_id.as_deref() == Some(&snap_client.id))
                    {
                        if client.zone_index != zone_cfg.index {
                            tracing::debug!(client = %client.name, from = client.zone_index, to = zone_cfg.index, "Client zone synced from Snapcast group");
                        }
                        client.zone_index = zone_cfg.index;
                    }
                }
            } else {
                tracing::warn!(zone = zone_cfg.index, stream = %zone_cfg.stream_name, "No Snapcast group found for zone");
            }
        }
    }

    /// Enforce the invariant: one Snapcast group per zone, all zone's clients in that group.
    /// Called after any client-zone assignment change.
    pub async fn reconcile_zone_groups(
        snap: &SnapcastClient,
        config: &AppConfig,
        store: &state::SharedState,
        notify: &crate::api::ws::NotifySender,
    ) {
        let status = match snap.server_get_status().await {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(error = %e, "Failed to fetch server status for reconciliation");
                return;
            }
        };

        let s = store.read().await;

        for zone_cfg in &config.zones {
            // What clients should be in this zone?
            let expected_snap_ids: Vec<String> = s
                .clients
                .values()
                .filter(|c| c.zone_index == zone_cfg.index && c.snapcast_id.is_some())
                .filter_map(|c| c.snapcast_id.clone())
                .collect();

            // Find the canonical group for this zone (by stream_id, prefer one with clients)
            let group = status
                .server
                .groups
                .iter()
                .filter(|g| g.stream_id == zone_cfg.stream_name)
                .max_by_key(|g| g.clients.len());

            if let Some(group) = group {
                let current_ids: Vec<&str> = group.clients.iter().map(|c| c.id.as_str()).collect();
                let mut expected_sorted = expected_snap_ids.clone();
                expected_sorted.sort();
                let mut current_sorted: Vec<&str> = current_ids.clone();
                current_sorted.sort_unstable();

                // Only send command if the client list differs
                if expected_sorted != current_sorted {
                    tracing::info!(
                        zone = zone_cfg.index,
                        clients = expected_snap_ids.len(),
                        "Reconcile: updating zone group"
                    );
                    tracing::debug!(zone = zone_cfg.index, expected = ?expected_snap_ids, "Reconcile details");
                    let _ = snap.group_set_clients(&group.id, expected_snap_ids).await;
                }
            }
        }

        drop(s);

        // Re-fetch and sync group IDs after reconciliation
        if let Ok(status) = snap.server_get_status().await {
            let mut s = store.write().await;
            sync_zones_from_groups(&status.server.groups, config, &mut s);
            let ws_notifs: Vec<_> = s
                .clients
                .iter()
                .map(|(&idx, c)| api::ws::Notification::ClientStateChanged {
                    client: idx,
                    volume: c.base_volume,
                    muted: c.muted,
                    connected: c.connected,
                    zone: c.zone_index,
                    is_snapdog: c.is_snapdog,
                })
                .collect::<Vec<_>>();
            drop(s);
            for n in ws_notifs {
                api::ws::broadcast_notification(notify, &n);
            }
        }
    }

    /// Build MAC → `snapcast_id` map from server status.
    #[must_use]
    pub fn build_client_mac_map(status: &ServerStatus) -> HashMap<String, String> {
        status
            .server
            .groups
            .iter()
            .flat_map(|g| &g.clients)
            .map(|c| (c.host.mac.to_lowercase(), c.id.clone()))
            .collect()
    }

    /// Build list of group IDs from server status.
    #[must_use]
    pub fn build_group_ids(status: &ServerStatus) -> Vec<String> {
        status.server.groups.iter().map(|g| g.id.clone()).collect()
    }

    /// Build group → client IDs map from server status.
    #[must_use]
    pub fn build_group_clients(status: &ServerStatus) -> HashMap<String, Vec<String>> {
        status
            .server
            .groups
            .iter()
            .map(|g| {
                (
                    g.id.clone(),
                    g.clients.iter().map(|c| c.id.clone()).collect(),
                )
            })
            .collect()
    }

    /// Handle a Snapcast server notification — update state + send WebSocket notification.
    #[allow(clippy::too_many_lines)]
    pub async fn handle_notification(
        notification: Notification,
        config: &AppConfig,
        snap: &SnapcastClient,
        store: &state::SharedState,
        notify: &crate::api::ws::NotifySender,
    ) {
        match notification {
            Notification::ClientOnConnect {
                id: _,
                client: snap_client,
            } => {
                let mac = snap_client.host.mac.to_lowercase();
                let snap_id = snap_client.id.clone();

                let mut s = store.write().await;
                if let Some((&idx, client)) = s
                    .clients
                    .iter_mut()
                    .find(|(_, c)| c.mac.to_lowercase() == mac)
                {
                    client.connected = true;
                    client.snapcast_id = Some(snap_id.clone());
                    let zone_index = client.zone_index;
                    let notif = api::ws::Notification::ClientStateChanged {
                        client: idx,
                        volume: client.volume,
                        muted: client.muted,
                        connected: client.connected,
                        zone: zone_index,
                        is_snapdog: client.is_snapdog,
                    };
                    let name = client.name.clone();
                    drop(s);
                    tracing::info!(client = %name, id = %snap_id, "Client connected");
                    crate::api::ws::broadcast_notification(notify, &notif);

                    setup_zone_group_for_client(zone_index, &snap_id, config, snap).await;
                }
            }
            Notification::ClientOnDisconnect { id: snap_id } => {
                let mut s = store.write().await;
                if let Some((&idx, client)) = s
                    .clients
                    .iter_mut()
                    .find(|(_, c)| c.snapcast_id.as_deref() == Some(&snap_id))
                {
                    client.connected = false;
                    let notif = api::ws::Notification::ClientStateChanged {
                        client: idx,
                        volume: client.volume,
                        muted: client.muted,
                        connected: client.connected,
                        zone: client.zone_index,
                        is_snapdog: client.is_snapdog,
                    };
                    let name = client.name.clone();
                    drop(s);
                    tracing::info!(client = %name, "Client disconnected");
                    crate::api::ws::broadcast_notification(notify, &notif);
                }
            }
            Notification::ClientOnVolumeChanged {
                id: snap_id,
                volume,
            } => {
                let vol = i32::from(volume.percent);
                let muted = volume.muted;
                let mut s = store.write().await;
                if let Some((&idx, client)) = s
                    .clients
                    .iter_mut()
                    .find(|(_, c)| c.snapcast_id.as_deref() == Some(&snap_id))
                {
                    client.volume = vol;
                    client.muted = muted;
                    let notif = api::ws::Notification::ClientStateChanged {
                        client: idx,
                        volume: client.volume,
                        muted: client.muted,
                        connected: client.connected,
                        zone: client.zone_index,
                        is_snapdog: client.is_snapdog,
                    };
                    let name = client.name.clone();
                    drop(s);
                    tracing::debug!(client = %name, volume = vol, muted, "Volume changed");
                    crate::api::ws::broadcast_notification(notify, &notif);
                }
            }
            Notification::ClientOnLatencyChanged {
                id: snap_id,
                latency,
            } => {
                #[allow(clippy::cast_possible_wrap)]
                let lat = latency as i32;
                let mut s = store.write().await;
                if let Some((&idx, client)) = s
                    .clients
                    .iter_mut()
                    .find(|(_, c)| c.snapcast_id.as_deref() == Some(&snap_id))
                {
                    client.latency_ms = lat;
                    let notif = api::ws::Notification::ClientStateChanged {
                        client: idx,
                        volume: client.volume,
                        muted: client.muted,
                        connected: client.connected,
                        zone: client.zone_index,
                        is_snapdog: client.is_snapdog,
                    };
                    let name = client.name.clone();
                    drop(s);
                    tracing::info!(client = %name, latency = lat, "Latency changed");
                    crate::api::ws::broadcast_notification(notify, &notif);
                }
            }
            Notification::ClientOnNameChanged {
                id: snap_id,
                name: new_name,
            } => {
                let mut s = store.write().await;
                if let Some((&idx, client)) = s
                    .clients
                    .iter_mut()
                    .find(|(_, c)| c.snapcast_id.as_deref() == Some(&snap_id))
                {
                    client.name.clone_from(&new_name);
                    let notif = api::ws::Notification::ClientStateChanged {
                        client: idx,
                        volume: client.volume,
                        muted: client.muted,
                        connected: client.connected,
                        zone: client.zone_index,
                        is_snapdog: client.is_snapdog,
                    };
                    drop(s);
                    tracing::info!(client = %new_name, "Name changed");
                    crate::api::ws::broadcast_notification(notify, &notif);
                }
            }
            Notification::GroupOnMute { id, mute } => {
                tracing::debug!(group = %id, mute, "Group mute changed");
            }
            Notification::GroupOnStreamChanged { id, stream_id } => {
                tracing::debug!(group = %id, stream = %stream_id, "Group stream changed");
            }
            Notification::GroupOnNameChanged { id, name } => {
                tracing::debug!(group = %id, name = %name, "Group name changed");
            }
            Notification::ServerOnUpdate { server } => {
                tracing::debug!("Server update — syncing group IDs");
                let mut s = store.write().await;
                // Only update group IDs, NOT zone_index (zone assignment is SnapDog-owned)
                for zone_cfg in &config.zones {
                    let group = server
                        .groups
                        .iter()
                        .filter(|g| g.stream_id == zone_cfg.stream_name)
                        .max_by_key(|g| g.clients.len());
                    if let Some(group) = group {
                        if let Some(zone) = s.zones.get_mut(&zone_cfg.index) {
                            if zone.snapcast_group_id.as_deref() != Some(&group.id) {
                                tracing::debug!(zone = zone_cfg.index, new = %group.id, "Zone group ID updated");
                                zone.snapcast_group_id = Some(group.id.clone());
                            }
                        }
                    }
                }
            }
            Notification::StreamOnUpdate { id, stream } => {
                tracing::trace!(stream = %id, status = ?stream.status, "Stream status updated");
            }
            Notification::StreamOnProperties { id, .. } => {
                tracing::trace!(stream = %id, "Stream properties updated");
            }
            Notification::Unknown { method, .. } => {
                tracing::debug!(method = %method, "Unknown Snapcast notification");
            }
        }
    }

    /// Setup zone group when a client connects.
    async fn setup_zone_group_for_client(
        zone_index: usize,
        snap_client_id: &str,
        config: &AppConfig,
        snap: &SnapcastClient,
    ) {
        // Re-fetch current state to get fresh group info
        let status = match snap.server_get_status().await {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(error = %e, "Failed to refresh Snapcast state for group setup");
                return;
            }
        };

        let zone_config = &config.zones[zone_index - 1];

        let zone_macs: Vec<String> = config
            .clients
            .iter()
            .filter(|c| c.zone_index == zone_index)
            .map(|c| c.mac.to_lowercase())
            .collect();

        let snap_client_ids: Vec<String> = zone_macs
            .iter()
            .filter_map(|mac| {
                status
                    .server
                    .groups
                    .iter()
                    .flat_map(|g| &g.clients)
                    .find(|c| c.host.mac.to_lowercase() == *mac)
                    .map(|c| c.id.clone())
            })
            .collect();

        if snap_client_ids.is_empty() {
            return;
        }

        let gid = status
            .server
            .groups
            .iter()
            .find(|g| g.clients.iter().any(|c| c.id == snap_client_id))
            .map(|g| g.id.clone())
            .or_else(|| status.server.groups.first().map(|g| g.id.clone()));

        let Some(gid) = gid else {
            tracing::warn!(
                zone = zone_index,
                "No Snapcast groups available for zone setup"
            );
            return;
        };

        if let Err(e) = snap.group_set_clients(&gid, snap_client_ids.clone()).await {
            tracing::warn!(error = %e, "Failed to set group clients");
        }
        if let Err(e) = snap.group_set_stream(&gid, &zone_config.stream_name).await {
            tracing::warn!(error = %e, "Failed to set group stream");
        }
        if let Err(e) = snap.group_set_name(&gid, &zone_config.name).await {
            tracing::warn!(error = %e, "Failed to set group name");
        }

        tracing::info!(
            zone = zone_index,
            group = %gid,
            clients = ?snap_client_ids,
            stream = %zone_config.stream_name,
            "Zone group configured dynamically"
        );
    }
} // mod process_impl

#[cfg(feature = "snapcast-process")]
pub use process_impl::*;
