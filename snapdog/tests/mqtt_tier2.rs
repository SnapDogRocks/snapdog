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
    ContainerAsync, GenericImage, ImageExt,
    core::{ContainerPort, WaitFor},
    runners::AsyncRunner,
};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

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

/// AppConfig (via TOML → load_raw) with `[mqtt]` pointed at `host:port`.
fn config_for_broker(host: &str, port: u16) -> config::AppConfig {
    let toml = format!(
        r#"
[mqtt]
broker = "{host}:{port}"
username = ""
password = ""
base_topic = "snapdog/"

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

/// A started bridge and the receiving ends of the channels it commands.
struct Rig {
    bridge: MqttBridge,
    zone_rx: mpsc::Receiver<ZoneCommand>,
    _snap_rx: mpsc::Receiver<snapdog::player::SnapcastCmd>,
    _notify_tx: snapdog::api::ws::NotifySender,
}

async fn start_bridge(host: &str, port: u16) -> Rig {
    let cfg = config_for_broker(host, port);
    let mqtt_cfg = cfg.mqtt.clone().expect("mqtt config present");
    assert_eq!(
        mqtt_cfg.base_topic, "snapdog/",
        "config normalises the base"
    );
    let store = state::init(&cfg, None).expect("state init");

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
    .await
    .expect("bridge starts");
    Rig {
        bridge,
        zone_rx,
        _snap_rx: snap_rx,
        _notify_tx: notify_tx,
    }
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
    let mut opts = MqttOptions::new("it-t32-seed", host, port);
    opts.set_keep_alive(Duration::from_secs(5));
    let (client, mut eventloop) = AsyncClient::new(opts, 16);
    for (topic, payload) in messages {
        client
            .publish(*topic, QoS::AtLeastOnce, true, payload.as_bytes())
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
