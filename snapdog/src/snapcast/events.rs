// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! Snapcast event handler — maps backend events to state updates and notifications.

use std::sync::Arc;

use crate::api;
use crate::audio::eq::{EqStore, TYPE_EQ_CONFIG};
use crate::config::AppConfig;
use crate::player::{ClientAction, GroupAction, SnapcastCmd};
use crate::snapcast::backend::{SnapcastBackend, SnapcastEvent};
use crate::state;

/// Shared EQ store type.
pub type SharedEqStore = Arc<std::sync::Mutex<EqStore>>;

/// Spawn a task that receives events from the backend and updates state.
#[cfg(feature = "snapcast-embedded")]
pub fn spawn_event_handler(
    mut event_rx: super::embedded::EmbeddedEventReceiver,
    config: Arc<AppConfig>,
    backend: Arc<dyn SnapcastBackend>,
    store: state::SharedState,
    notify: api::ws::NotifySender,
    eq_store: SharedEqStore,
) {
    tokio::spawn(async move {
        tracing::info!("Snapcast event handler started");
        while let Some(event) = event_rx.recv().await {
            handle_event(event, &config, &*backend, &store, &notify, &eq_store).await;
        }
        tracing::info!("Snapcast event handler stopped");
    });
}

async fn handle_event(
    event: SnapcastEvent,
    config: &AppConfig,
    backend: &dyn SnapcastBackend,
    store: &state::SharedState,
    notify: &api::ws::NotifySender,
    eq_store: &SharedEqStore,
) {
    match event {
        SnapcastEvent::ClientConnected { id, hello } => {
            let is_snapdog = hello.client_name == crate::SNAPDOG_CLIENT_NAME;
            let zone_index = {
                let mut s = store.write().await;
                let matched = if hello.mac.is_empty() {
                    s.clients
                        .iter_mut()
                        .find(|(_, c)| c.name == hello.host_name)
                } else {
                    s.clients
                        .iter_mut()
                        .find(|(_, c)| c.mac.to_lowercase() == hello.mac)
                };
                if let Some((&client_index, client)) = matched {
                    client.connected = true;
                    client.snapcast_id = Some(id.clone());
                    client.is_snapdog = is_snapdog;
                    tracing::info!(
                        client = %client.name, id = %id,
                        mac = %hello.mac, host = %hello.host_name,
                        client_name = %hello.client_name, version = %hello.version,
                        "Client connected"
                    );
                    Some((client.zone_index, client_index))
                } else {
                    tracing::info!(
                        id = %id,
                        mac = %hello.mac, host = %hello.host_name,
                        client_name = %hello.client_name, version = %hello.version,
                        "Unknown client connected (not in config)"
                    );
                    // Handle based on unknown_clients policy
                    if config.snapcast.unknown_clients == crate::config::UnknownClientPolicy::Accept
                    {
                        let zone_index = config
                            .snapcast
                            .default_zone
                            .as_ref()
                            .and_then(|name| config.zones.iter().find(|z| z.name == *name))
                            .map_or_else(
                                || config.zones.first().map_or(1, |z| z.index),
                                |z| z.index,
                            );
                        let client_index = {
                            let mut s = store.write().await;
                            let next_idx = s.clients.keys().max().copied().unwrap_or(0) + 1;
                            s.clients.insert(
                                next_idx,
                                crate::state::ClientState {
                                    name: hello.host_name.clone(),
                                    icon: String::new(),
                                    mac: hello.mac.clone(),
                                    zone_index,
                                    volume: crate::state::DEFAULT_VOLUME,
                                    base_volume: crate::state::DEFAULT_VOLUME,
                                    muted: false,
                                    latency_ms: 0,
                                    connected: true,
                                    snapcast_id: Some(id.clone()),
                                    max_volume: 100,
                                    is_snapdog,
                                },
                            );
                            next_idx
                        };
                        tracing::info!(
                            client = %hello.host_name, zone = zone_index,
                            "Unknown client accepted and assigned to zone"
                        );
                        Some((zone_index, client_index))
                    } else {
                        None
                    }
                }
            };
            broadcast_all_clients(store, notify).await;

            // Setup zone group for connecting client.
            // - First connect: full setup (assign all connected clients, stream, name)
            // - Subsequent connects: only merge client into existing zone group if needed
            if let Some((zone_index, client_index)) = zone_index {
                let zone_group_id = {
                    let s = store.read().await;
                    s.zones
                        .get(&zone_index)
                        .and_then(|z| z.snapcast_group_id.clone())
                };
                match zone_group_id {
                    None => {
                        setup_zone_group(zone_index, &id, config, backend, store).await;
                    }
                    Some(gid) => {
                        // Client may be in a different group — merge into zone group
                        merge_client_into_group(&id, &gid, zone_index, backend, store).await;
                    }
                }

                // Push snapdog's persisted volume/mute to snapcast (SSOT)
                let (push_vol, push_mute) = {
                    let s = store.read().await;
                    s.clients
                        .get(&client_index)
                        .map_or((crate::state::DEFAULT_VOLUME, false), |c| {
                            (c.base_volume, c.muted)
                        })
                };
                let _ = backend
                    .execute(SnapcastCmd::Client {
                        client_id: id.clone(),
                        action: ClientAction::Volume(push_vol),
                    })
                    .await;
                if push_mute {
                    let _ = backend
                        .execute(SnapcastCmd::Client {
                            client_id: id.clone(),
                            action: ClientAction::Mute(true),
                        })
                        .await;
                }

                // Push persisted client EQ config (only to SnapDog clients)
                if is_snapdog {
                    let eq_config = eq_store
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .get_client(client_index);
                    if eq_config.enabled && !eq_config.bands.is_empty() {
                        if let Ok(payload) = serde_json::to_vec(&eq_config) {
                            let _ = backend
                                .execute(SnapcastCmd::Client {
                                    client_id: id.clone(),
                                    action: ClientAction::SendCustom {
                                        type_id: TYPE_EQ_CONFIG,
                                        payload,
                                    },
                                })
                                .await;
                        }
                    }

                    // Push persisted speaker correction config
                    let speaker_config = eq_store
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .get_speaker_correction(client_index);
                    if speaker_config.enabled && !speaker_config.bands.is_empty() {
                        if let Ok(payload) = serde_json::to_vec(&speaker_config) {
                            let _ = backend
                                .execute(SnapcastCmd::Client {
                                    client_id: id,
                                    action: ClientAction::SendCustom {
                                        type_id: snapdog_common::MSG_TYPE_SPEAKER_EQ,
                                        payload,
                                    },
                                })
                                .await;
                        }
                    }
                }
            }
        }
        SnapcastEvent::ClientDisconnected { id } => {
            let mut s = store.write().await;
            if let Some(client) = s
                .clients
                .values_mut()
                .find(|c| c.snapcast_id.as_deref() == Some(&id))
            {
                client.connected = false;
                tracing::info!(client = %client.name, "Client disconnected");
            }
            drop(s);
            broadcast_all_clients(store, notify).await;
        }
        SnapcastEvent::ClientVolumeChanged { id, volume, muted } => {
            let mut s = store.write().await;
            if let Some((&idx, client)) = s
                .clients
                .iter_mut()
                .find(|(_, c)| c.snapcast_id.as_deref() == Some(&id))
            {
                client.volume = volume;
                client.muted = muted;
                let n = client_notification(idx, client);
                drop(s);
                crate::api::ws::broadcast_notification(notify, &n);
            }
        }
        SnapcastEvent::ClientLatencyChanged { id, latency } => {
            let mut s = store.write().await;
            if let Some((&idx, client)) = s
                .clients
                .iter_mut()
                .find(|(_, c)| c.snapcast_id.as_deref() == Some(&id))
            {
                client.latency_ms = latency;
                let n = client_notification(idx, client);
                drop(s);
                crate::api::ws::broadcast_notification(notify, &n);
            }
        }
        SnapcastEvent::ClientNameChanged { id, name } => {
            let mut s = store.write().await;
            if let Some((&idx, client)) = s
                .clients
                .iter_mut()
                .find(|(_, c)| c.snapcast_id.as_deref() == Some(&id))
            {
                client.name = name;
                let n = client_notification(idx, client);
                drop(s);
                crate::api::ws::broadcast_notification(notify, &n);
            }
        }
        SnapcastEvent::ServerUpdated => {
            sync_group_ids(config, backend, store).await;
        }
    }
}

/// Assign all clients of a zone to the same group, set stream + name.
async fn setup_zone_group(
    zone_index: usize,
    connecting_client_id: &str,
    config: &AppConfig,
    backend: &dyn SnapcastBackend,
    store: &state::SharedState,
) {
    let Some(zone_config) = config.zones.get(zone_index - 1) else {
        return;
    };

    // Get current server status to find group IDs
    let status = match backend.get_status().await {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(error = %e, "Failed to get server status for group setup");
            return;
        }
    };

    let Some(groups) = status.get("server").and_then(|s| s.get("groups")) else {
        return;
    };

    // Find all Snapcast client IDs that belong to this zone
    let s = store.read().await;
    let snap_client_ids: Vec<String> = s
        .clients
        .values()
        .filter(|c| c.zone_index == zone_index && c.connected)
        .filter_map(|c| c.snapcast_id.clone())
        .collect();
    drop(s);

    if snap_client_ids.is_empty() {
        return;
    }

    // Find the group the connecting client is in
    let group = groups.as_array().and_then(|groups| {
        groups.iter().find(|g| {
            g.get("clients")
                .and_then(|c| c.as_array())
                .is_some_and(|clients| {
                    clients.iter().any(|c| {
                        c.get("id").and_then(|id| id.as_str()) == Some(connecting_client_id)
                    })
                })
        })
    });

    let Some(group) = group else {
        tracing::warn!(zone = zone_index, "No group found for connecting client");
        return;
    };

    let gid = match group.get("id").and_then(|id| id.as_str()) {
        Some(id) => id.to_string(),
        None => return,
    };

    // Extract current group state to avoid redundant commands
    let current_clients: Vec<&str> = group
        .get("clients")
        .and_then(|c| c.as_array())
        .map(|clients| {
            clients
                .iter()
                .filter_map(|c| c.get("id").and_then(|id| id.as_str()))
                .collect()
        })
        .unwrap_or_default();
    let current_stream = group
        .get("stream_id")
        .and_then(|s| s.as_str())
        .unwrap_or("");
    let current_name = group.get("name").and_then(|n| n.as_str()).unwrap_or("");

    // Only send commands for things that actually differ
    let mut sorted_want = snap_client_ids.clone();
    sorted_want.sort_unstable();
    let mut sorted_have: Vec<&str> = current_clients;
    sorted_have.sort_unstable();
    let clients_match = sorted_want.len() == sorted_have.len()
        && sorted_want.iter().zip(&sorted_have).all(|(a, b)| a == b);

    if !clients_match {
        if let Err(e) = backend
            .execute(SnapcastCmd::Group {
                group_id: gid.clone(),
                action: GroupAction::Clients(snap_client_ids.clone()),
            })
            .await
        {
            tracing::warn!(error = %e, "Failed to set group clients");
        }
    }
    if current_stream != zone_config.stream_name {
        if let Err(e) = backend
            .execute(SnapcastCmd::Group {
                group_id: gid.clone(),
                action: GroupAction::Stream(zone_config.stream_name.clone()),
            })
            .await
        {
            tracing::warn!(error = %e, "Failed to set group stream");
        }
    }
    if current_name != zone_config.name {
        if let Err(e) = backend
            .execute(SnapcastCmd::Group {
                group_id: gid.clone(),
                action: GroupAction::Name(zone_config.name.clone()),
            })
            .await
        {
            tracing::warn!(error = %e, "Failed to set group name");
        }
    }

    // Store group ID
    let mut s = store.write().await;
    if let Some(zone) = s.zones.get_mut(&zone_index) {
        zone.snapcast_group_id = Some(gid.clone());
    }

    tracing::info!(
        zone = zone_index,
        group = %gid,
        clients = ?snap_client_ids,
        stream = %zone_config.stream_name,
        "Zone group configured"
    );
}

/// Merge a reconnecting client into its zone's existing group if it ended up elsewhere.
async fn merge_client_into_group(
    client_id: &str,
    zone_group_id: &str,
    zone_index: usize,
    backend: &dyn SnapcastBackend,
    store: &state::SharedState,
) {
    let Ok(status) = backend.get_status().await else {
        return;
    };

    let Some(groups) = status
        .get("server")
        .and_then(|s| s.get("groups"))
        .and_then(|g| g.as_array())
    else {
        return;
    };

    // Check if client is already in the zone's group
    let already_in_group = groups.iter().any(|g| {
        g.get("id").and_then(|id| id.as_str()) == Some(zone_group_id)
            && g.get("clients")
                .and_then(|c| c.as_array())
                .is_some_and(|clients| {
                    clients
                        .iter()
                        .any(|c| c.get("id").and_then(|id| id.as_str()) == Some(client_id))
                })
    });

    if already_in_group {
        return;
    }

    // Client is in a different group — collect all zone clients and merge
    let s = store.read().await;
    let all_zone_clients: Vec<String> = s
        .clients
        .values()
        .filter(|c| c.zone_index == zone_index)
        .filter_map(|c| c.snapcast_id.clone())
        .collect();
    drop(s);

    if let Err(e) = backend
        .execute(SnapcastCmd::Group {
            group_id: zone_group_id.to_string(),
            action: GroupAction::Clients(all_zone_clients),
        })
        .await
    {
        tracing::warn!(error = %e, "Failed to merge client into zone group");
    }
}

/// Re-sync zone group IDs from server status.
///
/// The server may reorganize groups (new client → new group, `SetGroupClients` → merge/delete).
/// The stream name is the stable identifier; the group ID is ephemeral.
async fn sync_group_ids(
    config: &AppConfig,
    backend: &dyn SnapcastBackend,
    store: &state::SharedState,
) {
    let Ok(status) = backend.get_status().await else {
        tracing::warn!("Failed to get server status for group sync");
        return;
    };

    let Some(groups) = status
        .get("server")
        .and_then(|s| s.get("groups"))
        .and_then(|g| g.as_array())
    else {
        return;
    };

    let mut s = store.write().await;

    // Clear all group IDs first — stale references to deleted groups are the
    // primary cause of zone-switch failures. Re-assign from current server state below.
    for zone in s.zones.values_mut() {
        zone.snapcast_group_id = None;
    }

    for zone_cfg in &config.zones {
        // Find the group whose stream matches this zone, prefer the one with most clients
        let best_group = groups
            .iter()
            .filter(|g| g.get("stream_id").and_then(|s| s.as_str()) == Some(&zone_cfg.stream_name))
            .max_by_key(|g| {
                g.get("clients")
                    .and_then(|c| c.as_array())
                    .map_or(0, std::vec::Vec::len)
            });

        if let Some(group) = best_group {
            if let Some(gid) = group.get("id").and_then(|id| id.as_str()) {
                if let Some(zone) = s.zones.get_mut(&zone_cfg.index) {
                    if zone.snapcast_group_id.as_deref() != Some(gid) {
                        tracing::debug!(zone = zone_cfg.index, new = %gid, "Zone group ID updated");
                        zone.snapcast_group_id = Some(gid.to_string());
                    }
                    let group_muted = group
                        .get("muted")
                        .and_then(serde_json::Value::as_bool)
                        .unwrap_or(false);
                    if zone.muted != group_muted {
                        tracing::debug!(
                            zone = zone_cfg.index,
                            muted = group_muted,
                            "Zone mute synced from Snapcast group"
                        );
                        zone.muted = group_muted;
                    }
                }
            }
        }
    }

    // Second pass: find groups with clients assigned to a zone but wrong stream.
    for zone_cfg in &config.zones {
        // Skip zones that already matched a group above
        if s.zones
            .get(&zone_cfg.index)
            .and_then(|z| z.snapcast_group_id.as_ref())
            .is_some()
        {
            continue;
        }
        // Find groups containing clients assigned to this zone
        let zone_snap_ids: Vec<&str> = s
            .clients
            .values()
            .filter(|c| c.zone_index == zone_cfg.index)
            .filter_map(|c| c.snapcast_id.as_deref())
            .collect();
        if zone_snap_ids.is_empty() {
            continue;
        }
        for group in groups {
            let group_clients: Vec<&str> = group
                .get("clients")
                .and_then(|c| c.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|c| c.get("id").and_then(|id| id.as_str()))
                        .collect()
                })
                .unwrap_or_default();
            let has_our_client = zone_snap_ids.iter().any(|id| group_clients.contains(id));
            if !has_our_client {
                continue;
            }
            let group_stream = group
                .get("stream_id")
                .and_then(|s| s.as_str())
                .unwrap_or("");
            if group_stream == zone_cfg.stream_name {
                // Stream already correct — adopt this group
                if let Some(gid) = group.get("id").and_then(|id| id.as_str()) {
                    if let Some(zone) = s.zones.get_mut(&zone_cfg.index) {
                        tracing::debug!(zone = zone_cfg.index, group = %gid, "Adopted group for zone");
                        zone.snapcast_group_id = Some(gid.to_string());
                    }
                }
            } else if let Some(gid) = group.get("id").and_then(|id| id.as_str()) {
                // Wrong stream — fix it
                tracing::info!(zone = zone_cfg.index, group = %gid, stream = %zone_cfg.stream_name, "Fixing group stream for zone");
                drop(s);
                let _ = backend
                    .execute(SnapcastCmd::Group {
                        group_id: gid.to_string(),
                        action: GroupAction::Stream(zone_cfg.stream_name.clone()),
                    })
                    .await;
                return; // Re-sync will happen via the next ServerUpdated
            }
            break;
        }
    }
}

const fn client_notification(idx: usize, client: &state::ClientState) -> api::ws::Notification {
    api::ws::Notification::ClientStateChanged {
        client: idx,
        volume: client.base_volume,
        muted: client.muted,
        connected: client.connected,
        zone: client.zone_index,
        is_snapdog: client.is_snapdog,
    }
}

async fn broadcast_all_clients(store: &state::SharedState, notify: &api::ws::NotifySender) {
    let s = store.read().await;
    for (&idx, client) in &s.clients {
        api::ws::broadcast_notification(notify, &client_notification(idx, client));
    }
}
