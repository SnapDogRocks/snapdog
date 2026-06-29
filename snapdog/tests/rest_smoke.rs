// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! Tier-1 smoke test: proves the in-process REST harness works end-to-end
//! (build `AppState` → `build_router` → `tower::oneshot`) with no socket. IT-0003.

mod common;

use axum::http::StatusCode;

#[tokio::test]
async fn health_endpoint_returns_ok() {
    let app = common::test_app();
    let (status, _body) = app.get("/health").await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn zones_list_returns_two_zones() {
    let app = common::test_app();
    let (status, body) = app.get("/api/v1/zones").await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    // The list should reflect the 2 zones from test_config(), regardless of
    // whether it's a bare array or a wrapper object.
    let count = body
        .as_array()
        .map(Vec::len)
        .or_else(|| body.get("zones").and_then(|z| z.as_array()).map(Vec::len));
    assert_eq!(count, Some(2), "expected 2 zones, body: {body}");
}

#[tokio::test]
async fn unknown_route_is_not_500() {
    let app = common::test_app();
    let (status, _) = app.get("/api/v1/does-not-exist").await;
    assert_ne!(status, StatusCode::INTERNAL_SERVER_ERROR);
}
