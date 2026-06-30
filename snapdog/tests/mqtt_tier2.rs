// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! IT-T32 (tier-2) — MqttBridge against a real mosquitto broker via testcontainers:
//! retained "online", a QoS1 retained zone-state round-trip, and the LWT "offline"
//! on an ungraceful disconnect. LOUD-SKIPs (no panic) when Docker is unavailable.
//!
//! Run: `cargo test -p snapdog --test mqtt_tier2 -- --test-threads=1`

#![allow(clippy::doc_markdown)] // doc mentions MqttBridge / QoS1 / AppConfig / load_raw

use std::collections::HashMap;
use std::time::Duration;

use rumqttc::{AsyncClient, Event, MqttOptions, Packet, QoS};
use snapdog::mqtt::MqttBridge;
use snapdog::{config, state};
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::mosquitto::Mosquitto;

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
    let container = match Mosquitto::default().start().await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("SKIP IT-T32: Docker/mosquitto unavailable: {e}");
            return;
        }
    };
    let host = container.get_host().await.expect("host").to_string();
    let port = container
        .get_host_port_ipv4(1883)
        .await
        .expect("mapped port");

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

    // rumqttc's EventLoop is not Send, so we can't spawn run() — instead drive the
    // bridge's loop (CONNECT registers the LWT + flushes the queued publishes) and
    // the subscriber's loop together in one select.
    let cmds: HashMap<usize, snapdog::player::ZoneCommandSender> = HashMap::new();
    let (snap_tx, _snap_rx) = tokio::sync::mpsc::channel(16);

    // (1) retained "online" + (2) QoS1 retained zone-state round-trip.
    let mut seen: HashMap<String, String> = HashMap::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(25);
    while !(seen.get("snapdog/status").map(String::as_str) == Some("online")
        && seen.contains_key("snapdog/zones/1/state"))
    {
        tokio::select! {
            () = bridge.poll_once(&cmds, &store, &snap_tx) => {}
            r = tokio::time::timeout_at(deadline, sub_loop.poll()) => {
                match r {
                    Ok(Ok(Event::Incoming(Packet::Publish(p)))) => {
                        seen.insert(p.topic.clone(), String::from_utf8_lossy(&p.payload).to_string());
                    }
                    Ok(_) => {}
                    Err(_) => break, // deadline
                }
            }
        }
    }
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
