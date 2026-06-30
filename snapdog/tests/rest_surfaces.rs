// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! REST contract for clients / system / health (IT-T12/T13). Tier-1, in-process.

mod common;

use axum::http::StatusCode;
use serde_json::json;

#[tokio::test]
async fn clients_count_and_list_reflect_config() {
    let app = common::test_app();
    let (s, b) = app.get("/api/v1/clients/count").await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(b, json!(2));

    let (s, b) = app.get("/api/v1/clients").await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(b.as_array().map(Vec::len), Some(2));
}

#[tokio::test]
async fn client_get_boundaries() {
    let app = common::test_app();
    assert_eq!(app.get("/api/v1/clients/1").await.0, StatusCode::OK);
    assert_eq!(app.get("/api/v1/clients/2").await.0, StatusCode::OK);
    assert_eq!(app.get("/api/v1/clients/0").await.0, StatusCode::NOT_FOUND);
    assert_eq!(app.get("/api/v1/clients/99").await.0, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn system_status_and_version() {
    let app = common::test_app();
    let (s, b) = app.get("/api/v1/system/status").await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(b["zones"], 2);
    assert_eq!(b["clients"], 2);
    assert_eq!(b["radios"], 0);
    assert!(b["version"].is_string(), "version present: {b}");

    let (s, b) = app.get("/api/v1/system/version").await;
    assert_eq!(s, StatusCode::OK);
    assert!(b["version"].is_string());
    assert!(b["rust_version"].is_string());
    assert_eq!(b["name"], "SnapDog"); // default_server_name
}

#[tokio::test]
async fn media_playlists_empty_and_indices_404() {
    // No subsonic + no radios → playlists is an empty list (no network), and every
    // playlist index is out of range.
    let app = common::test_app();

    let (s, b) = app.get("/api/v1/media/playlists").await;
    assert_eq!(s, StatusCode::OK);
    assert!(b.as_array().is_some_and(Vec::is_empty), "no playlists: {b}");

    for uri in [
        "/api/v1/media/playlists/0",
        "/api/v1/media/playlists/0/tracks",
        "/api/v1/media/playlists/0/tracks/0",
        "/api/v1/media/playlists/0/cover",
        "/api/v1/media/playlists/5/tracks/0/cover",
    ] {
        let (s, b) = app.get(uri).await;
        assert_eq!(s, StatusCode::NOT_FOUND, "{uri} → 404");
        assert_eq!(b["error"], "not_found", "{uri} body");
    }
}

#[tokio::test]
async fn client_speaker_404_and_422() {
    // Unknown client → 404; known client that isn't a SnapDog client → 422.
    let app = common::test_app();

    assert_eq!(
        app.get("/api/v1/clients/0/speaker").await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        app.get("/api/v1/clients/99/speaker").await.0,
        StatusCode::NOT_FOUND
    );

    let (s, b) = app.get("/api/v1/clients/1/speaker").await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(b["error"], "unprocessable");
}

#[tokio::test]
async fn knx_programming_mode_conflict_when_device_mode_inactive() {
    // test_app has knx_device_control: None → KNX routes short-circuit to 409.
    let app = common::test_app();

    let (s, b) = app.get("/api/v1/knx/programming-mode").await;
    assert_eq!(s, StatusCode::CONFLICT);
    assert_eq!(b["error"], "conflict");
    assert_eq!(b["message"], "KNX device mode not active");

    let (s, _) = app
        .request("PUT", "/api/v1/knx/programming-mode", Some(json!(true)))
        .await;
    assert_eq!(s, StatusCode::CONFLICT);
}

#[tokio::test]
async fn health_liveness_readiness() {
    let app = common::test_app();
    let (s, b) = app.get("/health").await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(b["status"], "ok");
    assert_eq!(b["zones"], 2);
    assert_eq!(b["clients"], 2);

    assert_eq!(app.get("/health/ready").await.0, StatusCode::OK);
    assert_eq!(app.get("/health/live").await.0, StatusCode::OK);
}
