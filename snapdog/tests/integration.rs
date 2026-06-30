// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! Tier-2 integration tests against real external services (Subsonic + MQTT).
//!
//! RFC IT-0003 tier-2: each test reads credentials from `.env.test` and loud-skips
//! when they're absent. Run with:
//! `cargo test --test integration --no-default-features --features snapcast-process -- --test-threads=1`.
//!
//! Real-snapserver end-to-end (control + per-zone TCP audio source) is owned by
//! `IT-T56`. The earlier in-process snapserver test bodies had drifted far past the
//! post-ADR-018 API (`init()`/`state()` removal, unified-playlist commands, sync
//! `SnapserverHandle`, the new `serve`/`MqttBridge` signatures, …) and were carried
//! as a disabled `#[cfg(any())]` block — they're removed here (`IT-T07`) rather than
//! kept as dead code; that coverage belongs to `IT-T56`.
#![cfg(feature = "snapcast-process")]

use snapdog::config;

// ── Subsonic tests (conditional) ──────────────────────────────

fn subsonic_config() -> Option<config::SubsonicConfig> {
    let _ = dotenvy::from_filename(".env.test");
    let url = std::env::var("SNAPDOG_TEST_SUBSONIC_URL").ok()?;
    let username = std::env::var("SNAPDOG_TEST_SUBSONIC_USERNAME").ok()?;
    let password = std::env::var("SNAPDOG_TEST_SUBSONIC_PASSWORD").ok()?;
    if url.is_empty() || username.is_empty() {
        return None;
    }
    Some(config::SubsonicConfig {
        url,
        username,
        password: password.into(),
        format: config::SubsonicFormat::Flac,
        tls_skip_verify: false,
        cache: config::SubsonicCacheConfig::default(),
    })
}

#[tokio::test]
async fn subsonic_ping() {
    let Some(cfg) = subsonic_config() else {
        eprintln!("Skipping — no credentials in .env.test");
        return;
    };
    let client = snapdog::subsonic::SubsonicClient::new(&cfg);
    if let Err(e) = client.ping().await {
        eprintln!("Skipping — Subsonic not reachable: {e}");
    }
}

#[tokio::test]
async fn subsonic_playlists_not_empty() {
    let Some(cfg) = subsonic_config() else {
        eprintln!("Skipping — no credentials in .env.test");
        return;
    };
    let client = snapdog::subsonic::SubsonicClient::new(&cfg);
    // Skip if the server isn't reachable (e.g. a fresh Navidrome without setup).
    if client.ping().await.is_err() {
        eprintln!("Skipping — Subsonic not reachable");
        return;
    }
    let playlists = client
        .get_playlists()
        .await
        .expect("Should fetch playlists");
    assert!(!playlists.is_empty(), "Should have at least one playlist");
}

// ── MQTT tests (conditional) ──────────────────────────────────

fn mqtt_config() -> Option<config::MqttConfig> {
    let _ = dotenvy::from_filename(".env.test");
    let broker = std::env::var("SNAPDOG_TEST_MQTT_BROKER").ok()?;
    let username = std::env::var("SNAPDOG_TEST_MQTT_USERNAME").ok()?;
    let password = std::env::var("SNAPDOG_TEST_MQTT_PASSWORD").ok()?;
    if broker.is_empty() {
        return None;
    }
    Some(config::MqttConfig {
        broker,
        client_id: "snapdog-test".to_string(),
        username,
        password: password.into(),
        base_topic: "snapdog/test".to_string(),
    })
}

#[tokio::test]
async fn mqtt_connect_and_subscribe() {
    let Some(cfg) = mqtt_config() else {
        eprintln!("Skipping — no MQTT credentials in .env.test");
        return;
    };
    let bridge = snapdog::mqtt::MqttBridge::connect(&cfg, "http://localhost:5555", "SnapDog")
        .await
        .expect("MQTT connect should succeed");
    bridge
        .subscribe_commands()
        .await
        .expect("MQTT subscribe should succeed");
}

#[tokio::test]
async fn mqtt_publish_and_receive() {
    let Some(cfg) = mqtt_config() else {
        eprintln!("Skipping — no MQTT credentials in .env.test");
        return;
    };
    let bridge = snapdog::mqtt::MqttBridge::connect(&cfg, "http://localhost:5555", "SnapDog")
        .await
        .expect("MQTT connect should succeed");
    bridge
        .publish("test/ping", "pong")
        .await
        .expect("MQTT publish should succeed");
}
