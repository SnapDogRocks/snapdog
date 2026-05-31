// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! Snapcast backend trait — abstraction over embedded server vs external process.

use std::future::Future;
use std::pin::Pin;

use anyhow::Result;

use crate::player::SnapcastCmd;

/// Client identification from the Snapcast Hello handshake.
#[derive(Debug, Clone, Default)]
pub struct ClientHello {
    /// Client-reported name (e.g. "`SnapDog`").
    pub client_name: String,
    /// MAC address.
    pub mac: String,
    /// Hostname.
    pub host_name: String,
    /// Client version string.
    pub version: String,
}

/// Events emitted by the Snapcast backend to the consumer.
#[derive(Debug)]
pub enum SnapcastEvent {
    /// A client connected.
    ClientConnected {
        /// Snapcast client ID.
        id: String,
        /// The client's Hello message.
        hello: ClientHello,
    },
    /// A client disconnected.
    ClientDisconnected {
        /// Snapcast client ID.
        id: String,
    },
    /// A client's volume changed.
    ClientVolumeChanged {
        /// Snapcast client ID.
        id: String,
        /// Volume (0–100).
        volume: i32,
        /// Mute state.
        muted: bool,
    },
    /// A client's latency changed.
    ClientLatencyChanged {
        /// Snapcast client ID.
        id: String,
        /// Latency in ms.
        latency: i32,
    },
    /// A client's name changed.
    ClientNameChanged {
        /// Snapcast client ID.
        id: String,
        /// New name.
        name: String,
    },
    /// Server state changed (groups reorganized, etc.)
    ServerUpdated,
    /// A custom protocol message received from a client.
    CustomMessage {
        /// Snapcast client ID that sent the message.
        client_id: String,
        /// Custom message type ID.
        type_id: u16,
        /// Message payload bytes.
        payload: Vec<u8>,
    },
}

/// Boxed future type for trait object safety.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Abstraction over Snapcast server implementations.
///
/// Two implementations:
/// - `EmbeddedBackend` (feature `snapcast-embedded`): in-process snapcast-server crate
/// - `ProcessBackend` (feature `snapcast-process`): external snapserver binary + JSON-RPC
pub trait SnapcastBackend: Send + Sync {
    /// Send interleaved f32 audio samples to a zone's stream.
    fn send_audio(
        &self,
        zone_index: usize,
        samples: &[f32],
        sample_rate: u32,
        channels: u16,
    ) -> BoxFuture<'_, Result<()>>;

    /// Execute a Snapcast command (volume, mute, group assignment, etc.)
    fn execute(&self, cmd: SnapcastCmd) -> BoxFuture<'_, Result<()>>;

    /// Graceful shutdown.
    fn stop(&self) -> BoxFuture<'_, Result<()>>;

    /// Get the server status as JSON (for API compatibility).
    fn get_status(&self) -> BoxFuture<'_, Result<serde_json::Value>>;

    /// Delete a client from the server.
    fn delete_client(&self, id: &str) -> BoxFuture<'_, Result<()>>;
}
