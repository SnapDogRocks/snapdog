// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! IT-T84 — headless boot / serve lifecycle.
//!
//! - Tier-1 floor: an in-process `/health` oneshot (no socket).
//! - Tier-2 lifecycle: the REAL `api::serve` over a loopback ephemeral port (no
//!   fixed-port collision across parallel runs), driven to graceful shutdown via a
//!   cooperative future — asserts the health endpoint is reachable over the socket,
//!   `serve` returns `Ok` on shutdown, and the listener is then closed.

mod common;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::http::StatusCode;
use common::{test_app, test_config};
use snapdog::api;
use snapdog::audio::eq::EqStore;
use snapdog::player::{SnapcastCmd, ZoneCommandSender};
use snapdog::state;
use tokio::net::TcpListener;
use tokio::sync::{mpsc, oneshot};

#[tokio::test]
async fn health_endpoint_in_process() {
    let app = test_app(); // 2 zones / 2 clients
    let (status, body) = app.get("/health").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "ok");
    assert_eq!(body["zones"], 2);
    assert_eq!(body["clients"], 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn serve_health_over_socket_then_graceful_shutdown() {
    let config = test_config();
    let store = state::init(&config, None).unwrap();
    let covers = state::cover::new_cache();
    let (notify_tx, _notify_rx) = api::ws::notification_channel();
    let (snap_tx, _snap_rx) = mpsc::channel::<SnapcastCmd>(64);
    let zone_commands: HashMap<usize, ZoneCommandSender> = HashMap::new();
    let tmp = tempfile::tempdir().unwrap();
    let eq_store = Arc::new(Mutex::new(EqStore::load(&tmp.path().join("eq.json"))));

    // Ephemeral loopback port — no fixed-port collision across parallel runs.
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    let handle = tokio::spawn(api::serve(
        listener,
        config,
        store,
        zone_commands,
        snap_tx,
        covers,
        notify_tx,
        eq_store,
        None,
        async move {
            let _ = shutdown_rx.await;
        },
    ));

    // Health reachable over the real socket (.text() avoids reqwest's json feature).
    let url = format!("http://{addr}/health");
    let resp = reqwest::Client::new()
        .get(&url)
        .timeout(Duration::from_secs(5))
        .send()
        .await
        .expect("health reachable");
    assert_eq!(resp.status().as_u16(), 200);
    let body: serde_json::Value = serde_json::from_str(&resp.text().await.unwrap()).unwrap();
    assert_eq!(body["status"], "ok");
    assert_eq!(body["zones"], 2);

    // Trigger graceful shutdown and await clean exit.
    shutdown_tx.send(()).unwrap();
    let res = tokio::time::timeout(Duration::from_secs(5), handle)
        .await
        .expect("serve stops within 5s")
        .expect("serve task did not panic");
    assert!(res.is_ok(), "serve returned Ok after graceful shutdown");

    // The listener is closed — a fresh request must now fail.
    let after = reqwest::Client::new()
        .get(&url)
        .timeout(Duration::from_secs(1))
        .send()
        .await;
    assert!(after.is_err(), "listener should be closed after shutdown");
}
