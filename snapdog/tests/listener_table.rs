// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! The listener endpoint and startup table retain every bound HTTP socket (#355).

mod common;

use axum::http::StatusCode;
use snapdog::{api, net::listener};

#[tokio::test]
async fn the_api_and_log_report_every_bound_http_socket() {
    let mut config = common::test_config();
    // Two ephemeral listeners on loopback avoid requiring IPv6 or another assigned
    // address. Their distinct OS-assigned ports must both survive the table merge.
    config.http.bind = vec!["127.0.0.1".into(), "127.0.0.1".into()];
    config.http.port = 0;
    let sockets = api::bind_http_listeners(&config.http.bind, 0)
        .await
        .expect("both HTTP sockets bind");
    let expected: Vec<_> = sockets
        .iter()
        .map(|socket| socket.local_addr().unwrap())
        .collect();
    assert_ne!(expected[0], expected[1]);

    let rows = listener::table(&config);
    let http: Vec<_> = rows
        .iter()
        .filter(|row| row.service == "HTTP API")
        .collect();
    assert_eq!(http.len(), 2, "no HTTP listener may disappear: {http:?}");
    for (row, address) in http.iter().zip(&expected) {
        assert_eq!(row.address, address.ip().to_string());
        assert_eq!(row.port, address.port());
        assert_eq!(row.bind_support, listener::BindSupport::Full);
    }

    let app = common::build_test_app(config);
    let (status, body) = app.get("/api/v1/system/listeners").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, serde_json::to_value(&rows).unwrap());

    let log = listener::format_table(&rows);
    assert_eq!(
        log.lines().filter(|line| line.contains("HTTP API")).count(),
        2
    );
    for address in expected {
        assert!(
            log.contains(&format!("{}:{}", address.ip(), address.port())),
            "{log}"
        );
    }
}
