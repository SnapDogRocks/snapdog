// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! JSON-RPC 2.0 message types and Snapcast method/notification enums.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::types;

// ── Generic JSON-RPC 2.0 ─────────────────────────────────────

/// A JSON-RPC 2.0 request sent to the Snapcast server.
#[derive(Debug, Clone, Serialize)]
pub struct Request {
    /// Protocol version (always `"2.0"`).
    pub jsonrpc: &'static str,
    /// Unique request identifier for correlating responses.
    pub id: uuid::Uuid,
    /// RPC method name (e.g. `"Client.SetVolume"`).
    pub method: &'static str,
    /// Method parameters as a JSON value.
    pub params: Value,
}

impl Request {
    /// Create a new JSON-RPC 2.0 request with a random UUID.
    #[must_use]
    pub fn new(method: &'static str, params: Value) -> Self {
        Self {
            jsonrpc: "2.0",
            id: uuid::Uuid::new_v4(),
            method,
            params,
        }
    }
}

/// Raw JSON-RPC message from the server (before routing).
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum RawMessage {
    /// A response to a previously sent request.
    Response {
        /// Request identifier this response correlates to.
        id: uuid::Uuid,
        /// Successful result payload, if any.
        result: Option<Value>,
        /// Error payload, if the request failed.
        error: Option<RpcError>,
    },
    /// An unsolicited server notification.
    Notification {
        /// Notification method name (e.g. `"Client.OnConnect"`).
        method: String,
        /// Notification parameters.
        params: Value,
    },
}

/// JSON-RPC 2.0 error object returned by the Snapcast server.
#[derive(Debug, Clone, Deserialize)]
pub struct RpcError {
    /// Numeric error code.
    pub code: i64,
    /// Human-readable error description.
    pub message: String,
    /// Optional additional error data.
    pub data: Option<Value>,
}

impl std::fmt::Display for RpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "JSON-RPC error {}: {}", self.code, self.message)
    }
}

impl std::error::Error for RpcError {}

// ── Snapcast Notifications ───────────────────────────────────

/// Typed Snapcast server notification.
#[derive(Debug, Clone)]
pub enum Notification {
    /// Client connected to Snapcast server.
    ClientOnConnect {
        /// Snapcast client identifier.
        id: String,
        /// Full client state at connection time.
        client: types::Client,
    },
    /// Client disconnected from Snapcast server.
    ClientOnDisconnect {
        /// Snapcast client identifier.
        id: String,
    },
    /// Client volume changed.
    ClientOnVolumeChanged {
        /// Snapcast client identifier.
        id: String,
        /// New volume state.
        volume: types::Volume,
    },
    /// Client playback latency changed.
    ClientOnLatencyChanged {
        /// Snapcast client identifier.
        id: String,
        /// New latency in milliseconds.
        latency: usize,
    },
    /// Client display name changed.
    ClientOnNameChanged {
        /// Snapcast client identifier.
        id: String,
        /// New client name.
        name: String,
    },
    /// Group mute state changed.
    GroupOnMute {
        /// Snapcast group identifier.
        id: String,
        /// Whether the group is now muted.
        mute: bool,
    },
    /// Group switched to a different audio stream.
    GroupOnStreamChanged {
        /// Snapcast group identifier.
        id: String,
        /// New stream identifier.
        stream_id: String,
    },
    /// Group display name changed.
    GroupOnNameChanged {
        /// Snapcast group identifier.
        id: String,
        /// New group name.
        name: String,
    },
    /// Full server state update (groups, clients, streams).
    ServerOnUpdate {
        /// Complete server state snapshot.
        server: types::Server,
    },
    /// Audio stream metadata or status updated.
    StreamOnUpdate {
        /// Snapcast stream identifier.
        id: String,
        /// Updated stream state.
        stream: types::Stream,
    },
    /// Audio stream properties changed (e.g. now-playing metadata).
    StreamOnProperties {
        /// Snapcast stream identifier.
        id: String,
        /// Updated stream properties.
        properties: types::StreamProperties,
    },
    /// Unrecognized notification method.
    Unknown {
        /// Raw method name from the server.
        method: String,
        /// Raw parameters.
        params: Value,
    },
}

impl Notification {
    /// Parse a raw JSON-RPC notification into a typed Snapcast notification.
    #[must_use]
    #[allow(clippy::too_many_lines)]
    pub fn parse(method: &str, params: Value) -> Self {
        match method {
            "Client.OnConnect" => {
                #[derive(Deserialize)]
                struct P {
                    id: String,
                    client: types::Client,
                }
                match serde_json::from_value::<P>(params.clone()) {
                    Ok(p) => Self::ClientOnConnect {
                        id: p.id,
                        client: p.client,
                    },
                    Err(_) => Self::Unknown {
                        method: method.to_string(),
                        params,
                    },
                }
            }
            "Client.OnDisconnect" => {
                #[derive(Deserialize)]
                struct P {
                    id: String,
                }
                match serde_json::from_value::<P>(params.clone()) {
                    Ok(p) => Self::ClientOnDisconnect { id: p.id },
                    Err(_) => Self::Unknown {
                        method: method.to_string(),
                        params,
                    },
                }
            }
            "Client.OnVolumeChanged" => {
                #[derive(Deserialize)]
                struct P {
                    id: String,
                    volume: types::Volume,
                }
                match serde_json::from_value::<P>(params.clone()) {
                    Ok(p) => Self::ClientOnVolumeChanged {
                        id: p.id,
                        volume: p.volume,
                    },
                    Err(_) => Self::Unknown {
                        method: method.to_string(),
                        params,
                    },
                }
            }
            "Client.OnLatencyChanged" => {
                #[derive(Deserialize)]
                struct P {
                    id: String,
                    latency: usize,
                }
                match serde_json::from_value::<P>(params.clone()) {
                    Ok(p) => Self::ClientOnLatencyChanged {
                        id: p.id,
                        latency: p.latency,
                    },
                    Err(_) => Self::Unknown {
                        method: method.to_string(),
                        params,
                    },
                }
            }
            "Client.OnNameChanged" => {
                #[derive(Deserialize)]
                struct P {
                    id: String,
                    name: String,
                }
                match serde_json::from_value::<P>(params.clone()) {
                    Ok(p) => Self::ClientOnNameChanged {
                        id: p.id,
                        name: p.name,
                    },
                    Err(_) => Self::Unknown {
                        method: method.to_string(),
                        params,
                    },
                }
            }
            "Group.OnMute" => {
                #[derive(Deserialize)]
                struct P {
                    id: String,
                    mute: bool,
                }
                match serde_json::from_value::<P>(params.clone()) {
                    Ok(p) => Self::GroupOnMute {
                        id: p.id,
                        mute: p.mute,
                    },
                    Err(_) => Self::Unknown {
                        method: method.to_string(),
                        params,
                    },
                }
            }
            "Group.OnStreamChanged" => {
                #[derive(Deserialize)]
                struct P {
                    id: String,
                    stream_id: String,
                }
                match serde_json::from_value::<P>(params.clone()) {
                    Ok(p) => Self::GroupOnStreamChanged {
                        id: p.id,
                        stream_id: p.stream_id,
                    },
                    Err(_) => Self::Unknown {
                        method: method.to_string(),
                        params,
                    },
                }
            }
            "Group.OnNameChanged" => {
                #[derive(Deserialize)]
                struct P {
                    id: String,
                    name: String,
                }
                match serde_json::from_value::<P>(params.clone()) {
                    Ok(p) => Self::GroupOnNameChanged {
                        id: p.id,
                        name: p.name,
                    },
                    Err(_) => Self::Unknown {
                        method: method.to_string(),
                        params,
                    },
                }
            }
            "Server.OnUpdate" => {
                #[derive(Deserialize)]
                struct P {
                    server: types::Server,
                }
                match serde_json::from_value::<P>(params.clone()) {
                    Ok(p) => Self::ServerOnUpdate { server: p.server },
                    Err(_) => Self::Unknown {
                        method: method.to_string(),
                        params,
                    },
                }
            }
            "Stream.OnUpdate" => {
                #[derive(Deserialize)]
                struct P {
                    id: String,
                    stream: types::Stream,
                }
                match serde_json::from_value::<P>(params.clone()) {
                    Ok(p) => Self::StreamOnUpdate {
                        id: p.id,
                        stream: p.stream,
                    },
                    Err(_) => Self::Unknown {
                        method: method.to_string(),
                        params,
                    },
                }
            }
            "Stream.OnProperties" => {
                #[derive(Deserialize)]
                struct P {
                    id: String,
                    properties: types::StreamProperties,
                }
                match serde_json::from_value::<P>(params.clone()) {
                    Ok(p) => Self::StreamOnProperties {
                        id: p.id,
                        properties: p.properties,
                    },
                    Err(_) => Self::Unknown {
                        method: method.to_string(),
                        params,
                    },
                }
            }
            _ => Self::Unknown {
                method: method.to_string(),
                params,
            },
        }
    }
}
