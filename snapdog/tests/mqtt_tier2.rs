// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! IT-T32 (tier-2) — the MQTT bridge against a real mosquitto broker via
//! testcontainers: availability and retained state, the last will on an ungraceful
//! disconnect, a command that travels broker → bridge → zone channel, recovery
//! after a broker restart, a graceful shutdown, and the cleanup of retained
//! messages that older releases left behind.
//!
//! The tests LOUD-SKIP (no panic) when Docker is unavailable, so a green run
//! without a Docker socket proves nothing: look for `SKIP` in the output. On macOS
//! with colima: `DOCKER_HOST=unix://$HOME/.colima/default/docker.sock`.
//!
//! Run: `cargo test -p snapdog --test mqtt_tier2 -- --nocapture --test-threads=1`

#![allow(clippy::doc_markdown)] // doc mentions MqttBridge / QoS / AppConfig / load_raw

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rumqttc::{AsyncClient, Event, MqttOptions, Packet, QoS};
use snapdog::mqtt::{MqttBridge, MqttRuntime};
use snapdog::player::ZoneCommand;
use snapdog::{config, state};
use testcontainers::{
    ContainerAsync, CopyTargetOptions, GenericImage, ImageExt,
    core::{ContainerPort, WaitFor},
    runners::AsyncRunner,
};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

// The throwaway CA of the TLS tests, shared with the unit tests.
#[path = "../src/mqtt/test_pki.rs"]
mod test_pki;

// ── Broker ────────────────────────────────────────────────────

/// A running anonymous mosquitto (the container lives as long as this value).
struct Broker {
    container: ContainerAsync<GenericImage>,
    host: String,
    port: u16,
}

fn mosquitto() -> GenericImage {
    // Keep the same broker and readiness condition as the former Mosquitto module.
    // GenericImage lets this test track current testcontainers independently.
    GenericImage::new("eclipse-mosquitto", "2.0.18").with_wait_for(WaitFor::message_on_stderr(
        "mosquitto version 2.0.18 running",
    ))
}

/// Start mosquitto, or `None` (after a loud SKIP line) when Docker is unavailable.
async fn start_broker(test: &str) -> Option<Broker> {
    let image = mosquitto().with_cmd(["mosquitto", "-c", "/mosquitto-no-auth.conf"]);
    let container = match image.start().await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("SKIP {test}: Docker/mosquitto unavailable: {e}");
            return None;
        }
    };
    let host = container.get_host().await.expect("host").to_string();
    let port = container
        .get_host_port_ipv4(1883)
        .await
        .expect("mapped port");
    Some(Broker {
        container,
        host,
        port,
    })
}

/// Like [`start_broker`], on a host port chosen up front. A restarted container
/// keeps an explicit mapping, a random one may change.
async fn start_broker_on_fixed_port(test: &str) -> Option<Broker> {
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .expect("free port")
        .local_addr()
        .expect("addr")
        .port();
    let image = mosquitto()
        .with_cmd(["mosquitto", "-c", "/mosquitto-no-auth.conf"])
        .with_mapped_port(port, ContainerPort::Tcp(1883));
    let container = match image.start().await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("SKIP {test}: Docker/mosquitto unavailable: {e}");
            return None;
        }
    };
    let host = container.get_host().await.expect("host").to_string();
    Some(Broker {
        container,
        host,
        port,
    })
}

// ── Bridge under test ─────────────────────────────────────────

/// AppConfig (via TOML → load_raw) with the given `[mqtt]` section.
fn config_with_mqtt(mqtt_section: &str) -> config::AppConfig {
    let toml = format!(
        r#"
[mqtt]
{mqtt_section}

[[zone]]
name = "Z1"

[[client]]
name = "C1"
mac = "02:42:ac:11:00:10"
zone = "Z1"
"#
    );
    let raw: config::FileConfig = toml::from_str(&toml).expect("test TOML parses");
    config::load_raw(raw).expect("config resolves")
}

/// AppConfig with `[mqtt]` pointed at the plaintext `host:port`.
fn config_for_broker(host: &str, port: u16) -> config::AppConfig {
    config_with_mqtt(&format!(
        "broker = \"{host}:{port}\"\nusername = \"\"\npassword = \"\"\nbase_topic = \"snapdog/\""
    ))
}

/// A started bridge and the receiving ends of the channels it commands.
struct Rig {
    bridge: MqttBridge,
    zone_rx: mpsc::Receiver<ZoneCommand>,
    _snap_rx: mpsc::Receiver<snapdog::player::SnapcastCmd>,
    _notify_tx: snapdog::api::ws::NotifySender,
}

async fn start_bridge(host: &str, port: u16) -> Rig {
    try_start_bridge(&config_for_broker(host, port))
        .await
        .expect("bridge starts")
}

async fn try_start_bridge(cfg: &config::AppConfig) -> anyhow::Result<Rig> {
    let mqtt_cfg = cfg.mqtt.clone().expect("mqtt config present");
    let store = state::init(cfg, None).expect("state init");

    let (zone_tx, zone_rx) = mpsc::channel(16);
    let zone_commands: HashMap<usize, snapdog::player::ZoneCommandSender> =
        HashMap::from([(1, zone_tx)]);
    let (snap_tx, snap_rx) = mpsc::channel(16);
    let (notify_tx, _) = snapdog::api::ws::notification_channel();

    let bridge = MqttBridge::start(
        &mqtt_cfg,
        "http://test:5555",
        MqttRuntime {
            zone_commands,
            state: store.clone(),
            snap_tx,
            notifications: notify_tx.subscribe(),
        },
    )
    .await?;
    Ok(Rig {
        bridge,
        zone_rx,
        _snap_rx: snap_rx,
        _notify_tx: notify_tx,
    })
}

// ── Second clients ────────────────────────────────────────────

/// One message as an independent subscriber saw it.
#[derive(Clone, Debug)]
struct Seen {
    topic: String,
    payload: String,
}

/// Subscribes to a filter with its own client and records everything it sees. It
/// subscribes again after every reconnect, so it also watches a broker restart.
struct Observer {
    log: Arc<Mutex<Vec<Seen>>>,
    task: JoinHandle<()>,
}

impl Observer {
    fn start(client_id: &str, host: &str, port: u16, filter: &str) -> Self {
        let mut opts = MqttOptions::new(client_id, host, port);
        opts.set_keep_alive(Duration::from_secs(5));
        let (client, mut eventloop) = AsyncClient::new(opts, 64);
        let log = Arc::new(Mutex::new(Vec::new()));
        let seen = log.clone();
        let filter = filter.to_string();
        let task = tokio::spawn(async move {
            loop {
                match eventloop.poll().await {
                    Ok(Event::Incoming(Packet::ConnAck(_))) => {
                        let _ = client.try_subscribe(&filter, QoS::AtLeastOnce);
                    }
                    Ok(Event::Incoming(Packet::Publish(p))) => {
                        seen.lock().unwrap().push(Seen {
                            topic: p.topic.clone(),
                            payload: String::from_utf8_lossy(&p.payload).to_string(),
                        });
                    }
                    Ok(_) => {}
                    Err(_) => tokio::time::sleep(Duration::from_millis(200)).await,
                }
            }
        });
        Self { log, task }
    }

    fn seen(&self) -> Vec<Seen> {
        self.log.lock().unwrap().clone()
    }

    /// Wait until `done` holds for what was seen so far, up to `max_secs`.
    async fn wait_for(&self, max_secs: u64, done: impl Fn(&[Seen]) -> bool) -> bool {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(max_secs);
        loop {
            if done(&self.seen()) {
                return true;
            }
            if tokio::time::Instant::now() >= deadline {
                return false;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
}

impl Drop for Observer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// The last payload seen on `topic`.
fn last_on<'a>(seen: &'a [Seen], topic: &str) -> Option<&'a str> {
    seen.iter()
        .rev()
        .find(|m| m.topic == topic)
        .map(|m| m.payload.as_str())
}

/// Publish `payload` (not retained) to `topic` with a second client until `rx`
/// yields a command, or `max_secs` elapse. A QoS1 publish that races the bridge's
/// SUBSCRIBE is lost, hence the retry; it makes the test independent of timing.
async fn publish_until_received(
    host: &str,
    port: u16,
    topic: &str,
    payload: &str,
    rx: &mut mpsc::Receiver<ZoneCommand>,
    max_secs: u64,
) -> Option<ZoneCommand> {
    let mut opts = MqttOptions::new("it-t32-publisher", host, port);
    opts.set_keep_alive(Duration::from_secs(5));
    let (publisher, mut pub_loop) = AsyncClient::new(opts, 16);
    let driver = tokio::spawn(async move {
        loop {
            if pub_loop.poll().await.is_err() {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
    });
    let deadline = tokio::time::Instant::now() + Duration::from_secs(max_secs);
    let received = loop {
        publisher
            .publish(topic, QoS::AtLeastOnce, false, payload.as_bytes())
            .await
            .expect("queue publish");
        match tokio::time::timeout_at(
            deadline.min(tokio::time::Instant::now() + Duration::from_millis(300)),
            rx.recv(),
        )
        .await
        {
            Ok(cmd) => break cmd,
            Err(_) if tokio::time::Instant::now() >= deadline => break None,
            Err(_) => {}
        }
    };
    driver.abort();
    received
}

/// Publish `messages` retained (QoS 1) with a throw-away client and wait for the
/// broker's acknowledgements.
async fn seed_retained(host: &str, port: u16, messages: &[(&str, &str)]) {
    let messages: Vec<(&str, Vec<u8>)> = messages
        .iter()
        .map(|(topic, payload)| (*topic, payload.as_bytes().to_vec()))
        .collect();
    seed_retained_bytes(host, port, &messages).await;
}

/// [`seed_retained`] for arbitrary payloads, including ones above rumqttc's 10 KiB
/// default packet limit.
async fn seed_retained_bytes(host: &str, port: u16, messages: &[(&str, Vec<u8>)]) {
    let mut opts = MqttOptions::new("it-t32-seed", host, port);
    opts.set_keep_alive(Duration::from_secs(5));
    opts.set_max_packet_size(4 << 20, 4 << 20);
    let (client, mut eventloop) = AsyncClient::new(opts, 16);
    for (topic, payload) in messages {
        client
            .publish(*topic, QoS::AtLeastOnce, true, payload.clone())
            .await
            .expect("queue seed");
    }
    let mut acked = 0;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    while acked < messages.len() {
        let event = tokio::time::timeout_at(deadline, eventloop.poll())
            .await
            .expect("seed messages were not acknowledged")
            .expect("seed client failed");
        if matches!(event, Event::Incoming(Packet::PubAck(_))) {
            acked += 1;
        }
    }
    client.disconnect().await.expect("disconnect");
    let _ = tokio::time::timeout(Duration::from_secs(2), async {
        while eventloop.poll().await.is_ok() {}
    })
    .await;
}

/// What the broker retains under `filter`, as it hands it to a new subscriber.
async fn retained_under(host: &str, port: u16, filter: &str) -> Vec<Seen> {
    let observer = Observer::start("it-t32-reader", host, port, filter);
    // Retained messages arrive right after the SUBACK; a quiet period ends it.
    tokio::time::sleep(Duration::from_secs(2)).await;
    observer.seen()
}

// ── Tests ─────────────────────────────────────────────────────

/// Availability and retained state are announced after connecting, and the broker
/// publishes the last will when the bridge disappears without a DISCONNECT.
#[tokio::test(flavor = "multi_thread")]
async fn mqtt_tier2_online_state_roundtrip_and_lwt() {
    // Held until the end of the test: dropping it stops the container.
    let Some(broker) = start_broker("IT-T32").await else {
        return;
    };
    let (host, port) = (broker.host.clone(), broker.port);

    let observer = Observer::start("it-t32-sub", &host, port, "snapdog/#");
    let rig = start_bridge(&host, port).await;
    assert!(
        rig.bridge.wait_connected(Duration::from_secs(20)).await,
        "bridge connects"
    );

    // (1) retained "online", (2) the state snapshot of every zone and client.
    let announced = observer
        .wait_for(20, |seen| {
            last_on(seen, "snapdog/status") == Some("online")
                && last_on(seen, "snapdog/zones/1/state").is_some()
                && last_on(seen, "snapdog/clients/1/state").is_some()
        })
        .await;
    assert!(announced, "announcement; saw: {:?}", observer.seen());
    let zone_state = last_on(&observer.seen(), "snapdog/zones/1/state")
        .expect("zone state")
        .to_string();
    let json: serde_json::Value = serde_json::from_str(&zone_state).expect("state is JSON");
    assert_eq!(json["state"], "idle");
    assert_eq!(json["volume_level"], 0.5);

    // (3) Ungraceful disconnect: the connection tasks are aborted, the sockets
    // close without DISCONNECT, and the broker publishes the retained will.
    drop(rig);
    let offline = observer
        .wait_for(20, |seen| {
            last_on(seen, "snapdog/status") == Some("offline")
        })
        .await;
    assert!(
        offline,
        "LWT after ungraceful disconnect; saw: {:?}",
        observer.seen()
    );
}

/// A command published to `<base>zones/1/volume/set` by another client reaches the
/// zone command channel. This is the path that was dead in production: the bridge
/// subscribed to `snapdog/zones/+/volume/set` but then rejected every message
/// because the base topic (normalised to end in `/`) was re-derived by hand.
#[tokio::test(flavor = "multi_thread")]
async fn mqtt_tier2_command_reaches_zone_channel() {
    let Some(broker) = start_broker("IT-T32 command").await else {
        return;
    };
    let (host, port) = (broker.host.clone(), broker.port);
    let mut rig = start_bridge(&host, port).await;
    assert!(rig.bridge.wait_connected(Duration::from_secs(20)).await);

    let cmd = publish_until_received(
        &host,
        port,
        "snapdog/zones/1/volume/set",
        "42",
        &mut rig.zone_rx,
        20,
    )
    .await;
    assert!(
        matches!(cmd, Some(ZoneCommand::SetVolume(42))),
        "command published via the broker must reach the zone channel; got {cmd:?}"
    );
}

/// A retained message on a `/set` topic is replayed by the broker at every
/// subscribe, i.e. on every restart and reconnect. It must not be executed.
#[tokio::test(flavor = "multi_thread")]
async fn mqtt_tier2_retained_command_is_not_executed() {
    let Some(broker) = start_broker("IT-T32 retained").await else {
        return;
    };
    let (host, port) = (broker.host.clone(), broker.port);
    seed_retained(
        &host,
        port,
        &[
            ("snapdog/zones/1/volume/set", "13"),
            ("snapdog/zones/1/control/set", "stop"),
        ],
    )
    .await;

    let mut rig = start_bridge(&host, port).await;
    assert!(rig.bridge.wait_connected(Duration::from_secs(20)).await);
    // The retained messages reach the bridge right after its SUBACK, before any
    // live message. A live command then proves the subscription works, and if it is
    // the first thing on the zone channel, the retained ones were not executed.
    let first = publish_until_received(
        &host,
        port,
        "snapdog/zones/1/volume/set",
        "44",
        &mut rig.zone_rx,
        20,
    )
    .await;
    assert!(
        matches!(first, Some(ZoneCommand::SetVolume(44))),
        "the retained command must not run; first command was {first:?}"
    );
    tokio::time::sleep(Duration::from_millis(500)).await;
    while let Ok(cmd) = rig.zone_rx.try_recv() {
        assert!(
            matches!(cmd, ZoneCommand::SetVolume(44)),
            "only the live command may arrive; got {cmd:?}"
        );
    }
}

/// F1: one retained message above rumqttc's 10 KiB default limit on a command topic
/// used to drop the connection at every subscribe (the broker replays it each
/// time), so MQTT never stayed up and no live command arrived. It must now be
/// received, ignored, and leave the bridge connected and working.
#[tokio::test(flavor = "multi_thread")]
async fn mqtt_tier2_oversized_retained_command_does_not_disable_mqtt() {
    let Some(broker) = start_broker("IT-T32 oversized retained").await else {
        return;
    };
    let (host, port) = (broker.host.clone(), broker.port);
    seed_retained_bytes(
        &host,
        port,
        &[("snapdog/zones/1/volume/set", vec![b'1'; 20_000])],
    )
    .await;

    let mut rig = start_bridge(&host, port).await;
    assert!(rig.bridge.wait_connected(Duration::from_secs(20)).await);
    let cmd = publish_until_received(
        &host,
        port,
        "snapdog/zones/1/volume/set",
        "42",
        &mut rig.zone_rx,
        20,
    )
    .await;
    assert!(
        matches!(cmd, Some(ZoneCommand::SetVolume(42))),
        "a live command must arrive despite the oversized retained one; got {cmd:?}"
    );
    // And it stays up: no reconnect loop in the background.
    for _ in 0..30 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(rig.bridge.is_connected(), "the connection dropped again");
    }
    // The 20,000-byte payload was ignored, never executed.
    assert!(rig.zone_rx.try_recv().is_err(), "nothing else may run");
}

/// F5: a burst of commands for one setting (a dragged slider) loses its
/// intermediate values but not its last one, however full the zone's channel is.
#[tokio::test(flavor = "multi_thread")]
async fn mqtt_tier2_the_last_value_of_a_burst_arrives() {
    let Some(broker) = start_broker("IT-T32 burst").await else {
        return;
    };
    let (host, port) = (broker.host.clone(), broker.port);
    let mut rig = start_bridge(&host, port).await;
    assert!(rig.bridge.wait_connected(Duration::from_secs(20)).await);
    // Warm up: the subscription is live once a command gets through.
    let warm = publish_until_received(
        &host,
        port,
        "snapdog/zones/1/volume/set",
        "10",
        &mut rig.zone_rx,
        20,
    )
    .await;
    assert!(matches!(warm, Some(ZoneCommand::SetVolume(10))), "{warm:?}");

    // 300 values in one go (more than the burst budget and the channel of 16 take,
    // nobody is receiving yet), the last one being 77.
    let mut opts = MqttOptions::new("it-t32-burst", &host, port);
    opts.set_keep_alive(Duration::from_secs(5));
    let (client, mut eventloop) = AsyncClient::new(opts, 512);
    let driver = tokio::spawn(async move {
        loop {
            if eventloop.poll().await.is_err() {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
    });
    for i in 0..299 {
        client
            .publish(
                "snapdog/zones/1/volume/set",
                QoS::AtMostOnce,
                false,
                format!("{}", 20 + i % 50),
            )
            .await
            .expect("queue");
    }
    client
        .publish("snapdog/zones/1/volume/set", QoS::AtMostOnce, false, "77")
        .await
        .expect("queue");
    // Let the whole burst reach the bridge before anything is received.
    tokio::time::sleep(Duration::from_secs(2)).await;
    let mut last = None;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_millis(1500), rig.zone_rx.recv()).await {
            Ok(Some(ZoneCommand::SetVolume(v))) => last = Some(v),
            Ok(Some(_)) => {}
            // Quiet for 1.5 s: everything that was coming has come.
            Ok(None) | Err(_) => break,
        }
    }
    driver.abort();
    assert_eq!(last, Some(77), "the last value of the burst must arrive");
}

/// SnapDog used to publish `media_player` discovery configs, a platform that Home
/// Assistant's MQTT integration does not have. Those retained messages (and the
/// stale double-slash status) must be cleared, and nothing under `homeassistant/`
/// may be published any more.
#[tokio::test(flavor = "multi_thread")]
async fn mqtt_tier2_legacy_retained_messages_are_cleared() {
    let Some(broker) = start_broker("IT-T32 legacy").await else {
        return;
    };
    let (host, port) = (broker.host.clone(), broker.port);
    seed_retained(
        &host,
        port,
        &[
            (
                "homeassistant/media_player/snapdog_zone_1/config",
                r#"{"name":"old"}"#,
            ),
            (
                "homeassistant/media_player/snapdog_zone_12/config",
                r#"{"name":"old, beyond the configured zones"}"#,
            ),
            ("snapdog//status", "online"),
        ],
    )
    .await;
    assert_eq!(
        retained_under(&host, port, "homeassistant/#").await.len(),
        2,
        "the seeded legacy discovery messages are retained before SnapDog starts"
    );

    let rig = start_bridge(&host, port).await;
    assert!(rig.bridge.wait_connected(Duration::from_secs(20)).await);
    tokio::time::sleep(Duration::from_secs(2)).await;

    let ha = retained_under(&host, port, "homeassistant/#").await;
    assert!(ha.is_empty(), "legacy discovery not cleared; saw: {ha:?}");
    let own = retained_under(&host, port, "snapdog/#").await;
    assert!(
        own.iter().all(|m| m.topic != "snapdog//status"),
        "double-slash status not cleared; saw: {own:?}"
    );
    assert_eq!(last_on(&own, "snapdog/status"), Some("online"));
}

/// After the broker restarted (it forgets sessions, subscriptions and retained
/// messages) the bridge reconnects by itself, subscribes again, and publishes
/// availability and state again. Before, it stayed silent until the process was
/// restarted: no subscriptions (commands dead) and no `online`.
#[tokio::test(flavor = "multi_thread")]
async fn mqtt_tier2_recovers_after_broker_restart() {
    let Some(broker) = start_broker_on_fixed_port("IT-T32 restart").await else {
        return;
    };
    let (host, port) = (broker.host.clone(), broker.port);
    let mut rig = start_bridge(&host, port).await;
    assert!(rig.bridge.wait_connected(Duration::from_secs(20)).await);
    let first = publish_until_received(
        &host,
        port,
        "snapdog/zones/1/volume/set",
        "11",
        &mut rig.zone_rx,
        20,
    )
    .await;
    assert!(
        matches!(first, Some(ZoneCommand::SetVolume(11))),
        "{first:?}"
    );

    // Broker restart: retained messages and all subscriptions are gone.
    broker
        .container
        .stop_with_timeout(Some(1))
        .await
        .expect("stop broker");
    tokio::time::timeout(Duration::from_secs(10), async {
        while rig.bridge.is_connected() {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("bridge notices the lost connection");
    tokio::time::sleep(Duration::from_secs(1)).await;
    broker.container.start().await.expect("restart broker");

    // Reconnect (jittered backoff, a few seconds at most), then announce again.
    assert!(
        rig.bridge.wait_connected(Duration::from_secs(60)).await,
        "bridge reconnects after the broker restart"
    );
    let observer = Observer::start("it-t32-after", &host, port, "snapdog/#");
    let reannounced = observer
        .wait_for(30, |seen| {
            last_on(seen, "snapdog/status") == Some("online")
                && last_on(seen, "snapdog/zones/1/state").is_some()
        })
        .await;
    assert!(
        reannounced,
        "re-announced after reconnect; saw: {:?}",
        observer.seen()
    );

    // The subscriptions were re-established too: commands arrive again.
    let again = publish_until_received(
        &host,
        port,
        "snapdog/zones/1/volume/set",
        "22",
        &mut rig.zone_rx,
        30,
    )
    .await;
    assert!(
        matches!(again, Some(ZoneCommand::SetVolume(22))),
        "commands after the restart; got {again:?}"
    );
}

/// A graceful shutdown publishes the retained `offline` itself (a DISCONNECT
/// suppresses the last will) and returns promptly.
#[tokio::test(flavor = "multi_thread")]
async fn mqtt_tier2_graceful_shutdown_publishes_offline() {
    let Some(broker) = start_broker("IT-T32 shutdown").await else {
        return;
    };
    let (host, port) = (broker.host.clone(), broker.port);
    let observer = Observer::start("it-t32-sub", &host, port, "snapdog/#");
    let rig = start_bridge(&host, port).await;
    assert!(rig.bridge.wait_connected(Duration::from_secs(20)).await);
    assert!(
        observer
            .wait_for(20, |seen| last_on(seen, "snapdog/status") == Some("online"))
            .await
    );

    let started = tokio::time::Instant::now();
    rig.bridge.shutdown(Duration::from_secs(2)).await;
    assert!(
        started.elapsed() < Duration::from_millis(1900),
        "shutdown took {:?}",
        started.elapsed()
    );
    let offline = observer
        .wait_for(5, |seen| last_on(seen, "snapdog/status") == Some("offline"))
        .await;
    assert!(offline, "retained offline; saw: {:?}", observer.seen());
}

// ── TLS ───────────────────────────────────────────────────────

/// mosquitto with a plaintext listener (for observers), a TLS listener and a
/// listener that demands a client certificate, all signed by a throwaway CA.
struct TlsBroker {
    _container: ContainerAsync<GenericImage>,
    plain_port: u16,
    tls_port: u16,
    mtls_port: u16,
    /// The CA, a client certificate and key, and an unrelated CA, as files.
    dir: tempfile::TempDir,
}

impl TlsBroker {
    fn file(&self, name: &str) -> String {
        self.dir.path().join(name).display().to_string()
    }
}

const MOSQUITTO_TLS_CONF: &str = "\
persistence false
allow_anonymous true

listener 1883

listener 8883
cafile /mosquitto/config/ca.pem
certfile /mosquitto/config/server.pem
keyfile /mosquitto/config/server.key

listener 8884
cafile /mosquitto/config/ca.pem
certfile /mosquitto/config/server.pem
keyfile /mosquitto/config/server.key
require_certificate true
";

/// Start the TLS mosquitto with a server certificate valid for `server_names`.
async fn start_tls_broker(test: &str, server_names: &[&str]) -> Option<TlsBroker> {
    let pki = test_pki::generate_for(server_names);
    let copy = |path: &str| CopyTargetOptions::new(path).with_mode(0o644);
    let image = mosquitto()
        .with_exposed_port(ContainerPort::Tcp(8883))
        .with_exposed_port(ContainerPort::Tcp(8884))
        .with_copy_to(
            copy("/mosquitto/config/tls.conf"),
            MOSQUITTO_TLS_CONF.as_bytes().to_vec(),
        )
        .with_copy_to(
            copy("/mosquitto/config/ca.pem"),
            pki.ca.clone().into_bytes(),
        )
        .with_copy_to(
            copy("/mosquitto/config/server.pem"),
            pki.server_cert.clone().into_bytes(),
        )
        .with_copy_to(
            copy("/mosquitto/config/server.key"),
            pki.server_key.clone().into_bytes(),
        )
        .with_cmd(["mosquitto", "-c", "/mosquitto/config/tls.conf"]);
    let container = match image.start().await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("SKIP {test}: Docker/mosquitto unavailable: {e}");
            return None;
        }
    };
    let dir = tempfile::tempdir().expect("tempdir");
    for (name, content) in [
        ("ca.pem", pki.ca.as_str()),
        ("other-ca.pem", test_pki::unrelated_ca_pem().as_str()),
        ("client.pem", pki.client_cert.as_str()),
        ("client.key", pki.client_key.as_str()),
    ] {
        std::fs::write(dir.path().join(name), content).expect("write PEM");
    }
    Some(TlsBroker {
        plain_port: container.get_host_port_ipv4(1883).await.expect("1883"),
        tls_port: container.get_host_port_ipv4(8883).await.expect("8883"),
        mtls_port: container.get_host_port_ipv4(8884).await.expect("8884"),
        _container: container,
        dir,
    })
}

/// A bridge on `base` (so that scenarios on one broker do not see each other) with
/// the given `[mqtt]` lines after the broker address.
async fn tls_bridge(broker: &str, base: &str, tls_lines: &str) -> anyhow::Result<Rig> {
    try_start_bridge(&config_with_mqtt(&format!(
        "broker = \"{broker}\"\nbase_topic = \"{base}\"\n{tls_lines}"
    )))
    .await
}

/// Whether the bridge connects within `secs`.
async fn connects(rig: &Rig, secs: u64) -> bool {
    rig.bridge.wait_connected(Duration::from_secs(secs)).await
}

/// TLS: the bridge connects when it trusts the CA, and does not otherwise. A
/// failed handshake is never turned into a plaintext connection.
#[tokio::test(flavor = "multi_thread")]
async fn mqtt_tier2_tls_verifies_the_server() {
    // The server certificate is for `localhost` only.
    let Some(b) = start_tls_broker("IT-T32 TLS", &["localhost"]).await else {
        return;
    };
    let ca = b.file("ca.pem");
    let tls = format!("mqtts://localhost:{}", b.tls_port);
    // Plaintext observer: what the bridge publishes over TLS is visible here.
    let observer = Observer::start("it-t32-tls-obs", "localhost", b.plain_port, "#");

    // Without the CA the system roots do not know the test CA: rejected.
    let rig = tls_bridge(&tls, "tls/no-ca/", "").await.expect("starts");
    assert!(!connects(&rig, 4).await, "an unknown CA must not verify");
    drop(rig);

    // With a CA that did not sign the server certificate: rejected.
    let other = b.file("other-ca.pem");
    let rig = tls_bridge(&tls, "tls/wrong-ca/", &format!("ca_file = {other:?}"))
        .await
        .expect("starts");
    assert!(!connects(&rig, 4).await, "a foreign CA must not verify");
    drop(rig);

    // The right CA but the wrong name: 127.0.0.1 is not in the certificate.
    let by_ip = format!("mqtts://127.0.0.1:{}", b.tls_port);
    let rig = tls_bridge(&by_ip, "tls/wrong-name/", &format!("ca_file = {ca:?}"))
        .await
        .expect("starts");
    assert!(
        !connects(&rig, 4).await,
        "a certificate for another name must not verify"
    );
    drop(rig);

    // TLS to a plaintext listener: no handshake, no connection, and no fallback.
    let to_plain = format!("mqtts://localhost:{}", b.plain_port);
    let rig = tls_bridge(&to_plain, "tls/to-plain/", &format!("ca_file = {ca:?}"))
        .await
        .expect("starts");
    assert!(
        !connects(&rig, 4).await,
        "TLS to a plaintext port must fail"
    );
    drop(rig);

    // Nothing of the failed attempts reached the broker, in plaintext or otherwise.
    assert!(
        observer.seen().iter().all(|m| !m.topic.starts_with("tls/")),
        "failed TLS attempts must publish nothing; saw {:?}",
        observer.seen()
    );

    // The right CA and the right name: connected, and the retained online arrives.
    let rig = tls_bridge(&tls, "tls/ok/", &format!("ca_file = {ca:?}"))
        .await
        .expect("starts");
    assert!(connects(&rig, 20).await, "the right CA must verify");
    assert!(
        observer
            .wait_for(20, |seen| last_on(seen, "tls/ok/status") == Some("online"))
            .await,
        "retained online over TLS; saw {:?}",
        observer.seen()
    );
}

/// A command published in plaintext reaches a bridge that listens over TLS: the
/// transport is the only thing that differs.
#[tokio::test(flavor = "multi_thread")]
async fn mqtt_tier2_tls_carries_commands() {
    let Some(b) = start_tls_broker("IT-T32 TLS command", &["localhost"]).await else {
        return;
    };
    let tls = format!("mqtts://localhost:{}", b.tls_port);
    let mut rig = tls_bridge(
        &tls,
        "snapdog/",
        &format!("ca_file = {:?}", b.file("ca.pem")),
    )
    .await
    .expect("starts");
    assert!(connects(&rig, 20).await);
    let cmd = publish_until_received(
        "localhost",
        b.plain_port,
        "snapdog/zones/1/volume/set",
        "37",
        &mut rig.zone_rx,
        20,
    )
    .await;
    assert!(matches!(cmd, Some(ZoneCommand::SetVolume(37))), "{cmd:?}");
}

/// Mutual TLS: the broker demands a client certificate, and the bridge presents
/// the one it is configured with (or none, and is turned away).
#[tokio::test(flavor = "multi_thread")]
async fn mqtt_tier2_mutual_tls() {
    let Some(b) = start_tls_broker("IT-T32 mTLS", &["localhost"]).await else {
        return;
    };
    let ca = b.file("ca.pem");
    let mtls = format!("mqtts://localhost:{}", b.mtls_port);
    let observer = Observer::start("it-t32-mtls-obs", "localhost", b.plain_port, "#");

    let rig = tls_bridge(&mtls, "mtls/no-cert/", &format!("ca_file = {ca:?}"))
        .await
        .expect("starts");
    assert!(
        !connects(&rig, 4).await,
        "no client certificate: turned away"
    );
    drop(rig);

    let identity = format!(
        "ca_file = {ca:?}\nclient_cert = {:?}\nclient_key = {:?}",
        b.file("client.pem"),
        b.file("client.key")
    );
    let rig = tls_bridge(&mtls, "mtls/ok/", &identity)
        .await
        .expect("starts");
    assert!(connects(&rig, 20).await, "client certificate accepted");
    assert!(
        observer
            .wait_for(20, |seen| last_on(seen, "mtls/ok/status") == Some("online"))
            .await,
        "retained online over mutual TLS; saw {:?}",
        observer.seen()
    );
    assert!(
        observer
            .seen()
            .iter()
            .all(|m| !m.topic.starts_with("mtls/no-cert/")),
        "the rejected attempt published nothing"
    );
}

/// Material that cannot be used stops the bridge from starting. There is no
/// plaintext fallback to start with instead.
#[tokio::test(flavor = "multi_thread")]
async fn mqtt_tier2_unusable_tls_material_stops_the_bridge() {
    let dir = tempfile::tempdir().expect("tempdir");
    let bogus = dir.path().join("bogus.pem");
    std::fs::write(
        &bogus,
        "-----BEGIN CERTIFICATE-----\nbm90IGEgY2VydGlmaWNhdGU=\n-----END CERTIFICATE-----\n",
    )
    .expect("write");
    let section = format!(
        "broker = \"mqtts://localhost:8883\"\nca_file = {:?}",
        bogus.display().to_string()
    );

    // The config loader sees it (it looks like PEM, but the TLS library refuses it):
    // MQTT is switched off, with the reason, and the rest of the config loads.
    let cfg = config_with_mqtt(&section);
    assert!(cfg.mqtt.is_none(), "MQTT is off");
    let problem = cfg.mqtt_problem.expect("and says why");
    assert!(problem.contains("ca_file"), "{problem}");

    // A caller that skips the loader gets the same refusal from the bridge.
    let mut cfg = config_with_mqtt("broker = \"localhost:1883\"");
    cfg.mqtt = Some(toml::from_str(&section).expect("parses"));
    let Err(err) = try_start_bridge(&cfg).await else {
        panic!("a bridge must not start with unusable TLS material");
    };
    let msg = format!("{err:#}");
    assert!(msg.contains("ca_file"), "{msg}");

    // TLS options on a plaintext address are refused at load, too.
    let cfg = config_with_mqtt("broker = \"localhost:1883\"\nca_file = \"/nonexistent/ca.pem\"");
    assert!(cfg.mqtt.is_none());
    let problem = cfg.mqtt_problem.expect("and says why");
    assert!(
        problem.contains("plaintext"),
        "TLS options on a plaintext address: {problem}"
    );
}
