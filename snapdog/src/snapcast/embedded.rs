// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! Embedded Snapcast backend — in-process server via snapcast-server crate.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use tokio::sync::mpsc;

use snapcast_server::{ServerCommand, ServerConfig, ServerEvent, SnapServer};

use super::backend::{BoxFuture, SnapcastBackend, SnapcastEvent};
use crate::config::AppConfig;
use crate::player::{ClientAction, GroupAction, SnapcastCmd};
use crate::state;

/// Default audio buffer size in milliseconds for the embedded server.
const DEFAULT_BUFFER_MS: u32 = 1000;

/// How long a volume push waits for its echo. The server echoes at once, so this only
/// bounds the leftovers of echoes it dropped.
const ECHO_WINDOW: Duration = Duration::from_secs(5);

/// Volume pushes per client that wait for their echo at most.
const MAX_PENDING_ECHOES: usize = 64;

/// Tells `SnapDog`'s own volume pushes apart from volumes a client reports itself.
///
/// snapcast-server announces both with the same `ClientVolumeChanged` event: once after
/// `SetClientVolume`, and once when a client sends `ClientInfo`. The echo of a push carries
/// exactly the pushed values and arrives in order, so each push is remembered until its
/// echo shows up. An event that matches no push came from the client.
#[derive(Default)]
struct EchoFilter {
    pending: HashMap<String, VecDeque<(u16, bool, Instant)>>,
}

impl EchoFilter {
    /// Remember a push to `client_id`.
    fn expect(&mut self, client_id: &str, volume: u16, muted: bool, now: Instant) {
        let queue = self.pending.entry(client_id.to_owned()).or_default();
        queue.retain(|&(_, _, at)| now.duration_since(at) < ECHO_WINDOW);
        if queue.len() >= MAX_PENDING_ECHOES {
            queue.pop_front();
        }
        queue.push_back((volume, muted, now));
    }

    /// Whether the event is the echo of a push. Consumes that push and any older ones,
    /// whose echoes the server dropped.
    fn is_echo(&mut self, client_id: &str, volume: u16, muted: bool, now: Instant) -> bool {
        let Some(queue) = self.pending.get_mut(client_id) else {
            return false;
        };
        queue.retain(|&(_, _, at)| now.duration_since(at) < ECHO_WINDOW);
        let Some(pos) = queue
            .iter()
            .position(|&(v, m, _)| v == volume && m == muted)
        else {
            return false;
        };
        queue.drain(..=pos);
        true
    }
}

type SharedEchoFilter = Arc<Mutex<EchoFilter>>;

/// Embedded Snapcast server backend.
pub struct EmbeddedBackend {
    cmd_tx: mpsc::Sender<ServerCommand>,
    audio_txs: HashMap<usize, tokio::sync::Mutex<snapcast_server::F32AudioSender>>,
    store: state::SharedState,
    volume_modes: HashMap<usize, crate::config::GroupVolumeMode>,
    echoes: SharedEchoFilter,
    streaming_addr: std::net::SocketAddr,
}

/// Codec name and encryption key for the embedded server: `f32lz4e` is `f32lz4` plus
/// encryption with the configured key, or with the built-in public default key. An
/// empty key counts as no key (the config layer warns about it), so it never encrypts
/// with an empty secret.
fn codec_and_key(snapcast: &crate::config::SnapcastConfig) -> (String, Option<String>) {
    let key = snapcast
        .encryption_psk
        .as_ref()
        .filter(|s| !s.is_empty())
        .map(|s| s.as_str().to_string());
    if snapcast.codec == crate::config::AudioCodec::F32lz4e {
        let key = key.unwrap_or_else(|| snapcast_server::DEFAULT_ENCRYPTION_PSK.into());
        ("f32lz4".into(), Some(key))
    } else {
        (snapcast.codec.as_str().to_string(), key)
    }
}

/// Bind the audio streaming port on `[snapcast].streaming_bind` (all IPv4 interfaces
/// when unset). Synchronous (std bind, then non-blocking) so `start` stays sync.
fn bind_streaming_listener(
    snapcast: &crate::config::SnapcastConfig,
) -> Result<std::net::TcpListener> {
    let addr = snapcast.streaming_socket_addr();
    let listener = std::net::TcpListener::bind(addr)
        .map_err(|e| anyhow::anyhow!(streaming_bind_message(addr, &e)))?;
    listener
        .set_nonblocking(true)
        .context("set embedded Snapcast listener non-blocking")?;
    Ok(listener)
}

/// What a failed bind of the streaming port means, by kind of error.
///
/// A port that is taken is a second instance, an address that is not ours is a wrong
/// `[snapcast].streaming_bind`, and anything else is reported as it is.
#[must_use]
pub fn streaming_bind_message(addr: std::net::SocketAddr, error: &std::io::Error) -> String {
    match error.kind() {
        std::io::ErrorKind::AddrInUse => format!(
            "Snapcast streaming port {addr} is already in use - is snapdog already running? ({error})"
        ),
        std::io::ErrorKind::AddrNotAvailable => format!(
            "Snapcast streaming port {addr} cannot be bound: {} is not an address of this host. \
             Check [snapcast].streaming_bind ({error})",
            addr.ip()
        ),
        std::io::ErrorKind::PermissionDenied => format!(
            "Snapcast streaming port {addr} cannot be bound: permission denied. Ports below 1024 \
             need extra privileges ({error})"
        ),
        _ => format!("Snapcast streaming port {addr} cannot be bound: {error}"),
    }
}

/// Check that the streaming port can be bound, then release it. `main` calls this
/// before starting anything, so a port clash fails fast with a clear message; the
/// server binds the port itself a moment later.
///
/// # Errors
///
/// Returns an error if the address cannot be bound, for example because the port is in use.
pub fn check_streaming_port(snapcast: &crate::config::SnapcastConfig) -> Result<()> {
    bind_streaming_listener(snapcast).map(drop)
}

/// Event receiver from the embedded server.
pub struct EmbeddedEventReceiver {
    event_rx: mpsc::Receiver<ServerEvent>,
    echoes: SharedEchoFilter,
}

impl EmbeddedBackend {
    /// Address the audio streaming port is bound to (the real port when configured as 0).
    #[must_use]
    pub const fn streaming_addr(&self) -> std::net::SocketAddr {
        self.streaming_addr
    }

    /// Start the embedded server. Returns the backend + event receiver.
    ///
    /// # Errors
    ///
    /// Returns an error if the server cannot be started.
    pub fn start(
        config: &AppConfig,
        store: state::SharedState,
    ) -> Result<(Self, EmbeddedEventReceiver)> {
        let (codec, encryption_psk) = codec_and_key(&config.snapcast);

        // Build client filter based on unknown_clients policy
        let client_filter: Option<std::sync::Arc<dyn snapcast_server::auth::ClientFilter>> =
            if config.snapcast.unknown_clients == crate::config::UnknownClientPolicy::Reject {
                struct MacAllowlist(Vec<String>);
                impl snapcast_server::auth::ClientFilter for MacAllowlist {
                    fn accept(&self, hello: &snapcast_server::Hello) -> bool {
                        self.0.iter().any(|m| m == &hello.mac.to_lowercase())
                    }
                }
                let allowed_macs: Vec<String> = config
                    .clients
                    .iter()
                    .map(|c| c.mac.to_lowercase())
                    .collect();
                Some(std::sync::Arc::new(MacAllowlist(allowed_macs)))
            } else {
                None
            };

        // 0.17: the library no longer owns the port or persists state.
        // `stream_port` moved to the embedder (we bind the listener below), and
        // `state_file` is gone — `initial_state` defaults to `None`, so the
        // embedded server starts empty and snapdog's own `Store`
        // (sync_initial_state / reconcile_zone_groups) owns cross-restart state.
        let server_config = ServerConfig {
            buffer_ms: DEFAULT_BUFFER_MS,
            codec,
            sample_format: config.audio.sample_format(),
            encryption_psk,
            client_filter,
            ..ServerConfig::default()
        };

        let (mut server, event_rx) = SnapServer::new(server_config);

        // One stream per zone
        let mut audio_txs = HashMap::new();
        for zone in &config.zones {
            let tx = server
                .add_f32_stream(&zone.stream_name)
                .map_err(|e| anyhow::anyhow!(e))?;
            audio_txs.insert(zone.index, tokio::sync::Mutex::new(tx));
        }

        let cmd_tx = server.command_sender();

        // 0.17: the library no longer binds the port — the embedder owns the
        // TcpListener and hands it to serve(). A port clash fails boot here
        // (fail-fast) rather than logging inside the server task and limping on.
        let std_listener = bind_streaming_listener(&config.snapcast)?;
        let streaming_addr = std_listener
            .local_addr()
            .context("read the embedded Snapcast listener address")?;
        let listener = tokio::net::TcpListener::from_std(std_listener)
            .context("adopt embedded Snapcast listener into the tokio reactor")?;

        tokio::spawn(async move {
            if let Err(e) = server.serve(listener).await {
                tracing::error!(error = %e, "Embedded Snapcast server error");
            }
        });

        tracing::info!(
            address = %streaming_addr,
            zones = audio_txs.len(),
            "Embedded Snapcast server started"
        );

        let echoes = SharedEchoFilter::default();
        Ok((
            Self {
                cmd_tx,
                audio_txs,
                store,
                volume_modes: config
                    .zones
                    .iter()
                    .map(|z| (z.index, z.group_volume_mode))
                    .collect(),
                echoes: Arc::clone(&echoes),
                streaming_addr,
            },
            EmbeddedEventReceiver { event_rx, echoes },
        ))
    }

    /// Map a `SnapcastCmd` to one or more `ServerCommand`s.
    fn map_command(cmd: SnapcastCmd) -> Vec<ServerCommand> {
        match cmd {
            SnapcastCmd::Group { group_id, action } => match action {
                GroupAction::Stream(stream_id) => {
                    vec![ServerCommand::SetGroupStream {
                        group_id,
                        stream_id,
                    }]
                }
                GroupAction::Clients(clients) => {
                    vec![ServerCommand::SetGroupClients { group_id, clients }]
                }
                GroupAction::Name(name) => {
                    vec![ServerCommand::SetGroupName { group_id, name }]
                }
                GroupAction::Volume(_) => vec![], // handled in execute()
                GroupAction::Mute(muted) => {
                    vec![ServerCommand::SetGroupMute { group_id, muted }]
                }
            },
            SnapcastCmd::Client { client_id, action } => match action {
                ClientAction::Volume(percent) => {
                    vec![ServerCommand::SetClientVolume {
                        client_id,
                        volume: percent.clamp(0, 100) as u16,
                        muted: false, // preserve current — server merges
                    }]
                }
                ClientAction::Mute(muted) => {
                    vec![ServerCommand::SetClientVolume {
                        client_id,
                        volume: 0, // preserve current — server merges
                        muted,
                    }]
                }
                ClientAction::Latency(ms) => {
                    vec![ServerCommand::SetClientLatency {
                        client_id,
                        latency: ms,
                    }]
                }
                ClientAction::SendCustom { type_id, payload } => {
                    vec![ServerCommand::SendToClient {
                        client_id,
                        message: snapcast_server::CustomMessage::new(type_id, payload),
                    }]
                }
                ClientAction::AdjustVolume(_) => {
                    // Converted to absolute Volume in main loop before reaching backend
                    vec![]
                }
            },
            SnapcastCmd::ReconcileZones => unreachable!("handled in execute"),
        }
    }

    /// Map a `ServerEvent` to a `SnapcastEvent`.
    fn map_event(event: ServerEvent) -> Option<SnapcastEvent> {
        match event {
            ServerEvent::ClientConnected { id, hello } => Some(SnapcastEvent::ClientConnected {
                id,
                hello: super::backend::ClientHello {
                    client_name: hello.client_name,
                    mac: hello.mac,
                    host_name: hello.host_name,
                    version: hello.version,
                },
            }),
            ServerEvent::ClientDisconnected { id } => {
                Some(SnapcastEvent::ClientDisconnected { id })
            }
            ServerEvent::ClientVolumeChanged {
                client_id,
                volume,
                muted,
            } => Some(SnapcastEvent::ClientVolumeChanged {
                id: client_id,
                volume: i32::from(volume),
                muted,
            }),
            ServerEvent::ClientLatencyChanged { client_id, latency } => {
                Some(SnapcastEvent::ClientLatencyChanged {
                    id: client_id,
                    latency,
                })
            }
            ServerEvent::ClientNameChanged { client_id, name } => {
                Some(SnapcastEvent::ClientNameChanged {
                    id: client_id,
                    name,
                })
            }
            ServerEvent::GroupStreamChanged { .. }
            | ServerEvent::GroupMuteChanged { .. }
            | ServerEvent::GroupNameChanged { .. }
            | ServerEvent::StreamStatus { .. }
            | ServerEvent::ServerUpdated => Some(SnapcastEvent::ServerUpdated),
            ServerEvent::CustomMessage {
                client_id, message, ..
            } => Some(SnapcastEvent::CustomMessage {
                client_id,
                type_id: message.type_id,
                payload: message.payload,
            }),
            _ => None,
        }
    }
}

impl EmbeddedEventReceiver {
    /// Receive the next mapped event. Returns `None` when the server shuts down.
    pub async fn recv(&mut self) -> Option<SnapcastEvent> {
        loop {
            let event = self.event_rx.recv().await?;
            if let Some(mapped) = self.classify(event) {
                return Some(mapped);
            }
        }
    }

    /// Map an event, telling the echo of a volume push from a client's own change.
    fn classify(&self, event: ServerEvent) -> Option<SnapcastEvent> {
        if let ServerEvent::ClientVolumeChanged {
            ref client_id,
            volume,
            muted,
        } = event
        {
            let echo = self
                .echoes
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .is_echo(client_id, volume, muted, Instant::now());
            if !echo {
                return Some(SnapcastEvent::ClientVolumeReported {
                    id: client_id.clone(),
                    volume: i32::from(volume),
                    muted,
                });
            }
        }
        EmbeddedBackend::map_event(event)
    }
}

impl SnapcastBackend for EmbeddedBackend {
    fn send_audio(
        &self,
        zone_index: usize,
        samples: &[f32],
        _sample_rate: u32,
        _channels: u16,
    ) -> BoxFuture<'_, Result<()>> {
        let samples = samples.to_vec();
        Box::pin(async move {
            let tx = self
                .audio_txs
                .get(&zone_index)
                .ok_or_else(|| anyhow::anyhow!("No audio stream for zone {zone_index}"))?;
            tx.lock()
                .await
                .send(&samples)
                .await
                .map_err(|_| anyhow::anyhow!("Audio channel closed for zone {zone_index}"))
        })
    }

    #[allow(clippy::too_many_lines)] // Command match dispatcher — splitting would obscure control flow
    fn execute(&self, cmd: SnapcastCmd) -> BoxFuture<'_, Result<()>> {
        Box::pin(async move {
            let commands = match &cmd {
                // Group volume: compute per-client effective volume based on mode
                SnapcastCmd::Group {
                    group_id,
                    action: GroupAction::Volume(percent),
                } => {
                    let s = self.store.read().await;
                    let zone_index = s
                        .zones
                        .iter()
                        .find(|(_, z)| z.snapcast_group_id.as_deref() == Some(group_id))
                        .map(|(&zi, _)| zi);
                    let mode = zone_index
                        .and_then(|zi| self.volume_modes.get(&zi).copied())
                        .unwrap_or_default();
                    s.clients
                        .values()
                        .filter(|c| zone_index.is_some_and(|zi| c.zone_index == zi))
                        .filter_map(|c| {
                            c.snapcast_id
                                .as_ref()
                                .map(|sid| ServerCommand::SetClientVolume {
                                    client_id: sid.clone(),
                                    volume: mode.effective(c.base_volume, *percent, c.max_volume)
                                        as u16,
                                    muted: c.muted,
                                })
                        })
                        .collect()
                }
                // Mute needs current volume — SetClientVolume sets both
                SnapcastCmd::Client {
                    client_id,
                    action: ClientAction::Mute(muted),
                } => {
                    let s = self.store.read().await;
                    let volume = s
                        .clients
                        .values()
                        .find(|c| c.snapcast_id.as_deref() == Some(client_id))
                        .map_or(100, |c| c.volume.clamp(0, 100) as u16);
                    drop(s);
                    vec![ServerCommand::SetClientVolume {
                        client_id: client_id.clone(),
                        volume,
                        muted: *muted,
                    }]
                }
                // Volume: update base_volume, compute effective via zone mode
                SnapcastCmd::Client {
                    client_id,
                    action: ClientAction::Volume(percent),
                } => {
                    let mut s = self.store.write().await;
                    // Read zone info before mutable client borrow
                    let client_zone = s
                        .clients
                        .values()
                        .find(|c| c.snapcast_id.as_deref() == Some(client_id))
                        .map(|c| (c.zone_index, c.muted, c.max_volume));
                    let (zone_vol, mode) = client_zone.map_or_else(
                        || (100, crate::config::GroupVolumeMode::default()),
                        |(zi, _, _)| {
                            let zv = s.zones.get(&zi).map_or(100, |z| z.volume);
                            let m = self.volume_modes.get(&zi).copied().unwrap_or_default();
                            (zv, m)
                        },
                    );
                    let muted = client_zone.is_some_and(|(_, m, _)| m);
                    let max_vol = client_zone.map_or(100, |(_, _, mv)| mv);
                    // Now mutate
                    if let Some(c) = s
                        .clients
                        .values_mut()
                        .find(|c| c.snapcast_id.as_deref() == Some(client_id))
                    {
                        c.base_volume = (*percent).clamp(0, max_vol);
                    }
                    let base = (*percent).clamp(0, max_vol);
                    drop(s);
                    vec![ServerCommand::SetClientVolume {
                        client_id: client_id.clone(),
                        volume: mode.effective(base, zone_vol, max_vol) as u16,
                        muted,
                    }]
                }
                SnapcastCmd::ReconcileZones => {
                    // Move clients to their zone's group. Issue SetGroupClients for
                    // ALL zones that have a known group — including zones losing clients
                    // (empty list removes the client from that group, snapcast-server
                    // creates a new group for evicted clients automatically).
                    // The ServerUpdated event then triggers sync_group_ids which
                    // picks up new groups and fixes stream assignments.
                    let s = self.store.read().await;

                    // Build zone_index → [snapcast_ids] for zones with clients
                    let mut zone_clients: HashMap<usize, Vec<String>> = HashMap::new();
                    for c in s.clients.values() {
                        if let Some(ref sid) = c.snapcast_id {
                            zone_clients
                                .entry(c.zone_index)
                                .or_default()
                                .push(sid.clone());
                        }
                    }

                    let mut cmds = vec![];
                    for (&zi, zone) in &s.zones {
                        if let Some(ref gid) = zone.snapcast_group_id {
                            let clients = zone_clients.remove(&zi).unwrap_or_default();
                            cmds.push(ServerCommand::SetGroupClients {
                                group_id: gid.clone(),
                                clients,
                            });
                        }
                    }
                    drop(s);
                    cmds
                }
                _ => Self::map_command(cmd),
            };
            for c in commands {
                // Before sending: the echo can arrive before `send` returns.
                if let ServerCommand::SetClientVolume {
                    ref client_id,
                    volume,
                    muted,
                } = c
                {
                    self.echoes
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .expect(client_id, volume, muted, Instant::now());
                }
                self.cmd_tx
                    .send(c)
                    .await
                    .context("Server command channel closed")?;
            }
            Ok(())
        })
    }

    fn stop(&self) -> BoxFuture<'_, Result<()>> {
        Box::pin(async move {
            self.cmd_tx
                .send(ServerCommand::Stop)
                .await
                .context("Failed to send stop command")
        })
    }

    fn get_status(&self) -> BoxFuture<'_, Result<serde_json::Value>> {
        Box::pin(async move {
            let (tx, rx) = tokio::sync::oneshot::channel();
            self.cmd_tx
                .send(ServerCommand::GetStatus { response_tx: tx })
                .await
                .context("Server command channel closed")?;
            let status = rx.await.context("Status response channel closed")?;
            serde_json::to_value(status).context("Failed to serialize status")
        })
    }

    fn delete_client(&self, id: &str) -> BoxFuture<'_, Result<()>> {
        let client_id = id.to_string();
        Box::pin(async move {
            self.cmd_tx
                .send(ServerCommand::DeleteClient { client_id })
                .await
                .context("Server command channel closed")
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config;

    fn snapcast_config(extra: &str) -> config::AppConfig {
        config::load_str(&format!(
            "[snapcast]\n{extra}\n[[zone]]\nname = \"Living\"\n"
        ))
        .expect("test config loads")
    }

    // ── codec and key at the use site ────────────────────────────

    #[test]
    fn f32lz4e_encrypts_with_the_configured_key_or_the_public_default() {
        let keyed = snapcast_config("codec = \"f32lz4e\"\nencryption_psk = \"s3cret\"");
        assert_eq!(
            codec_and_key(&keyed.snapcast),
            ("f32lz4".to_string(), Some("s3cret".to_string()))
        );
        let unkeyed = snapcast_config("codec = \"f32lz4e\"");
        assert_eq!(
            codec_and_key(&unkeyed.snapcast),
            (
                "f32lz4".to_string(),
                Some(snapcast_server::DEFAULT_ENCRYPTION_PSK.to_string())
            )
        );
    }

    #[test]
    fn an_empty_key_is_never_used_as_a_secret() {
        // Config loading refuses this combination; the use site must still not trust it.
        let mut empty = snapcast_config("codec = \"f32lz4e\"\nencryption_psk = \"x\"");
        empty.snapcast.encryption_psk = Some(crate::config::SecretString::from(String::new()));
        assert_eq!(
            codec_and_key(&empty.snapcast).1.as_deref(),
            Some(snapcast_server::DEFAULT_ENCRYPTION_PSK)
        );
        let empty_plain = snapcast_config("codec = \"flac\"\nencryption_psk = \"\"");
        assert_eq!(
            codec_and_key(&empty_plain.snapcast),
            ("flac".to_string(), None)
        );
        let keyed_plain = snapcast_config("codec = \"flac\"\nencryption_psk = \"k\"");
        assert_eq!(
            codec_and_key(&keyed_plain.snapcast),
            ("flac".to_string(), Some("k".to_string()))
        );
    }

    // ── the streaming port (1704) ────────────────────────────────

    #[test]
    fn the_streaming_port_listens_on_all_interfaces_unless_told_otherwise() {
        let config = snapcast_config("streaming_port = 0");
        let listener = bind_streaming_listener(&config.snapcast).unwrap();
        assert_eq!(
            listener.local_addr().unwrap().ip(),
            std::net::Ipv4Addr::UNSPECIFIED
        );
    }

    #[test]
    fn streaming_bind_restricts_the_listener_to_one_address() {
        let config = snapcast_config("streaming_port = 0\nstreaming_bind = \"127.0.0.1\"");
        let listener = bind_streaming_listener(&config.snapcast).unwrap();
        assert_eq!(
            listener.local_addr().unwrap().ip(),
            std::net::Ipv4Addr::LOCALHOST
        );
    }

    #[test]
    fn the_port_check_binds_the_configured_address_not_the_external_server_address() {
        // `address` belongs to an external snapserver. It names a host that is not this
        // machine, so using it for the bind would fail.
        let config = snapcast_config("streaming_port = 0\naddress = \"192.0.2.1\"");
        check_streaming_port(&config.snapcast).expect("the port is free");
    }

    #[test]
    fn the_port_check_reports_a_taken_port_on_the_configured_address() {
        let taken = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = taken.local_addr().unwrap().port();
        let config = snapcast_config(&format!(
            "streaming_port = {port}\nstreaming_bind = \"127.0.0.1\""
        ));
        let error = check_streaming_port(&config.snapcast).unwrap_err();
        assert!(
            format!("{error:#}").contains(&format!("127.0.0.1:{port}")),
            "{error:#}"
        );
        drop(taken);
        check_streaming_port(&config.snapcast).expect("free again");
    }

    #[test]
    fn a_bind_failure_is_explained_by_its_kind() {
        use std::io::{Error, ErrorKind};
        let addr: std::net::SocketAddr = "192.0.2.1:1704".parse().unwrap();
        let in_use = streaming_bind_message(addr, &Error::from(ErrorKind::AddrInUse));
        assert!(in_use.contains("already in use"), "{in_use}");
        assert!(in_use.contains("already running"), "{in_use}");

        let unassigned = streaming_bind_message(addr, &Error::from(ErrorKind::AddrNotAvailable));
        assert!(
            unassigned.contains("192.0.2.1 is not an address of this host"),
            "{unassigned}"
        );
        assert!(unassigned.contains("streaming_bind"), "{unassigned}");
        assert!(!unassigned.contains("already running"), "{unassigned}");

        let denied = streaming_bind_message(addr, &Error::from(ErrorKind::PermissionDenied));
        assert!(denied.contains("permission denied"), "{denied}");
        assert!(!denied.contains("already running"), "{denied}");

        let other = streaming_bind_message(addr, &Error::from(ErrorKind::TimedOut));
        assert!(!other.contains("already running"), "{other}");
    }

    #[test]
    fn the_real_errors_get_the_matching_message() {
        // Taken port: a second instance.
        let taken = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = taken.local_addr().unwrap().port();
        let config = snapcast_config(&format!(
            "streaming_port = {port}\nstreaming_bind = \"127.0.0.1\""
        ));
        let error = check_streaming_port(&config.snapcast)
            .unwrap_err()
            .to_string();
        assert!(error.contains("already running"), "{error}");

        // An address that no interface has (TEST-NET-1, reserved for documentation).
        let config = snapcast_config("streaming_port = 0\nstreaming_bind = \"192.0.2.1\"");
        let error = check_streaming_port(&config.snapcast)
            .unwrap_err()
            .to_string();
        assert!(error.contains("not an address of this host"), "{error}");
        assert!(!error.contains("already running"), "{error}");
    }

    #[tokio::test]
    async fn the_started_server_accepts_connections_on_the_configured_address() {
        let config = snapcast_config("streaming_port = 0\nstreaming_bind = \"127.0.0.1\"");
        let store = crate::state::init(&config, None).unwrap();
        let (backend, _events) = EmbeddedBackend::start(&config, store).unwrap();
        let addr = backend.streaming_addr();
        assert_eq!(addr.ip(), std::net::Ipv4Addr::LOCALHOST);
        assert_ne!(addr.port(), 0, "the real port is reported");
        tokio::net::TcpStream::connect(addr)
            .await
            .expect("the server listens there");
    }

    #[test]
    fn map_command_group_stream() {
        let cmd = SnapcastCmd::Group {
            group_id: "g1".into(),
            action: GroupAction::Stream("Zone1".into()),
        };
        let cmds = EmbeddedBackend::map_command(cmd);
        assert_eq!(cmds.len(), 1);
        assert!(
            matches!(&cmds[0], ServerCommand::SetGroupStream { group_id, stream_id } if group_id == "g1" && stream_id == "Zone1")
        );
    }

    #[test]
    fn map_command_group_clients() {
        let cmd = SnapcastCmd::Group {
            group_id: "g1".into(),
            action: GroupAction::Clients(vec!["c1".into(), "c2".into()]),
        };
        let cmds = EmbeddedBackend::map_command(cmd);
        assert_eq!(cmds.len(), 1);
        assert!(
            matches!(&cmds[0], ServerCommand::SetGroupClients { clients, .. } if clients.len() == 2)
        );
    }

    #[test]
    fn map_command_group_mute() {
        let cmd = SnapcastCmd::Group {
            group_id: "g1".into(),
            action: GroupAction::Mute(true),
        };
        let cmds = EmbeddedBackend::map_command(cmd);
        assert!(matches!(
            &cmds[0],
            ServerCommand::SetGroupMute { muted: true, .. }
        ));
    }

    #[test]
    fn map_command_group_volume_noop() {
        let cmd = SnapcastCmd::Group {
            group_id: "g1".into(),
            action: GroupAction::Volume(50),
        };
        assert!(EmbeddedBackend::map_command(cmd).is_empty());
    }

    #[test]
    fn map_command_client_volume() {
        let cmd = SnapcastCmd::Client {
            client_id: "c1".into(),
            action: ClientAction::Volume(75),
        };
        let cmds = EmbeddedBackend::map_command(cmd);
        assert!(matches!(
            &cmds[0],
            ServerCommand::SetClientVolume { volume: 75, .. }
        ));
    }

    #[test]
    fn map_command_client_volume_clamps() {
        let cmd = SnapcastCmd::Client {
            client_id: "c1".into(),
            action: ClientAction::Volume(200),
        };
        let cmds = EmbeddedBackend::map_command(cmd);
        assert!(matches!(
            &cmds[0],
            ServerCommand::SetClientVolume { volume: 100, .. }
        ));
    }

    #[test]
    fn map_command_client_mute() {
        let cmd = SnapcastCmd::Client {
            client_id: "c1".into(),
            action: ClientAction::Mute(true),
        };
        let cmds = EmbeddedBackend::map_command(cmd);
        assert!(matches!(
            &cmds[0],
            ServerCommand::SetClientVolume { muted: true, .. }
        ));
    }

    #[test]
    fn map_command_client_latency() {
        let cmd = SnapcastCmd::Client {
            client_id: "c1".into(),
            action: ClientAction::Latency(50),
        };
        let cmds = EmbeddedBackend::map_command(cmd);
        assert!(matches!(
            &cmds[0],
            ServerCommand::SetClientLatency { latency: 50, .. }
        ));
    }

    #[test]
    fn map_event_client_connected() {
        let event = ServerEvent::ClientConnected {
            id: "c1".into(),
            hello: snapcast_server::Hello {
                mac: "aa:bb:cc:dd:ee:ff".into(),
                host_name: "Kitchen".into(),
                version: "0.1.0".into(),
                client_name: "Snapclient".into(),
                os: "Linux".into(),
                arch: "x86_64".into(),
                instance: 1,
                id: "c1".into(),
                snap_stream_protocol_version: 2,
                auth: None,
            },
        };
        let mapped = EmbeddedBackend::map_event(event).unwrap();
        assert!(
            matches!(mapped, SnapcastEvent::ClientConnected { id, ref hello } if id == "c1" && hello.host_name == "Kitchen")
        );
    }

    #[test]
    fn map_event_client_disconnected() {
        let event = ServerEvent::ClientDisconnected { id: "c1".into() };
        let mapped = EmbeddedBackend::map_event(event).unwrap();
        assert!(matches!(mapped, SnapcastEvent::ClientDisconnected { id } if id == "c1"));
    }

    #[test]
    fn map_event_volume_changed() {
        let event = ServerEvent::ClientVolumeChanged {
            client_id: "c1".into(),
            volume: 80,
            muted: false,
        };
        let mapped = EmbeddedBackend::map_event(event).unwrap();
        assert!(matches!(
            mapped,
            SnapcastEvent::ClientVolumeChanged {
                volume: 80,
                muted: false,
                ..
            }
        ));
    }

    #[test]
    fn echoes_of_pushes_are_recognised_in_order() {
        let mut f = EchoFilter::default();
        let t = Instant::now();
        f.expect("c1", 40, false, t);
        f.expect("c1", 41, false, t);
        assert!(f.is_echo("c1", 40, false, t));
        assert!(f.is_echo("c1", 41, false, t));
        assert!(!f.is_echo("c1", 41, false, t), "each push echoes once");
        assert!(!f.is_echo("c2", 40, false, t), "per client");
    }

    #[test]
    fn a_value_never_pushed_is_the_clients_own() {
        let mut f = EchoFilter::default();
        let t = Instant::now();
        f.expect("c1", 40, false, t);
        assert!(!f.is_echo("c1", 55, false, t));
        assert!(!f.is_echo("c1", 40, true, t), "mute differs");
        assert!(f.is_echo("c1", 40, false, t), "the push still waits");
    }

    #[test]
    fn a_dropped_echo_does_not_swallow_a_later_report() {
        let mut f = EchoFilter::default();
        let t = Instant::now();
        f.expect("c1", 40, false, t);
        f.expect("c1", 41, false, t);
        // The echo of 40 got lost; 41 arrives and clears both.
        assert!(f.is_echo("c1", 41, false, t));
        assert!(!f.is_echo("c1", 40, false, t));
    }

    #[test]
    fn unanswered_pushes_expire() {
        let mut f = EchoFilter::default();
        let t = Instant::now();
        f.expect("c1", 40, false, t);
        assert!(!f.is_echo("c1", 40, false, t + ECHO_WINDOW));
    }

    #[test]
    fn pending_pushes_are_bounded() {
        let mut f = EchoFilter::default();
        let t = Instant::now();
        for v in 0..200 {
            f.expect("c1", v, false, t);
        }
        assert_eq!(f.pending["c1"].len(), MAX_PENDING_ECHOES);
    }

    #[test]
    fn classify_splits_echoes_from_reports() {
        let (_tx, event_rx) = mpsc::channel(1);
        let rx = EmbeddedEventReceiver {
            event_rx,
            echoes: SharedEchoFilter::default(),
        };
        rx.echoes
            .lock()
            .unwrap()
            .expect("c1", 30, false, Instant::now());
        let event = |volume| ServerEvent::ClientVolumeChanged {
            client_id: "c1".into(),
            volume,
            muted: false,
        };
        assert!(matches!(
            rx.classify(event(30)),
            Some(SnapcastEvent::ClientVolumeChanged { volume: 30, .. })
        ));
        assert!(matches!(
            rx.classify(event(35)),
            Some(SnapcastEvent::ClientVolumeReported { volume: 35, .. })
        ));
    }

    #[test]
    fn map_event_group_events_become_server_updated() {
        let event = ServerEvent::GroupMuteChanged {
            group_id: "g1".into(),
            muted: true,
        };
        assert!(matches!(
            EmbeddedBackend::map_event(event),
            Some(SnapcastEvent::ServerUpdated)
        ));
    }

    #[test]
    fn map_event_group_name_changed() {
        let event = ServerEvent::GroupNameChanged {
            group_id: "g1".into(),
            name: "Living Room".into(),
        };
        assert!(matches!(
            EmbeddedBackend::map_event(event),
            Some(SnapcastEvent::ServerUpdated)
        ));
    }

    #[test]
    fn map_event_server_updated() {
        assert!(matches!(
            EmbeddedBackend::map_event(ServerEvent::ServerUpdated),
            Some(SnapcastEvent::ServerUpdated)
        ));
    }

    #[test]
    fn map_event_client_latency_changed() {
        let event = ServerEvent::ClientLatencyChanged {
            client_id: "c1".into(),
            latency: 42,
        };
        let mapped = EmbeddedBackend::map_event(event).unwrap();
        assert!(matches!(
            mapped,
            SnapcastEvent::ClientLatencyChanged { id, latency } if id == "c1" && latency == 42
        ));
    }

    #[test]
    fn map_event_client_name_changed() {
        let event = ServerEvent::ClientNameChanged {
            client_id: "c1".into(),
            name: "Kitchen".into(),
        };
        let mapped = EmbeddedBackend::map_event(event).unwrap();
        assert!(matches!(
            mapped,
            SnapcastEvent::ClientNameChanged { id, name } if id == "c1" && name == "Kitchen"
        ));
    }

    #[test]
    fn map_event_stream_and_group_stream_collapse_to_server_updated() {
        // The remaining members of the collapse (mute/name already covered above).
        for event in [
            ServerEvent::GroupStreamChanged {
                group_id: "g1".into(),
                stream_id: "Zone1".into(),
            },
            ServerEvent::StreamStatus {
                stream_id: "Zone1".into(),
                status: "playing".into(),
            },
        ] {
            assert!(matches!(
                EmbeddedBackend::map_event(event),
                Some(SnapcastEvent::ServerUpdated)
            ));
        }
    }

    #[test]
    fn map_event_custom_message_extracts_type_and_payload() {
        let event = ServerEvent::CustomMessage {
            client_id: "c1".into(),
            message: snapcast_proto::CustomMessage {
                type_id: 14,
                payload: vec![1, 2, 3],
            },
        };
        let mapped = EmbeddedBackend::map_event(event).unwrap();
        assert!(matches!(
            mapped,
            SnapcastEvent::CustomMessage { client_id, type_id, payload }
                if client_id == "c1" && type_id == 14 && payload == vec![1, 2, 3]
        ));
    }

    #[test]
    fn map_event_unhandled_variants_are_dropped() {
        // These ServerEvents intentionally map to None (the `_ => None` arm),
        // pinning the *silent drop* — notably stream metadata and stream-control
        // are not propagated today. NOTE: `ServerEvent` is `#[non_exhaustive]` (a
        // foreign enum), so a compile-time exhaustiveness guard is impossible — a
        // future upgrade adding a new variant silently lands here as None until
        // `map_event` is taught about it. This test is the behavioral firewall.
        assert!(
            EmbeddedBackend::map_event(ServerEvent::StreamMetaChanged {
                stream_id: "Zone1".into(),
                metadata: std::collections::HashMap::new(),
            })
            .is_none()
        );
        assert!(
            EmbeddedBackend::map_event(ServerEvent::StreamControl {
                stream_id: "Zone1".into(),
                command: "play".into(),
                params: serde_json::Value::Null,
            })
            .is_none()
        );
        // `StateChanged` (new in snapcast-server 0.17) is deliberately dropped:
        // snapdog does not persist the library's `ServerState` — its own `Store`
        // (sync_initial_state / reconcile_zone_groups) owns cross-restart state.
        // Pin that this stays a no-op mapping so the choice is explicit, not a
        // forgotten variant silently falling through `_ => None`.
        assert!(
            EmbeddedBackend::map_event(ServerEvent::StateChanged(
                snapcast_server::ServerState::default()
            ))
            .is_none()
        );
    }
}
