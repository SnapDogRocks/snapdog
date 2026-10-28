// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! A WebSocket client that cannot keep up with the notification buffer is closed
//! with its own code (1013, "try again later"), not with the shutdown code 1001.
//! Own binary: the connection counter in `ws.rs` is process-global.

mod common;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::StreamExt;
use snapdog::api;
use snapdog::audio::eq::EqStore;
use snapdog::player::{SnapcastCmd, ZoneCommand};
use snapdog::state;
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;

#[tokio::test(flavor = "multi_thread")]
async fn a_client_that_falls_behind_is_closed_with_try_again_later() {
    let config = common::test_config();
    let store = state::init(&config, None).unwrap();
    let covers = state::cover::new_cache();
    let (notify_tx, _rx) = api::ws::notification_channel();
    let publisher = notify_tx.clone();
    let (snap_tx, _snap_rx) = mpsc::channel::<SnapcastCmd>(64);
    let zone_commands: HashMap<usize, mpsc::Sender<ZoneCommand>> = HashMap::new();
    let tmp = tempfile::tempdir().unwrap();
    let eq_store = Arc::new(Mutex::new(EqStore::load(&tmp.path().join("eq.json"))));
    let (listener, addr) = common::bind_ephemeral().await;
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

    let (mut ws, _) = connect_async(format!("ws://{addr}/ws").as_str())
        .await
        .expect("connect");
    // The first Ping proves the server task is subscribed to the broadcast.
    assert!(matches!(ws.next().await, Some(Ok(Message::Ping(_)))));

    // Flood the buffer without reading. Big messages fill the socket buffers, so the
    // server blocks in its send and the broadcast ring (256 slots) overruns for it.
    let payload: Arc<str> = "x".repeat(64 * 1024).into();
    for _ in 0..2_000 {
        let _ = publisher.send(payload.clone());
        tokio::task::yield_now().await;
    }

    // Now read: some messages arrive, then the server's close frame.
    let close = tokio::time::timeout(Duration::from_secs(30), async {
        let mut received = 0usize;
        while let Some(message) = ws.next().await {
            match message.expect("read") {
                Message::Text(_) => received += 1,
                Message::Close(frame) => return (received, frame),
                _ => {}
            }
        }
        (received, None)
    })
    .await
    .expect("the server closes the lagging client");

    let (received, frame) = close;
    let frame = frame.expect("a close frame with code and reason");
    assert_eq!(
        frame.code,
        CloseCode::Again,
        "1013, not Away (1001): {frame:?}"
    );
    assert!(frame.reason.contains("too slow"), "{frame:?}");
    assert!(
        received < 2_000,
        "the client cannot have received every message"
    );

    let _ = sd_tx.send(());
}
