// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! `ZonePlayer` shared context and Snapcast command types.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::mpsc;

use crate::audio;

use crate::config::AppConfig;
use crate::snapcast::backend::SnapcastBackend;
use crate::state;
use crate::state::cover::SharedCoverCache;

/// Broadcast sender for WebSocket notifications.
pub type NotifySender = crate::api::ws::NotifySender;
/// Channel sender for zone player commands.
pub type ZoneCommandSender = mpsc::Sender<super::ZoneCommand>;
/// Channel sender for Snapcast JSON-RPC commands.
pub type SnapcastCmdSender = mpsc::Sender<SnapcastCmd>;

/// Shared context for all `ZonePlayers`. Cloned (Arc) per zone task.
pub struct ZonePlayerContext {
    /// Application configuration (zones, clients, audio settings, etc.).
    pub config: Arc<AppConfig>,
    /// Shared application state (zone states, client states).
    pub store: state::SharedState,
    /// Content-addressed cover art cache (`AirPlay` only).
    pub covers: SharedCoverCache,
    /// Broadcast sender for WebSocket notifications.
    pub notify: NotifySender,
    /// Channel to send Snapcast JSON-RPC commands to the main task.
    pub snap_tx: SnapcastCmdSender,
    /// Snapcast backend (embedded or process).
    pub backend: Arc<dyn SnapcastBackend>,
    /// Shared parametric EQ configuration store.
    pub eq_store: Arc<std::sync::Mutex<crate::audio::eq::EqStore>>,
    /// Pre-extracted: Snapcast client MAC (lowercase) → Snapcast client ID.
    pub client_mac_map: HashMap<String, String>,
    /// Pre-extracted: all Snapcast group IDs.
    pub group_ids: Vec<String>,
    /// Pre-extracted: group ID → client IDs in that group.
    pub group_clients: HashMap<String, Vec<String>>,
}

/// Command sent to the main loop for Snapcast JSON-RPC execution.
///
/// The Snapcast connection is `!Send`, so all JSON-RPC calls must happen
/// on the main task. Zone players and API handlers send commands via channel.
#[derive(Debug)]
pub enum SnapcastCmd {
    /// Group-level command (volume, mute, stream, clients, name).
    Group {
        /// Snapcast group ID to target.
        group_id: String,
        /// The group action to perform.
        action: GroupAction,
    },
    /// Client-level command (volume, mute, latency).
    Client {
        /// Snapcast client ID to target.
        client_id: String,
        /// The client action to perform.
        action: ClientAction,
    },
    /// Re-sync Snapcast groups to match zone assignments from config.
    ReconcileZones,
}

/// Actions that can be performed on a Snapcast group.
#[derive(Debug)]
pub enum GroupAction {
    /// Set the group's audio stream by name.
    Stream(String),
    /// Assign a list of client IDs to the group.
    Clients(Vec<String>),
    /// Set the group's display name.
    Name(String),
    /// Set the group volume (0–100).
    Volume(i32),
    /// Set the group mute state.
    Mute(bool),
}

/// Actions that can be performed on a Snapcast client.
#[derive(Debug)]
pub enum ClientAction {
    /// Set the client volume (0–100).
    Volume(i32),
    /// Adjust the client volume by a relative delta (from DPT 3.007 dimming).
    AdjustVolume(i32),
    /// Set the client mute state.
    Mute(bool),
    /// Set the client latency offset in milliseconds.
    Latency(i32),
    /// Send a custom binary message to the client.
    SendCustom {
        /// Custom message type ID (9+).
        type_id: u16,
        /// Raw payload bytes.
        payload: Vec<u8>,
    },
}

/// Stop the current decode task and clear the PCM receiver.
pub async fn stop_decode(
    current: &mut Option<tokio::task::JoinHandle<()>>,
    rx: &mut Option<mpsc::Receiver<audio::PcmMessage>>,
) {
    if let Some(handle) = current.take() {
        handle.abort();
    }
    *rx = None;
}

/// Update zone state and broadcast a WebSocket notification.
pub async fn update_and_notify(
    store: &state::SharedState,
    zone_index: usize,
    notify: &NotifySender,
    f: impl FnOnce(&mut state::ZoneState),
) {
    let notification = {
        let mut s = store.write().await;
        let Some(zone) = s.zones.get_mut(&zone_index) else {
            return;
        };
        f(zone);
        let track = zone.track.as_ref();
        let n = crate::api::ws::Notification::ZoneChanged {
            zone: zone_index,
            playback: zone.playback.to_string(),
            source: zone.source.to_string(),
            shuffle: zone.shuffle,
            repeat: zone.repeat,
            title: track.map_or_else(String::new, |t| t.title.clone()),
            artist: track.map_or_else(String::new, |t| t.artist.clone()),
            album: track.map_or_else(String::new, |t| t.album.clone()),
            album_artist: track.and_then(|t| t.album_artist.clone()),
            genre: track.and_then(|t| t.genre.clone()),
            year: track.and_then(|t| t.year),
            track_number: track.and_then(|t| t.track_number),
            disc_number: track.and_then(|t| t.disc_number),
            duration_ms: track.map_or(0, |t| t.duration_ms),
            position_ms: track.map_or(0, |t| t.position_ms),
            seekable: track.is_some_and(|t| t.seekable),
            cover_url: zone.cover_url.clone(),
            bitrate_kbps: track.and_then(|t| t.bitrate_kbps),
            content_type: track.and_then(|t| t.content_type.clone()),
            playlist_index: zone.playlist_track_index,
            playlist_count: zone.playlist_track_count,
            can_next: zone
                .playlist_track_count
                .is_some_and(|c| zone.playlist_track_index.is_some_and(|i| i + 1 < c)),
            can_prev: zone.playlist_track_index.is_some_and(|i| i > 0),
            volume: zone.volume,
            muted: zone.muted,
        };
        s.dirty = true;
        drop(s);
        n
    };
    crate::api::ws::broadcast_notification(notify, &notification);
}

/// Update zone volume/mute and broadcast a lightweight `ZoneVolumeChanged` notification.
pub async fn update_volume_and_notify(
    store: &state::SharedState,
    zone_index: usize,
    notify: &NotifySender,
    f: impl FnOnce(&mut state::ZoneState),
) {
    let notification = {
        let mut s = store.write().await;
        let Some(zone) = s.zones.get_mut(&zone_index) else {
            return;
        };
        f(zone);
        let n = crate::api::ws::Notification::ZoneVolumeChanged {
            zone: zone_index,
            volume: zone.volume,
            muted: zone.muted,
        };
        s.dirty = true;
        drop(s);
        n
    };
    crate::api::ws::broadcast_notification(notify, &notification);
}

/// Set up Snapcast group for a zone: find clients by MAC, assign to group, set stream.
pub async fn setup_zone_group(zone_index: usize, ctx: &ZonePlayerContext) -> Option<String> {
    let zone_config = &ctx.config.zones[zone_index - 1];
    let zone_macs: Vec<String> = ctx
        .config
        .clients
        .iter()
        .filter(|c| c.zone_index == zone_index)
        .map(|c| c.mac.to_lowercase())
        .collect();

    if zone_macs.is_empty() {
        return None;
    }

    let snap_client_ids: Vec<String> = zone_macs
        .iter()
        .filter_map(|mac| ctx.client_mac_map.get(mac).cloned())
        .collect();

    if snap_client_ids.is_empty() {
        tracing::warn!(zone = zone_index, macs = ?zone_macs, "No Snapcast clients found");
        return None;
    }

    let gid = ctx
        .group_clients
        .iter()
        .find(|(_, clients)| clients.iter().any(|c| snap_client_ids.contains(c)))
        .map(|(id, _)| id.clone())
        .or_else(|| ctx.group_ids.first().cloned());

    let Some(gid) = gid else {
        tracing::warn!(zone = zone_index, "No Snapcast groups available");
        return None;
    };

    let _ = ctx
        .snap_tx
        .send(SnapcastCmd::Group {
            group_id: gid.clone(),
            action: GroupAction::Stream(zone_config.stream_name.clone()),
        })
        .await;
    let _ = ctx
        .snap_tx
        .send(SnapcastCmd::Group {
            group_id: gid.clone(),
            action: GroupAction::Clients(snap_client_ids.clone()),
        })
        .await;
    let _ = ctx
        .snap_tx
        .send(SnapcastCmd::Group {
            group_id: gid.clone(),
            action: GroupAction::Name(zone_config.name.clone()),
        })
        .await;

    tracing::info!(
        zone = zone_index,
        clients = snap_client_ids.len(),
        "Zone group assigned"
    );
    Some(gid)
}
