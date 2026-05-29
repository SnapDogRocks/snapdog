// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! TCP connection to Snapcast JSON-RPC server.
//!
//! Spawns a background task that:
//! - Reads newline-delimited JSON from the TCP stream
//! - Routes Response messages to pending request oneshots (matched by id)
//! - Routes Notification messages to a broadcast channel

use std::collections::HashMap;
use std::net::SocketAddr;

use anyhow::{Context, Result};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::sync::{broadcast, mpsc, oneshot};

use super::protocol::{Notification, RawMessage, Request};

/// Maximum connection attempts before giving up on initial connect.
const MAX_CONNECT_ATTEMPTS: u32 = 10;
/// Delay between initial connection attempts.
const CONNECT_RETRY_DELAY: std::time::Duration = std::time::Duration::from_millis(500);
/// Maximum reconnection attempts before giving up.
const MAX_RECONNECT_ATTEMPTS: u64 = 10;
/// Maximum reconnect delay in seconds (capped exponential backoff).
const MAX_RECONNECT_DELAY_SECS: u64 = 5;

/// A pending request awaiting its response.
type PendingTx = oneshot::Sender<Result<Value, super::protocol::RpcError>>;

/// Command sent from the client API to the connection task.
enum ConnCmd {
    Send { request: Request, reply: PendingTx },
}

/// Handle to the Snapcast JSON-RPC connection.
///
/// All methods are `&self` — the connection task runs in the background.
/// Cloning is cheap (just clones the channel senders).
#[derive(Clone)]
pub struct Connection {
    cmd_tx: mpsc::Sender<ConnCmd>,
    notification_tx: broadcast::Sender<Notification>,
}

impl Connection {
    /// Connect to the Snapcast JSON-RPC server and spawn the background task.
    /// # Errors
    /// Returns an error if the request fails.
    pub async fn connect(addr: SocketAddr) -> Result<Self> {
        let stream = {
            let mut attempts = 0;
            loop {
                match TcpStream::connect(addr).await {
                    Ok(s) => break s,
                    Err(e) => {
                        attempts += 1;
                        if attempts >= MAX_CONNECT_ATTEMPTS {
                            return Err(e).context("Failed to connect to Snapcast JSON-RPC");
                        }
                        tracing::debug!(attempt = attempts, "Snapcast not ready, retrying...");
                        tokio::time::sleep(CONNECT_RETRY_DELAY).await;
                    }
                }
            }
        };
        tracing::info!(%addr, "Snapcast JSON-RPC connected");

        let (cmd_tx, cmd_rx) = mpsc::channel::<ConnCmd>(64);
        let (notification_tx, _) = broadcast::channel::<Notification>(256);

        let ntx = notification_tx.clone();
        tokio::spawn(connection_task(stream, cmd_rx, ntx, addr));

        Ok(Self {
            cmd_tx,
            notification_tx,
        })
    }

    /// Send a JSON-RPC request and await the response.
    /// # Errors
    /// Returns an error if the request fails.
    pub async fn request(&self, method: &'static str, params: Value) -> Result<Value> {
        let req = Request::new(method, params);
        let (reply_tx, reply_rx) = oneshot::channel();

        self.cmd_tx
            .send(ConnCmd::Send {
                request: req,
                reply: reply_tx,
            })
            .await
            .context("Connection task gone")?;

        reply_rx
            .await
            .context("Connection task dropped reply")?
            .map_err(|e| anyhow::anyhow!("{e}"))
    }

    /// Subscribe to Snapcast server notifications.
    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<Notification> {
        self.notification_tx.subscribe()
    }
}

/// Background task: reads from TCP, routes messages, writes requests.
async fn connection_task(
    stream: TcpStream,
    mut cmd_rx: mpsc::Receiver<ConnCmd>,
    notification_tx: broadcast::Sender<Notification>,
    addr: SocketAddr,
) {
    let (reader, mut writer) = stream.into_split();
    let mut lines = BufReader::new(reader).lines();
    let mut pending: HashMap<uuid::Uuid, PendingTx> = HashMap::new();

    loop {
        tokio::select! {
            // Incoming message from Snapcast server
            line = lines.next_line() => {
                match line {
                    Ok(Some(line)) => {
                        handle_incoming_line(&line, &mut pending, &notification_tx);
                    }
                    Ok(None) => {
                        tracing::warn!("Snapcast connection closed");
                        reject_all_pending(&mut pending);
                        // Try to reconnect
                        if let Some((new_lines, new_writer)) = reconnect(addr).await {
                            lines = new_lines;
                            writer = new_writer;
                            tracing::info!("Snapcast reconnected");
                        } else {
                            tracing::error!("Snapcast reconnect failed, giving up");
                            break;
                        }
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "Snapcast read error");
                        reject_all_pending(&mut pending);
                        if let Some((new_lines, new_writer)) = reconnect(addr).await {
                            lines = new_lines;
                            writer = new_writer;
                            tracing::info!("Snapcast reconnected");
                        } else {
                            break;
                        }
                    }
                }
            }
            // Outgoing request from client API
            Some(cmd) = cmd_rx.recv() => {
                match cmd {
                    ConnCmd::Send { request, reply } => {
                        let id = request.id;
                        match serde_json::to_string(&request) {
                            Ok(mut json) => {
                                json.push('\n');
                                if let Err(e) = writer.write_all(json.as_bytes()).await {
                                    tracing::warn!(error = %e, "Snapcast write error");
                                    let _ = reply.send(Err(super::protocol::RpcError {
                                        code: -1,
                                        message: format!("Write error: {e}"),
                                        data: None,
                                    }));
                                } else {
                                    pending.insert(id, reply);
                                }
                            }
                            Err(e) => {
                                tracing::warn!(error = %e, "Failed to serialize request");
                            }
                        }
                    }
                }
            }
        }
    }
}

fn handle_incoming_line(
    line: &str,
    pending: &mut HashMap<uuid::Uuid, PendingTx>,
    notification_tx: &broadcast::Sender<Notification>,
) {
    match serde_json::from_str::<RawMessage>(line) {
        Ok(RawMessage::Response { id, result, error }) => {
            if let Some(reply) = pending.remove(&id) {
                if let Some(err) = error {
                    let _ = reply.send(Err(err));
                } else {
                    let _ = reply.send(Ok(result.unwrap_or(Value::Null)));
                }
            } else {
                tracing::debug!(%id, "Response for unknown request");
            }
        }
        Ok(RawMessage::Notification { method, params }) => {
            let notification = Notification::parse(&method, params);
            let _ = notification_tx.send(notification);
        }
        Err(e) => {
            tracing::debug!(error = %e, line = %line, "Failed to parse Snapcast message");
        }
    }
}

fn reject_all_pending(pending: &mut HashMap<uuid::Uuid, PendingTx>) {
    for (_, reply) in pending.drain() {
        let _ = reply.send(Err(super::protocol::RpcError {
            code: -1,
            message: "Connection lost".to_string(),
            data: None,
        }));
    }
}

async fn reconnect(
    addr: SocketAddr,
) -> Option<(
    tokio::io::Lines<BufReader<tokio::net::tcp::OwnedReadHalf>>,
    tokio::net::tcp::OwnedWriteHalf,
)> {
    for attempt in 1..=MAX_RECONNECT_ATTEMPTS {
        let delay = std::time::Duration::from_secs(attempt.min(MAX_RECONNECT_DELAY_SECS));
        tracing::info!(
            attempt,
            delay_secs = delay.as_secs(),
            "Reconnecting to Snapcast..."
        );
        tokio::time::sleep(delay).await;

        match TcpStream::connect(addr).await {
            Ok(stream) => {
                let (reader, writer) = stream.into_split();
                let lines = BufReader::new(reader).lines();
                return Some((lines, writer));
            }
            Err(e) => {
                tracing::warn!(attempt, error = %e, "Reconnect attempt failed");
            }
        }
    }
    None
}
