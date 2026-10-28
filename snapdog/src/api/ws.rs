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
/// Close code for a server that is shutting down (RFC 6455 "Going Away").
const CLOSE_GOING_AWAY: u16 = 1001;
/// Close code for a client that fell behind the notification buffer (IANA "Try Again
/// Later"). It has missed events, so it has to reconnect and re-read the state over REST.
pub const CLOSE_TOO_SLOW: u16 = 1013;

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
    /// Playback error occurred.
    PlaybackError {
        /// Zone index (1-based).
        zone: usize,
        /// Primary user-facing error message.
        message: String,
        /// Technical details (e.g. source error description).
        details: Option<String>,
        /// Whether the error is recoverable.
        recoverable: bool,
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

/// What to do with one result of the broadcast receiver.
enum Delivery {
    /// Send this notification to the client.
    Send(std::sync::Arc<str>),
    /// End the connection with this close frame.
    Close(axum::extract::ws::CloseFrame),
}

/// Map a broadcast receive result to an action. A closed channel means the server is
/// shutting down. A lagged receiver means this client could not keep up and the
/// channel dropped events for it: telling it "server shutting down" would be wrong
/// (the `WebUI` shows a banner for 1001), so it gets its own code and reason.
fn delivery(result: Result<std::sync::Arc<str>, broadcast::error::RecvError>) -> Delivery {
    use axum::extract::ws::CloseFrame;
    match result {
        Ok(json) => Delivery::Send(json),
        Err(broadcast::error::RecvError::Lagged(skipped)) => {
            tracing::warn!(skipped, "WebSocket client too slow, closing it");
            Delivery::Close(CloseFrame {
                code: CLOSE_TOO_SLOW,
                reason: "Client too slow, reconnect and reload the state".into(),
            })
        }
        Err(broadcast::error::RecvError::Closed) => Delivery::Close(CloseFrame {
            code: CLOSE_GOING_AWAY,
            reason: "Server shutting down".into(),
        }),
    }
}

async fn handle_socket(mut socket: WebSocket, state: SharedState) {
    ACTIVE_CONNECTIONS.fetch_add(1, Ordering::Relaxed);
    let mut rx = state.notifications.subscribe();
    let mut ping_interval = tokio::time::interval(WS_PING_INTERVAL);
    tracing::debug!("WebSocket client connected");

    loop {
        tokio::select! {
            result = rx.recv() => {
                match delivery(result) {
                    Delivery::Send(json) => {
                        if socket.send(Message::Text(json.as_ref().into())).await.is_err() {
                            break;
                        }
                    }
                    Delivery::Close(frame) => {
                        let _ = socket.send(Message::Close(Some(frame))).await;
                        break;
                    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_lagging_client_is_not_told_the_server_is_shutting_down() {
        let Delivery::Close(frame) = delivery(Err(broadcast::error::RecvError::Lagged(12))) else {
            panic!("a lagged receiver must be closed");
        };
        assert_eq!(frame.code, CLOSE_TOO_SLOW);
        assert_ne!(frame.code, CLOSE_GOING_AWAY);
        assert!(frame.reason.contains("too slow"));
    }

    #[test]
    fn a_closed_channel_is_a_server_shutdown() {
        let Delivery::Close(frame) = delivery(Err(broadcast::error::RecvError::Closed)) else {
            panic!("a closed channel must close the socket");
        };
        assert_eq!(frame.code, CLOSE_GOING_AWAY);
        assert!(frame.reason.contains("shutting down"));
    }

    #[test]
    fn a_message_is_forwarded() {
        let Delivery::Send(json) = delivery(Ok("{}".into())) else {
            panic!("a message must be forwarded");
        };
        assert_eq!(&*json, "{}");
    }
}
