// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! IT-T32 (tier-2) — MqttBridge against a real mosquitto broker via testcontainers:
//! retained "online", a QoS1 retained zone-state round-trip, the LWT "offline"
//! on an ungraceful disconnect, and a command published by a second client that
//! must reach the zone command channel. LOUD-SKIPs (no panic) when Docker is
//! unavailable (on macOS with colima: `DOCKER_HOST=unix://$HOME/.colima/default/docker.sock`).
//!
//! Run: `cargo test -p snapdog --test mqtt_tier2 -- --test-threads=1`

#![allow(clippy::doc_markdown)] // doc mentions MqttBridge / QoS1 / AppConfig / load_raw

use std::collections::HashMap;
use std::time::Duration;

use rumqttc::{AsyncClient, Event, MqttOptions, Packet, QoS};
use snapdog::mqtt::MqttBridge;
use snapdog::player::ZoneCommand;
use snapdog::{config, state};
use testcontainers::{ContainerAsync, GenericImage, ImageExt, core::WaitFor, runners::AsyncRunner};
use tokio::sync::mpsc;

/// A running anonymous mosquitto (the container lives as long as this value).
struct Broker {
    _container: ContainerAsync<GenericImage>,
    host: String,
    port: u16,
}

/// Start mosquitto, or `None` (after a loud SKIP line) when Docker is unavailable.
async fn start_broker(test: &str) -> Option<Broker> {
    // Keep the same broker and readiness condition as the former Mosquitto module.
    // GenericImage lets this test track current testcontainers independently.
    let image = GenericImage::new("eclipse-mosquitto", "2.0.18")
        .with_wait_for(WaitFor::message_on_stderr(
            "mosquitto version 2.0.18 running",
        ))
        .with_cmd(["mosquitto", "-c", "/mosquitto-no-auth.conf"]);
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
        _container: container,
        host,
        port,
    })
}

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

/// Drive the subscriber event loop, recording the latest payload per topic, until
/// `done` is satisfied or `max_secs` elapses.
async fn collect_until(
    eventloop: &mut rumqttc::EventLoop,
    max_secs: u64,
    done: impl Fn(&HashMap<String, String>) -> bool,
) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(max_secs);
    loop {
        match tokio::time::timeout_at(deadline, eventloop.poll()).await {
            Ok(Ok(Event::Incoming(Packet::Publish(p)))) => {
                map.insert(
                    p.topic.clone(),
                    String::from_utf8_lossy(&p.payload).to_string(),
                );
                if done(&map) {
                    break;
                }
            }
            Ok(Ok(_)) => {}
            Ok(Err(_)) => tokio::time::sleep(Duration::from_millis(200)).await,
            Err(_) => break, // deadline
        }
    }
    map
}

#[tokio::test(flavor = "multi_thread")]
async fn mqtt_tier2_online_state_roundtrip_and_lwt() {
    // Held until the end of the test: dropping it stops the container.
    let Some(broker) = start_broker("IT-T32").await else {
        return;
    };
    let (host, port) = (broker.host.clone(), broker.port);
    let cfg = config_for_broker(&host, port);
    let mqtt_cfg = cfg.mqtt.clone().expect("mqtt config present");
    let store = state::init(&cfg, None).expect("state init");
    let zone1 = store.read().await.zones.get(&1).unwrap().clone();

    // Bridge: queue the retained "online" + a QoS1 retained zone state (rumqttc
    // enqueues these; they flush once the event loop is polled below).
    let mut bridge = MqttBridge::connect(&mqtt_cfg, "http://test:5555", "SnapDog")
        .await
        .expect("bridge connect");
    bridge.publish_ha_discovery(&[]).await.expect("online");
    bridge
        .publish_zone_state(1, &zone1)
        .await
        .expect("zone state");

    // Independent subscriber — connects + subscribes so retained delivery and the
    // later LWT update are both observed.
    let mut sub_opts = MqttOptions::new("it-t32-sub", host.clone(), port);
    sub_opts.set_keep_alive(Duration::from_secs(5));
    let (sub, mut sub_loop) = AsyncClient::new(sub_opts, 64);
    sub.subscribe("snapdog/#", QoS::AtLeastOnce)
        .await
        .expect("subscribe");

    // Keep both event-loop futures alive until collection finishes. Recreating a
    // select around poll_once cancels an in-flight CONNECT whenever the subscriber
    // wins; dropping that socket can fire the LWT before "online" is flushed.
    // A biased select does not fix cancellation of a pending branch.
    let cmds: HashMap<usize, snapdog::player::ZoneCommandSender> = HashMap::new();
    let (snap_tx, _snap_rx) = tokio::sync::mpsc::channel(16);

    // (1) retained "online" + (2) QoS1 retained zone-state round-trip.
    let seen = tokio::select! {
        result = bridge.run(cmds, store, snap_tx) => {
            panic!("bridge event loop stopped unexpectedly: {result:?}");
        }
        seen = collect_until(&mut sub_loop, 25, |m| {
            m.get("snapdog/status").map(String::as_str) == Some("online")
                && m.contains_key("snapdog/zones/1/state")
        }) => seen,
    };
    assert_eq!(
        seen.get("snapdog/status").map(String::as_str),
        Some("online"),
        "retained online; saw: {seen:?}"
    );
    assert!(
        seen.contains_key("snapdog/zones/1/state"),
        "retained QoS1 zone state round-trip; saw: {seen:?}"
    );

    // (3) Ungraceful disconnect: drop the bridge (event loop dropped, no clean
    // DISCONNECT) → broker publishes the retained LWT "offline".
    drop(bridge);
    let after = collect_until(&mut sub_loop, 20, |m| {
        m.get("snapdog/status").map(String::as_str) == Some("offline")
    })
    .await;
    assert_eq!(
        after.get("snapdog/status").map(String::as_str),
        Some("offline"),
        "LWT offline after ungraceful disconnect; saw: {after:?}"
    );
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

    let cfg = config_for_broker(&host, port);
    let mqtt_cfg = cfg.mqtt.clone().expect("mqtt config present");
    assert_eq!(
        mqtt_cfg.base_topic, "snapdog/",
        "config normalises the base"
    );
    let store = state::init(&cfg, None).expect("state init");

    let mut bridge = MqttBridge::connect(&mqtt_cfg, "http://test:5555", "SnapDog")
        .await
        .expect("bridge connect");
    bridge.subscribe_commands().await.expect("subscribe");

    let (zone_tx, mut zone_rx) = mpsc::channel(16);
    let cmds: HashMap<usize, snapdog::player::ZoneCommandSender> = HashMap::from([(1, zone_tx)]);
    let (snap_tx, _snap_rx) = mpsc::channel(16);
    // The bridge's own event loop and the publishing client run side by side. The
    // bridge future is never cancelled until the command has arrived.
    let cmd = tokio::select! {
        result = bridge.run(cmds, store, snap_tx) => {
            panic!("bridge event loop stopped unexpectedly: {result:?}");
        }
        cmd = publish_until_received(
            &host,
            port,
            "snapdog/zones/1/volume/set",
            "42",
            &mut zone_rx,
            20,
        ) => cmd,
    };
    assert!(
        matches!(cmd, Some(ZoneCommand::SetVolume(42))),
        "command published via the broker must reach the zone channel; got {cmd:?}"
    );
}
