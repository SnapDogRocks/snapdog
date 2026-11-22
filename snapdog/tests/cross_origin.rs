// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! A web page on another origin can neither drive the API nor open the WebSocket, also on a
//! server without an API key. Clients that send no `Origin` are not affected.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use snapdog::api;
use tower::ServiceExt;

async fn status(
    app: &common::TestApp,
    method: &str,
    uri: &str,
    origin: Option<&str>,
) -> StatusCode {
    let mut req = Request::builder()
        .method(method)
        .uri(uri)
        .header("host", "snapdog.local:5555");
    if uri == "/ws" {
        req = req
            .header("connection", "upgrade")
            .header("upgrade", "websocket")
            .header("sec-websocket-version", "13")
            .header("sec-websocket-key", "dGhlIHNhbXBsZSBub25jZQ==");
    }
    if let Some(origin) = origin {
        req = req.header("origin", origin);
    }
    let req = req.body(Body::empty()).unwrap();
    api::build_router(&app.state)
        .oneshot(req)
        .await
        .unwrap()
        .status()
}

#[tokio::test]
async fn a_foreign_origin_is_refused_on_rest_and_websocket() {
    let app = common::test_app();
    for (method, uri) in [
        ("POST", "/api/v1/zones/1/play"),
        ("POST", "/api/v1/system/updates/apply"),
        ("GET", "/api/v1/zones"),
        ("GET", "/ws"),
    ] {
        assert_eq!(
            status(&app, method, uri, Some("http://evil.example")).await,
            StatusCode::FORBIDDEN,
            "{method} {uri}"
        );
    }
}

#[tokio::test]
async fn the_own_origin_and_requests_without_origin_pass() {
    let app = common::test_app();
    for origin in [Some("http://snapdog.local:5555"), None] {
        let s = status(&app, "GET", "/api/v1/zones", origin).await;
        assert_eq!(s, StatusCode::OK, "{origin:?}");
        let s = status(&app, "POST", "/api/v1/zones/1/play", origin).await;
        assert_ne!(s, StatusCode::FORBIDDEN, "{origin:?}");
    }
}
