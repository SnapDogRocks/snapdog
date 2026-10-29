// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! With TLS enabled the server binds its address once and answers over HTTPS (issue 343).
//!
//! `main` binds the listener before anything else starts and hands it to `api::serve`.
//! The TLS path used to bind the same address a second time, which fails with "address
//! in use" because the first listener is still open, so HTTPS never came up.

mod common;

#[path = "../src/mqtt/test_pki.rs"]
mod test_pki;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use snapdog::api;
use snapdog::audio::eq::EqStore;
use snapdog::net::listener;
use snapdog::player::{SnapcastCmd, ZoneCommand};
use snapdog::state;
use tokio::sync::{mpsc, oneshot};

#[tokio::test(flavor = "multi_thread")]
async fn the_tls_server_listens_once_and_answers_over_https() {
    api::install_crypto_provider();

    let tmp = tempfile::tempdir().unwrap();
    // A throwaway CA and a server certificate for `localhost`: the client trusts that CA,
    // so the certificate is verified like a real one.
    let pki = test_pki::generate();
    let cert_path = tmp.path().join("cert.pem");
    let key_path = tmp.path().join("key.pem");
    std::fs::write(&cert_path, &pki.server_cert).unwrap();
    std::fs::write(&key_path, &pki.server_key).unwrap();

    let mut config = common::test_config();
    config.http.tls_cert = Some(cert_path);
    config.http.tls_key = Some(key_path);

    let store = state::init(&config, None).unwrap();
    let (notify_tx, _rx) = api::ws::notification_channel();
    let (snap_tx, _snap_rx) = mpsc::channel::<SnapcastCmd>(64);
    let zone_commands: HashMap<usize, mpsc::Sender<ZoneCommand>> = HashMap::new();
    let eq_store = Arc::new(Mutex::new(EqStore::load(&tmp.path().join("eq.json"))));

    let tcp = api::bind_http_listener("127.0.0.1", 0)
        .await
        .expect("bind the HTTP listener");
    let addr = tcp.local_addr().unwrap();

    let (sd_tx, sd_rx) = oneshot::channel::<()>();
    let server = tokio::spawn(api::serve(
        tcp,
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

    let client = reqwest::Client::builder()
        .add_root_certificate(reqwest::Certificate::from_pem(pki.ca.as_bytes()).unwrap())
        .build()
        .unwrap();
    let url = format!("https://localhost:{}/health", addr.port());
    let mut answer = None;
    for _ in 0..50 {
        if server.is_finished() {
            break;
        }
        match client.get(&url).send().await {
            Ok(response) => {
                answer = Some(response.status());
                break;
            }
            Err(_) => tokio::time::sleep(Duration::from_millis(100)).await,
        }
    }

    assert!(
        !server.is_finished(),
        "the TLS server stopped: {:?}",
        server.await
    );
    assert_eq!(answer, Some(reqwest::StatusCode::OK), "HTTPS must answer");

    let mine: Vec<_> = listener::bound()
        .into_iter()
        .filter(|l| l.service == "HTTP API" && l.addr == addr)
        .collect();
    assert_eq!(mine.len(), 1, "the address is bound once: {mine:?}");

    drop(client);
    let _ = sd_tx.send(());
    let result = tokio::time::timeout(Duration::from_secs(15), server)
        .await
        .expect("the server stops")
        .expect("the server task does not panic");
    assert!(result.is_ok(), "{result:?}");
}
