// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! `[http].bind` with several addresses: one server answers on every listener and stops
//! with all of them (RFC LBD-0008, M2).

mod common;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use snapdog::api;
use snapdog::audio::eq::EqStore;
use snapdog::player::{SnapcastCmd, ZoneCommand};
use snapdog::state;
use tokio::sync::{mpsc, oneshot};

#[tokio::test(flavor = "multi_thread")]
async fn one_server_answers_on_every_bound_address() {
    let tmp = tempfile::tempdir().unwrap();
    let config = common::test_config();
    let store = state::init(&config, None).unwrap();
    let (notify_tx, _rx) = api::ws::notification_channel();
    let (snap_tx, _snap_rx) = mpsc::channel::<SnapcastCmd>(64);
    let zone_commands: HashMap<usize, mpsc::Sender<ZoneCommand>> = HashMap::new();
    let eq_store = Arc::new(Mutex::new(EqStore::load(&tmp.path().join("eq.json"))));

    // Two addresses of the loopback range, each on its own ephemeral port.
    let listeners = api::bind_http_listeners(&["127.0.0.1".to_owned(), "127.0.0.1".to_owned()], 0)
        .await
        .expect("bind both addresses");
    let ports: Vec<u16> = listeners
        .iter()
        .map(|l| l.local_addr().unwrap().port())
        .collect();
    assert_eq!(ports.len(), 2);
    assert_ne!(ports[0], ports[1]);

    let (sd_tx, sd_rx) = oneshot::channel::<()>();
    let server = tokio::spawn(api::serve_on(
        listeners,
        config,
        store,
        zone_commands,
        snap_tx,
        state::cover::new_cache(),
        notify_tx,
        eq_store,
        None,
        async move {
            let _ = sd_rx.await;
        },
    ));

    let client = reqwest::Client::new();
    for port in &ports {
        let url = format!("http://127.0.0.1:{port}/health");
        let mut status = None;
        for _ in 0..50 {
            if let Ok(response) = client.get(&url).send().await {
                status = Some(response.status());
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert_eq!(status, Some(reqwest::StatusCode::OK), "{url}");
    }
    assert!(!server.is_finished(), "{:?}", server.await);

    drop(client);
    let _ = sd_tx.send(());
    let result = tokio::time::timeout(Duration::from_secs(15), server)
        .await
        .expect("both listeners stop on the one shutdown")
        .expect("the server task does not panic");
    assert!(result.is_ok(), "{result:?}");
}

#[tokio::test]
async fn a_bind_that_fails_names_the_address_and_the_error() {
    let taken =
        snapdog::net::listener::bind_tcp("test", "std", "127.0.0.1:0".parse().unwrap()).unwrap();
    let port = taken.local_addr().unwrap().port();
    let error = api::bind_http_listeners(&["127.0.0.1".to_owned(), "127.0.0.2".to_owned()], port)
        .await
        .unwrap_err();
    assert_eq!(error.0, "127.0.0.1");
    assert_eq!(error.1.kind(), std::io::ErrorKind::AddrInUse);
}
