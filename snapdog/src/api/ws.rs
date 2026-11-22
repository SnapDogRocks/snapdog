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
/// How long one frame may take to be accepted. A peer that stops reading (a suspended tab,
/// a phone that left the network) fills the socket buffers; after this it is closed and
/// its slot is free again.
const WS_SEND_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
/// Close code for a server that is shutting down (RFC 6455 "Going Away").
const CLOSE_GOING_AWAY: u16 = 1001;
/// Close code for a client that fell behind the notification buffer (IANA "Try Again
/// Later"). It has missed events, so it has to reconnect and re-read the state over REST.
pub const CLOSE_TOO_SLOW: u16 = 1013;

use std::sync::atomic::{AtomicUsize, Ordering};

/// Open WebSocket connections, counted against [`MAX_WS_CONNECTIONS`].
static ACTIVE_CONNECTIONS: Slots = Slots::new(MAX_WS_CONNECTIONS);

/// A bounded number of connection slots.
struct Slots {
    active: AtomicUsize,
    max: usize,
}

impl Slots {
    const fn new(max: usize) -> Self {
        Self {
            active: AtomicUsize::new(0),
            max,
        }
    }

    /// Take a slot, or `None` when all are in use. Counting and checking is one atomic
    /// step, so concurrent upgrades cannot pass the limit together.
    fn reserve(&self) -> Option<Slot<'_>> {
        self.active
            .try_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < self.max).then_some(n + 1)
            })
            .ok()
            .map(|_| Slot(self))
    }

    #[cfg(test)]
    fn in_use(&self) -> usize {
        self.active.load(Ordering::Acquire)
    }
}

/// A taken connection slot. Dropping it gives the slot back, on every path out of a
/// connection, a panic or a failed upgrade included.
struct Slot<'a>(&'a Slots);

impl Drop for Slot<'_> {
    fn drop(&mut self) {
        self.0.active.fetch_sub(1, Ordering::AcqRel);
    }
}

use axum::extract::State;

use axum::extract::ws::{Message, WebSocketUpgrade};

use axum::response::IntoResponse;

use axum::routing::get;

use serde::Serialize;

use tokio::sync::broadcast;

use crate::api::SharedState;

/// Notification broadcast to all connected WebSocket clients.
#[derive(Debug, Clone, Serialize, serde::Deserialize, utoipa::ToSchema)]
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
        /// Sample rate of the track in Hz.
        sample_rate: Option<u32>,
        /// MIME content type.
        content_type: Option<String>,
        // Playlist position
        /// Current track index in playlist.
        track_index: Option<usize>,
        /// Total tracks in playlist.
        track_count: Option<usize>,
        // Playlist navigation
        /// Current unified playlist index (0-based).
        #[serde(default)]
        playlist_index: Option<usize>,
        /// Stable reference of the active playlist: `snapdog:radio` or `subsonic:<id>`.
        #[serde(default)]
        playlist_id: Option<String>,
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
        /// Manual latency correction in milliseconds, as `GET /clients/{n}/latency` reports it.
        /// Absent in the events of servers before this field, which read as 0.
        #[serde(default)]
        latency_ms: i32,
        /// Display name of the client, as `GET /clients/{n}/name` reports it. Absent in the
        /// events of servers before this field, which read as an empty string.
        #[serde(default)]
        name: String,
    },
    /// Client equalizer configuration changed.
    ClientEqChanged {
        /// Client index (1-based).
        client: usize,
        /// Updated EQ configuration (flattened into the JSON object).
        #[serde(flatten)]
        config: crate::audio::eq::EqConfig,
    },
    /// Speaker correction of a client changed.
    ClientSpeakerChanged {
        /// Client index (1-based).
        client: usize,
        /// Updated speaker correction (flattened into the JSON object).
        #[serde(flatten)]
        config: crate::audio::eq::EqConfig,
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
        /// Auto-off delay in seconds, as `GET /zones/{n}/presence/timeout` reports it. It is sent
        /// when the delay changes too (API, MQTT, KNX). Absent in the events of servers before
        /// this field.
        #[serde(default)]
        auto_off_delay: Option<u16>,
    },
    /// The list of playlists, an index or a name may have changed. Read `GET /media/playlists`
    /// again.
    PlaylistsChanged {
        /// Number of playlists now (radio list included).
        total: usize,
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

/// Receive the next notification, skipping over a lag instead of ending.
///
/// When a burst outruns a listener, the broadcast channel drops the oldest events and reports
/// `Lagged` once. A listener that stops there misses everything after it for good. The next
/// event still carries the full state, so this logs the gap and keeps going. `None` means the
/// channel is closed.
pub async fn recv_skipping_lag(
    rx: &mut broadcast::Receiver<std::sync::Arc<str>>,
    listener: &'static str,
) -> Option<std::sync::Arc<str>> {
    loop {
        match rx.recv().await {
            Ok(message) => return Some(message),
            Err(broadcast::error::RecvError::Lagged(skipped)) => {
                tracing::warn!(skipped, listener, "Notification listener lagged behind");
            }
            Err(broadcast::error::RecvError::Closed) => return None,
        }
    }
}

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
    // The slot is taken before the upgrade and moves into the connection; if the upgrade
    // fails, the callback and with it the slot are dropped.
    let Some(slot) = ACTIVE_CONNECTIONS.reserve() else {
        return axum::http::StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    ws.on_upgrade(move |socket| async move {
        let _slot = slot;
        let rx = state.notifications.subscribe();
        tracing::debug!("WebSocket client connected");
        serve_connection(socket, rx, WS_PING_INTERVAL, WS_SEND_TIMEOUT).await;
        tracing::debug!("WebSocket client disconnected");
    })
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

/// Send one frame, giving up after `timeout`. `false` means the connection is done: the
/// peer is gone, or it stopped reading and the frame could not be handed over in time.
async fn send_frame<S>(socket: &mut S, message: Message, timeout: std::time::Duration) -> bool
where
    S: futures_util::Sink<Message> + Unpin,
{
    use futures_util::SinkExt;
    let Ok(result) = tokio::time::timeout(timeout, socket.send(message)).await else {
        tracing::warn!(?timeout, "WebSocket client stopped reading, closing it");
        return false;
    };
    result.is_ok()
}

/// Run one connection: forward notifications, ping, and end when the peer closes, stops
/// reading or falls behind. Generic over the socket so a test can use a peer that never
/// reads.
async fn serve_connection<S, E>(
    mut socket: S,
    mut rx: broadcast::Receiver<std::sync::Arc<str>>,
    ping_every: std::time::Duration,
    send_timeout: std::time::Duration,
) where
    S: futures_util::Sink<Message> + futures_util::Stream<Item = Result<Message, E>> + Unpin,
{
    use futures_util::StreamExt;
    let mut ping_interval = tokio::time::interval(ping_every);
    loop {
        tokio::select! {
            result = rx.recv() => {
                match delivery(result) {
                    Delivery::Send(json) => {
                        if !send_frame(&mut socket, Message::Text(json.as_ref().into()), send_timeout).await {
                            break;
                        }
                    }
                    Delivery::Close(frame) => {
                        let _ = send_frame(&mut socket, Message::Close(Some(frame)), send_timeout).await;
                        break;
                    }
                }
            }
            _ = ping_interval.tick() => {
                if !send_frame(&mut socket, Message::Ping(vec![].into()), send_timeout).await {
                    break;
                }
            }
            msg = socket.next() => {
                match msg {
                    Some(Ok(Message::Close(_))) | None => break,
                    _ => {}
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::pin::Pin;
    use std::task::{Context, Poll};

    /// A peer that never reads and never writes: every send stays pending.
    struct StalledPeer;

    impl futures_util::Sink<Message> for StalledPeer {
        type Error = axum::Error;
        fn poll_ready(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
            Poll::Pending
        }
        fn start_send(self: Pin<&mut Self>, _: Message) -> Result<(), Self::Error> {
            Ok(())
        }
        fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
            Poll::Pending
        }
        fn poll_close(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
            Poll::Pending
        }
    }

    impl futures_util::Stream for StalledPeer {
        type Item = Result<Message, axum::Error>;
        fn poll_next(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Option<Self::Item>> {
            Poll::Pending
        }
    }

    /// A peer whose connection fails on the first send.
    struct BrokenPeer;

    impl futures_util::Sink<Message> for BrokenPeer {
        type Error = axum::Error;
        fn poll_ready(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
            Poll::Ready(Err(axum::Error::new("connection reset")))
        }
        fn start_send(self: Pin<&mut Self>, _: Message) -> Result<(), Self::Error> {
            Ok(())
        }
        fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }
        fn poll_close(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }
    }

    impl futures_util::Stream for BrokenPeer {
        type Item = Result<Message, axum::Error>;
        fn poll_next(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Option<Self::Item>> {
            Poll::Pending
        }
    }

    #[tokio::test]
    async fn a_peer_that_stops_reading_is_closed_and_frees_its_slot() {
        let slots = Slots::new(4);
        let (tx, rx) = broadcast::channel::<std::sync::Arc<str>>(16);
        let slot = slots.reserve().unwrap();
        let events = tokio::spawn(async move {
            for i in 0..1000 {
                let _ = tx.send(format!("event {i}").into());
                tokio::time::sleep(std::time::Duration::from_millis(1)).await;
            }
        });
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            serve_connection(
                StalledPeer,
                rx,
                std::time::Duration::from_secs(3600),
                std::time::Duration::from_millis(50),
            ),
        )
        .await
        .expect("the send timeout ends the connection");
        drop(slot);
        assert_eq!(slots.in_use(), 0);
        events.abort();
    }

    #[tokio::test]
    async fn a_connection_that_fails_frees_its_slot() {
        let slots = Slots::new(4);
        let (_tx, rx) = broadcast::channel::<std::sync::Arc<str>>(16);
        {
            let _slot = slots.reserve().unwrap();
            serve_connection(
                BrokenPeer,
                rx,
                std::time::Duration::from_secs(3600),
                WS_SEND_TIMEOUT,
            )
            .await;
        }
        assert_eq!(slots.in_use(), 0);
    }

    #[test]
    fn concurrent_upgrades_never_exceed_the_limit() {
        let slots = std::sync::Arc::new(Slots::new(MAX_WS_CONNECTIONS));
        let start = std::sync::Arc::new(std::sync::Barrier::new(200));
        // All threads must exist before any is joined, or the barrier never opens.
        #[allow(clippy::needless_collect, reason = "spawn every thread before joining")]
        let threads: Vec<_> = (0..200)
            .map(|_| {
                let slots = slots.clone();
                let start = start.clone();
                std::thread::spawn(move || {
                    start.wait();
                    let taken = slots.reserve().map(std::mem::forget).is_some();
                    usize::from(taken)
                })
            })
            .collect();
        let taken: usize = threads.into_iter().map(|t| t.join().unwrap()).sum();
        assert_eq!(taken, MAX_WS_CONNECTIONS);
        assert_eq!(slots.in_use(), MAX_WS_CONNECTIONS);
    }

    #[test]
    fn slots_are_given_back_and_can_be_taken_again() {
        let slots = Slots::new(2);
        let a = slots.reserve().unwrap();
        let b = slots.reserve().unwrap();
        assert!(slots.reserve().is_none(), "the limit holds");
        assert_eq!(slots.in_use(), 2, "a refused reservation does not count");
        drop(a);
        let c = slots.reserve().expect("a freed slot can be taken");
        drop((b, c));
        assert_eq!(slots.in_use(), 0);
    }

    #[tokio::test]
    async fn a_listener_keeps_receiving_after_it_lagged_and_stops_when_closed() {
        let (tx, mut rx) = broadcast::channel::<std::sync::Arc<str>>(4);
        for i in 0..10 {
            tx.send(format!("event {i}").into()).unwrap();
        }
        // The first six are gone; the listener resumes with the oldest one kept.
        assert_eq!(
            recv_skipping_lag(&mut rx, "test").await.as_deref(),
            Some("event 6")
        );
        tx.send("after the lag".into()).unwrap();
        for expected in ["event 7", "event 8", "event 9", "after the lag"] {
            assert_eq!(
                recv_skipping_lag(&mut rx, "test").await.as_deref(),
                Some(expected)
            );
        }
        drop(tx);
        assert!(recv_skipping_lag(&mut rx, "test").await.is_none());
    }

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
