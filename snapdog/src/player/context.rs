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
    let notifications = {
        let mut s = store.write().await;
        let Some(zone) = s.zones.get_mut(&zone_index) else {
            return;
        };
        let old_track_title = zone.track.as_ref().map(|t| t.title.clone());
        let old_track_artist = zone.track.as_ref().map(|t| t.artist.clone());
        let old_position = zone.track.as_ref().map(|t| t.position_ms);
        let old_cover_url = zone.cover_url.clone();
        f(zone);
        let playback = zone.playback.to_string();
        let volume = zone.volume;
        let muted = zone.muted;
        let source = zone.source.to_string();
        let shuffle = zone.shuffle;
        let repeat = zone.repeat;
        let track_repeat = zone.track_repeat;

        // Send track changed if title, artist, or cover changed
        let new_track_title = zone.track.as_ref().map(|t| t.title.clone());
        let new_track_artist = zone.track.as_ref().map(|t| t.artist.clone());

        let track_changed_notif = if old_track_title != new_track_title
            || old_track_artist != new_track_artist
            || old_cover_url != zone.cover_url
        {
            zone.track
                .as_ref()
                .map(|t| crate::api::ws::Notification::ZoneTrackChanged {
                    zone: zone_index,
                    title: t.title.clone(),
                    artist: t.artist.clone(),
                    album: t.album.clone(),
                    duration_ms: t.duration_ms,
                    position_ms: t.position_ms,
                    seekable: t.seekable,
                    cover_url: zone.cover_url.clone(),
                })
        } else {
            None
        };

        // Send progress if position changed
        let new_position = zone.track.as_ref().map(|t| t.position_ms);
        let progress_notif = if old_position == new_position {
            None
        } else {
            zone.track
                .as_ref()
                .map(|t| crate::api::ws::Notification::ZoneProgress {
                    zone: zone_index,
                    position_ms: t.position_ms,
                    duration_ms: t.duration_ms,
                    buffered_ms: zone.buffered_ms,
                })
        };

        s.dirty = true;

        let mut notifs = vec![crate::api::ws::Notification::ZoneStateChanged {
            zone: zone_index,
            playback,
            volume,
            muted,
            source,
            shuffle,
            repeat,
            track_repeat,
        }];
        if let Some(n) = track_changed_notif {
            notifs.push(n);
        }
        if let Some(n) = progress_notif {
            notifs.push(n);
        }
        notifs
    };
    for n in notifications {
        crate::api::ws::broadcast_notification(notify, &n);
    }
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
