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
    assert!(b["version"].is_string(), "version present: {b}");

    assert_eq!(app.get("/api/v1/system/version").await.0, StatusCode::OK);
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
