// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! Fixtures shared by the MQTT unit tests.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use bytes::BytesMut;
use rumqttc::SubscribeReasonCode;
use rumqttc::mqttbytes::Error as WireError;
use rumqttc::{ConnAck, ConnectReturnCode, Packet, PubAck, Publish, QoS, SubAck};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{RwLock, broadcast, mpsc};
use tokio::task::JoinHandle;

use super::MqttRuntime;

use crate::player::{SnapcastCmd, ZoneCommand, ZoneCommandSender};
use crate::state;

/// Two zones and one connected client (`snap-1`) in zone 1.
pub fn test_state_with_client() -> state::SharedState {
    let store: state::Store = serde_json::from_value(serde_json::json!({
        "zones": {
            "1": { "name": "Zone 1", "icon": "", "volume": 50, "muted": false, "playback": "stopped", "shuffle": false, "repeat": "off", "track": null, "playlist_index": null, "playlist_name": null, "playlist_total": null, "playlist_track_index": null, "playlist_track_count": null, "source": "idle", "cover_url": null, "snapcast_group_id": null, "presence": false, "presence_enabled": true, "presence_source": false, "auto_off_active": false, "auto_off_delay": 0 },
            "2": { "name": "Zone 2", "icon": "", "volume": 50, "muted": false, "playback": "stopped", "shuffle": false, "repeat": "off", "track": null, "playlist_index": null, "playlist_name": null, "playlist_total": null, "playlist_track_index": null, "playlist_track_count": null, "source": "idle", "cover_url": null, "snapcast_group_id": null, "presence": false, "presence_enabled": true, "presence_source": false, "auto_off_active": false, "auto_off_delay": 0 }
        },
        "clients": {
            "1": {
                "name": "Test", "icon": "", "mac": "", "zone_index": 1,
                "volume": 50, "base_volume": 50, "muted": false,
                "latency_ms": 0, "connected": true, "snapcast_id": "snap-1"
            }
        }
    }))
    .unwrap();
    Arc::new(RwLock::new(store))
}

/// A command channel for zone 1 with room for `capacity` commands.
pub fn zone_channels_with(
    capacity: usize,
) -> (
    HashMap<usize, ZoneCommandSender>,
    mpsc::Receiver<ZoneCommand>,
) {
    let (tx, rx) = mpsc::channel(capacity);
    let mut map = HashMap::new();
    map.insert(1, tx);
    (map, rx)
}

/// A command channel for zone 1 with room for 16 commands.
pub fn zone_channels() -> (
    HashMap<usize, ZoneCommandSender>,
    mpsc::Receiver<ZoneCommand>,
) {
    zone_channels_with(16)
}

/// A Snapcast command channel with room for 16 commands.
pub fn snap_channel() -> (mpsc::Sender<SnapcastCmd>, mpsc::Receiver<SnapcastCmd>) {
    mpsc::channel(16)
}

/// The receiving ends of a [`test_runtime`], kept alive by the test.
pub struct RuntimeEnds {
    pub store: state::SharedState,
    pub zone_rx: mpsc::Receiver<ZoneCommand>,
    pub _snap_rx: mpsc::Receiver<SnapcastCmd>,
    pub notify_tx: broadcast::Sender<Arc<str>>,
}

/// What a bridge needs from the server, wired to channels the test owns.
pub fn test_runtime() -> (MqttRuntime, RuntimeEnds) {
    let (zone_commands, zone_rx) = zone_channels();
    let (snap_tx, snap_rx) = snap_channel();
    let (notify_tx, notifications) = broadcast::channel(16);
    let store = test_state_with_client();
    (
        MqttRuntime {
            zone_commands,
            state: store.clone(),
            snap_tx,
            notifications,
            server: test_server(),
        },
        RuntimeEnds {
            store,
            zone_rx,
            _snap_rx: snap_rx,
            notify_tx,
        },
    )
}

// ── A scripted broker ─────────────────────────────────────────

/// How a [`FakeBroker`] treats a client. It speaks just enough MQTT 3.1.1 to
/// accept a connection, and records what the client sends, so that a test can
/// look at the bytes on the wire (the quality of service of a subscription, an
/// oversized packet from the broker) without a container.
#[derive(Clone, Copy, Debug)]
pub enum Script {
    /// CONNACK, acknowledge subscriptions and `QoS` 1 publishes.
    Serve,
    /// CONNACK, then close the socket at once (what two clients with one client id
    /// do to each other).
    DropAfterConnack,
    /// Serve, and answer the first SUBSCRIBE with a retained PUBLISH of this many
    /// bytes.
    SendOversized(usize),
    /// Serve, and answer the SUBSCRIBE to this topic with a PUBLISH of `online` on it
    /// (Home Assistant's birth message).
    Birth(&'static str),
}

/// A broker on a loopback port that follows a [`Script`].
pub struct FakeBroker {
    pub port: u16,
    inject: broadcast::Sender<(String, Vec<u8>)>,
    /// Connections accepted so far.
    pub connections: Arc<AtomicUsize>,
    /// Every packet received, in order, over all connections.
    pub received: Arc<Mutex<Vec<Packet>>>,
    task: JoinHandle<()>,
}

impl FakeBroker {
    pub async fn start(script: Script) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let connections = Arc::new(AtomicUsize::new(0));
        let received = Arc::new(Mutex::new(Vec::new()));
        let (count, log) = (connections.clone(), received.clone());
        let (inject, _) = broadcast::channel(16);
        let injected = inject.clone();
        let task = tokio::spawn(async move {
            while let Ok((socket, _)) = listener.accept().await {
                count.fetch_add(1, Ordering::SeqCst);
                tokio::spawn(serve_connection(
                    socket,
                    script,
                    log.clone(),
                    injected.subscribe(),
                ));
            }
        });
        Self {
            port,
            inject,
            connections,
            received,
            task,
        }
    }

    /// Send a `QoS` 0 `PUBLISH` to every connected client.
    pub fn publish(&self, topic: &str, payload: &[u8]) {
        let _ = self.inject.send((topic.to_string(), payload.to_vec()));
    }

    /// The packets received so far.
    pub fn packets(&self) -> Vec<Packet> {
        self.received.lock().unwrap().clone()
    }
}

impl Drop for FakeBroker {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn serve_connection(
    mut socket: TcpStream,
    script: Script,
    log: Arc<Mutex<Vec<Packet>>>,
    mut injected: broadcast::Receiver<(String, Vec<u8>)>,
) {
    let mut input = BytesMut::new();
    let mut oversized_sent = false;
    loop {
        let packet = match Packet::read(&mut input, usize::MAX) {
            Ok(packet) => packet,
            Err(WireError::InsufficientBytes(_)) => {
                tokio::select! {
                    read = socket.read_buf(&mut input) => match read {
                        Ok(0) | Err(_) => return,
                        Ok(_) => continue,
                    },
                    Ok((topic, payload)) = injected.recv() => {
                        let mut out = BytesMut::new();
                        Publish::new(topic, QoS::AtMostOnce, payload).write(&mut out).unwrap();
                        if socket.write_all(&out).await.is_err() {
                            return;
                        }
                        continue;
                    }
                }
            }
            Err(_) => return,
        };
        log.lock().unwrap().push(packet.clone());
        let mut out = BytesMut::new();
        match &packet {
            Packet::Connect(_) => {
                ConnAck::new(ConnectReturnCode::Success, false)
                    .write(&mut out)
                    .unwrap();
                if matches!(script, Script::DropAfterConnack) {
                    let _ = socket.write_all(&out).await;
                    return;
                }
            }
            Packet::Subscribe(subscribe) => {
                let granted = subscribe
                    .filters
                    .iter()
                    .map(|f| SubscribeReasonCode::Success(f.qos))
                    .collect();
                SubAck::new(subscribe.pkid, granted)
                    .write(&mut out)
                    .unwrap();
                if let Script::Birth(topic) = script
                    && subscribe.filters.iter().any(|f| f.path == topic)
                {
                    Publish::new(topic, QoS::AtMostOnce, b"online".to_vec())
                        .write(&mut out)
                        .unwrap();
                }
                if let Script::SendOversized(bytes) = script
                    && !oversized_sent
                {
                    oversized_sent = true;
                    let mut publish = Publish::new(
                        "snapdog/zones/1/volume/set",
                        QoS::AtMostOnce,
                        vec![b'1'; bytes],
                    );
                    publish.retain = true;
                    publish.write(&mut out).unwrap();
                }
            }
            Packet::Publish(publish) if publish.qos == QoS::AtLeastOnce => {
                PubAck::new(publish.pkid).write(&mut out).unwrap();
            }
            Packet::PingReq => out.extend_from_slice(&[0xD0, 0x00]),
            Packet::Disconnect => return,
            _ => {}
        }
        if !out.is_empty() && socket.write_all(&out).await.is_err() {
            return;
        }
    }
}

// ── Log capture ───────────────────────────────────────────────

/// What the tracing subscriber of [`capture_logs`] has written.
#[derive(Clone, Default)]
pub struct CapturedLogs(Arc<Mutex<Vec<u8>>>);

impl CapturedLogs {
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().unwrap()).into_owned()
    }
}

impl std::io::Write for CapturedLogs {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for CapturedLogs {
    type Writer = Self;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

/// Capture the log lines of the current thread until the guard is dropped. Only
/// meaningful in a `current_thread` runtime (the default of `#[tokio::test]`),
/// where the spawned tasks run on the test's own thread.
pub fn capture_logs() -> (tracing::subscriber::DefaultGuard, CapturedLogs) {
    let logs = CapturedLogs::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(logs.clone())
        .with_ansi(false)
        .with_max_level(tracing::Level::DEBUG)
        .finish();
    (tracing::subscriber::set_default(subscriber), logs)
}

/// The server the discovery tests announce.
pub fn test_server() -> super::ServerInfo {
    super::ServerInfo {
        id: "0b1c2d3e-aaaa-bbbb-cccc-1234567890ab".into(),
        name: "SnapDog".into(),
        base_url: "http://snapdog.local:5555".into(),
    }
}
