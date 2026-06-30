// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! IT-T21 — WebSocket keepalive ping + connection-limit (503). Driven over a real
//! loopback socket because the WS upgrade can't go through `tower::oneshot`.
//! `ACTIVE_CONNECTIONS` is a process-global, so this is the sole test in its binary
//! (the ping-cadence test lives in `ws_ping.rs`, a separate binary).

mod common;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use futures_util::StreamExt;
use snapdog::api;
use snapdog::audio::eq::EqStore;
use snapdog::player::{SnapcastCmd, ZoneCommand};
use snapdog::state;
use tokio::net::TcpListener;
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::{Error as WsError, Message};

/// Bind an ephemeral loopback port and serve the real API (incl. `/ws`).
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

#[tokio::test(flavor = "multi_thread")]
async fn ws_pings_on_connect_and_enforces_connection_limit() {
    let (addr, _sd, _tmp) = spawn_ws_server().await;
    let url = format!("ws://{addr}/ws");

    // First connection: the ping interval's immediate first tick → a Ping frame.
    let (mut first, _) = connect_async(url.as_str()).await.expect("connect 1");
    assert!(
        matches!(first.next().await, Some(Ok(Message::Ping(_)))),
        "keepalive ping on connect"
    );

    // Fill to the 64-connection cap (first is #1; open 63 more). Awaiting each
    // socket's first Ping is the barrier proving its post-upgrade fetch_add ran.
    let mut conns = vec![first];
    for i in 0..63 {
        let (mut c, _) = connect_async(url.as_str())
            .await
            .unwrap_or_else(|e| panic!("connect {}: {e}", i + 2));
        assert!(matches!(c.next().await, Some(Ok(Message::Ping(_)))));
        conns.push(c);
    }

    // 65th is rejected: the handshake fails with HTTP 503 before the upgrade.
    let err = connect_async(url.as_str())
        .await
        .expect_err("65th connection rejected");
    match err {
        WsError::Http(resp) => {
            assert_eq!(
                resp.status().as_u16(),
                503,
                "65th → 503 Service Unavailable"
            );
        }
        other => panic!("expected HTTP 503, got {other:?}"),
    }

    drop(conns);
}
