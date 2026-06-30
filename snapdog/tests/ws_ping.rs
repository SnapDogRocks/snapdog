// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! IT-T21 — WebSocket ping cadence (`WS_PING_INTERVAL = 30s`) on a paused clock.
//! Separate binary from `ws_lifecycle.rs` so the process-global `ACTIVE_CONNECTIONS`
//! is isolated (a single connection here).

mod common;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::StreamExt;
use snapdog::api;
use snapdog::audio::eq::EqStore;
use snapdog::player::{SnapcastCmd, ZoneCommand};
use snapdog::state;
use tokio::net::TcpListener;
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

async fn spawn_ws_server() -> (SocketAddr, oneshot::Sender<()>, tempfile::TempDir) {
    let config = common::test_config();
    let store = state::init(&config, None).unwrap();
    let covers = state::cover::new_cache();
    let (notify_tx, _rx) = api::ws::notification_channel();
    let (snap_tx, _snap_rx) = mpsc::channel::<SnapcastCmd>(64);
    let zone_commands: HashMap<usize, mpsc::Sender<ZoneCommand>> = HashMap::new();
    let tmp = tempfile::tempdir().unwrap();
    let eq_store = Arc::new(Mutex::new(EqStore::load(&tmp.path().join("eq.json"))));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (sd_tx, sd_rx) = oneshot::channel::<()>();
    tokio::spawn(api::serve(
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
            let _ = sd_rx.await;
        },
    ));
    (addr, sd_tx, tmp)
}

#[tokio::test(start_paused = true)]
async fn ws_ping_fires_each_interval() {
    let (addr, _sd, _tmp) = spawn_ws_server().await;
    let (mut ws, _) = connect_async(format!("ws://{addr}/ws").as_str())
        .await
        .expect("connect");

    // The interval's first tick is immediate → a Ping right after the handshake.
    assert!(
        matches!(ws.next().await, Some(Ok(Message::Ping(_)))),
        "ping at t=0"
    );

    // Advance one full interval (clock paused) → exactly one more Ping.
    tokio::time::advance(Duration::from_secs(30)).await;
    assert!(
        matches!(ws.next().await, Some(Ok(Message::Ping(_)))),
        "ping at t=30s"
    );
}
