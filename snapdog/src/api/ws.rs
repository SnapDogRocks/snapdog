// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! WebSocket endpoint for real-time state notifications.

use axum::Router;

/// WebSocket ping interval to detect dead connections.
const WS_PING_INTERVAL: std::time::Duration = std::time::Duration::from_secs(30);
/// Capacity of the notification broadcast channel.
const NOTIFICATION_CHANNEL_SIZE: usize = 256;
/// Maximum number of concurrent WebSocket connections.
const MAX_WS_CONNECTIONS: usize = 64;

use std::sync::atomic::{AtomicUsize, Ordering};

static ACTIVE_CONNECTIONS: AtomicUsize = AtomicUsize::new(0);

use axum::extract::State;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};

use axum::response::IntoResponse;

use axum::routing::get;

use serde::Serialize;

use tokio::sync::broadcast;

use crate::api::SharedState;

/// Notification broadcast to all connected WebSocket clients.
#[derive(Debug, Clone, Serialize, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[allow(clippy::large_enum_variant)]
pub enum Notification {
    /// Full zone state update.
    ZoneChanged {
        /// Zone index (1-based).
        zone: usize,
        // Playback
        /// Playback state: "playing", "paused", or "stopped".
        playback: String,
        /// Active source name.
        source: String,
        /// Whether shuffle is enabled.
        shuffle: bool,
        /// Repeat mode.
        repeat: snapdog_common::RepeatMode,
        // Track metadata
        /// Track title.
        title: String,
        /// Track artist.
        artist: String,
        /// Track album.
        album: String,
        /// Album artist.
        album_artist: Option<String>,
        /// Genre tag.
        genre: Option<String>,
        /// Release year.
        year: Option<u32>,
        /// Track number.
        track_number: Option<u32>,
        /// Disc number.
        disc_number: Option<u32>,
        /// Total track duration in milliseconds.
        duration_ms: i64,
        /// Current playback position in milliseconds.
        position_ms: i64,
        /// Whether the track supports seeking.
        seekable: bool,
        /// Cover art URL.
        cover_url: Option<String>,
        // Stream info
        /// Audio bitrate in kbps.
        bitrate_kbps: Option<u32>,
        /// MIME content type.
        content_type: Option<String>,
        // Playlist position
        /// Current track index in playlist.
        track_index: Option<usize>,
        /// Total tracks in playlist.
        track_count: Option<usize>,
        // Playlist navigation
        /// Current playlist index (0-based).
        playlist: Option<usize>,
        /// Name of the active playlist.
        playlist_name: Option<String>,
        /// Total number of playlists available.
        playlist_total: Option<usize>,
        /// Whether there's a next playlist.
        can_playlist_next: bool,
        /// Whether there's a previous playlist.
        can_playlist_prev: bool,
        // Navigation
        /// Whether next track is available.
        can_next: bool,
        /// Whether previous track is available.
        can_prev: bool,
        // Volume
        /// Zone volume (0–100).
        volume: i32,
        /// Whether the zone is muted.
        muted: bool,
    },
    /// Zone volume changed (high-frequency, lightweight).
    ZoneVolumeChanged {
        /// Zone index (1-based).
        zone: usize,
        /// Zone volume (0–100).
        volume: i32,
        /// Whether the zone is muted.
        muted: bool,
    },
    /// Periodic playback position update for a zone.
    ZoneProgress {
        /// Zone index (1-based).
        zone: usize,
        /// Current playback position in milliseconds.
        position_ms: i64,
        /// Total track duration in milliseconds.
        duration_ms: i64,
        /// Buffered position in milliseconds (for stream-and-cache progress).
        #[serde(skip_serializing_if = "Option::is_none")]
        buffered_ms: Option<i64>,
    },
    /// Client connection or volume state changed.
    ClientStateChanged {
        /// Client index (1-based).
        client: usize,
        /// Client volume (0–100).
        volume: i32,
        /// Whether the client is muted.
        muted: bool,
        /// Whether the client is connected to Snapcast.
        connected: bool,
        /// Zone the client belongs to (1-based).
        zone: usize,
        /// Whether this is a `SnapDog` client (supports EQ).
        is_snapdog: bool,
    },
    /// Zone equalizer configuration changed.
    ZoneEqChanged {
        /// Zone index (1-based).
        zone: usize,
        /// Updated EQ configuration (flattened into the JSON object).
        #[serde(flatten)]
        config: crate::audio::eq::EqConfig,
    },
    /// Zone presence state changed.
    ZonePresenceChanged {
        /// Zone index (1-based).
        zone: usize,
        /// Whether presence is detected.
        presence: bool,
        /// Whether presence-triggered playback is enabled.
        enabled: bool,
        /// Whether the auto-off timer is running.
        timer_active: bool,
    },
}

/// Create a broadcast channel for notifications.
/// We broadcast a pre-serialized JSON string to all clients for efficiency.
pub type NotifySender = broadcast::Sender<std::sync::Arc<str>>;

/// Create a broadcast channel for notifications.
#[must_use]
pub fn notification_channel() -> (
    broadcast::Sender<std::sync::Arc<str>>,
    broadcast::Receiver<std::sync::Arc<str>>,
) {
    broadcast::channel(NOTIFICATION_CHANNEL_SIZE)
}

/// Helper to serialize and send a notification.
pub fn broadcast_notification(sender: &NotifySender, notification: &Notification) {
    if let Ok(json) = serde_json::to_string(notification) {
        let _ = sender.send(json.into());
    }
}

/// Build the WebSocket router.
pub fn router(state: SharedState) -> Router {
    Router::new()
        .route("/ws", get(ws_handler))
        .with_state(state)
}

async fn ws_handler(ws: WebSocketUpgrade, State(state): State<SharedState>) -> impl IntoResponse {
    if ACTIVE_CONNECTIONS.load(Ordering::Relaxed) >= MAX_WS_CONNECTIONS {
        return axum::http::StatusCode::SERVICE_UNAVAILABLE.into_response();
    }
    ws.on_upgrade(|socket| handle_socket(socket, state))
}

async fn handle_socket(mut socket: WebSocket, state: SharedState) {
    ACTIVE_CONNECTIONS.fetch_add(1, Ordering::Relaxed);
    let mut rx = state.notifications.subscribe();
    let mut ping_interval = tokio::time::interval(WS_PING_INTERVAL);
    tracing::debug!("WebSocket client connected");

    loop {
        tokio::select! {
            result = rx.recv() => {
                if let Ok(json) = result {
                    if socket.send(Message::Text(json.as_ref().into())).await.is_err() {
                        break;
                    }
                } else {
                    // Broadcast channel closed — server is shutting down
                    let _ = socket.send(Message::Close(Some(axum::extract::ws::CloseFrame {
                        code: 1001, // Going Away
                        reason: "Server shutting down".into(),
                    }))).await;
                    break;
                }
            }
            _ = ping_interval.tick() => {
                if socket.send(Message::Ping(vec![].into())).await.is_err() {
                    break;
                }
            }
            msg = socket.recv() => {
                match msg {
                    Some(Ok(Message::Close(_))) | None => break,
                    _ => {}
                }
            }
        }
    }

    ACTIVE_CONNECTIONS.fetch_sub(1, Ordering::Relaxed);
    tracing::debug!("WebSocket client disconnected");
}
