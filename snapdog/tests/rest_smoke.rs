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

#[tokio::test]
async fn eq_presets_lists_what_the_preset_calls_accept() {
    let app = common::test_app();
    let (status, body) = app.get("/api/v1/eq/presets").await;
    assert_eq!(status, StatusCode::OK, "body: {body}");

    let presets = body.as_array().expect("an array of presets");
    assert!(presets.len() > 1, "{presets:?}");
    let flat = presets
        .iter()
        .find(|p| p["name"] == "flat")
        .expect("the flat preset is listed");
    assert_eq!(flat["bands"].as_array().map(Vec::len), Some(0));
    assert!(
        presets
            .iter()
            .any(|p| p["bands"].as_array().is_some_and(|b| !b.is_empty())),
        "at least one preset carries bands"
    );

    // Every listed name is accepted by the call that applies a preset, on a zone.
    for preset in presets {
        let name = preset["name"].as_str().expect("a name");
        let (status, applied) = app
            .request(
                "POST",
                "/api/v1/zones/1/eq/preset",
                Some(serde_json::json!(name)),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{name}: {applied}");
        assert_eq!(applied["preset"], name);
        assert_eq!(applied["bands"], preset["bands"], "{name}");
    }
}
