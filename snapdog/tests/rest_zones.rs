// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! REST zone-endpoint contract (IT-T11): status + body + **exactly one**
//! captured `ZoneCommand`, plus boundary (zone 0 / unknown → 404), volume
//! parse/clamp, transport mapping, and the cover placeholder.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::json;
use snapdog::player::ZoneCommand;
use tower::ServiceExt;

#[tokio::test]
async fn count_and_list_reflect_config() {
    let app = common::test_app();
    let (s, b) = app.get("/api/v1/zones/count").await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(b, json!(2));

    let (s, b) = app.get("/api/v1/zones").await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(b.as_array().map(Vec::len), Some(2));
}

#[tokio::test]
async fn get_zone_boundaries() {
    let app = common::test_app();
    assert_eq!(app.get("/api/v1/zones/1").await.0, StatusCode::OK);
    assert_eq!(app.get("/api/v1/zones/2").await.0, StatusCode::OK);
    // 1-based: zone 0 is invalid (checked_sub underflow) and 99 is out of range.
    assert_eq!(app.get("/api/v1/zones/0").await.0, StatusCode::NOT_FOUND);
    assert_eq!(app.get("/api/v1/zones/99").await.0, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn set_volume_absolute_captures_exactly_one_command() {
    let mut app = common::test_app();
    let (s, b) = app
        .request("PUT", "/api/v1/zones/1/volume", Some(json!(75)))
        .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(b, json!(75));
    let cmds = app.drain_zone(1);
    assert_eq!(cmds.len(), 1, "exactly one command");
    assert!(matches!(cmds[0], ZoneCommand::SetVolume(75)));
}

#[tokio::test]
async fn set_volume_clamps_above_max() {
    let mut app = common::test_app();
    let (s, b) = app
        .request("PUT", "/api/v1/zones/1/volume", Some(json!(150)))
        .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(b, json!(100), "volume clamps to 100");
    assert!(matches!(
        app.drain_zone(1).as_slice(),
        [ZoneCommand::SetVolume(100)]
    ));
}

#[tokio::test]
async fn set_volume_relative_string_resolves() {
    let mut app = common::test_app();
    let (s, _b) = app
        .request("PUT", "/api/v1/zones/1/volume", Some(json!("+5")))
        .await;
    assert_eq!(s, StatusCode::OK);
    let cmds = app.drain_zone(1);
    assert_eq!(cmds.len(), 1);
    assert!(matches!(cmds[0], ZoneCommand::SetVolume(_)));
}

#[tokio::test]
async fn set_volume_invalid_relative_is_400() {
    let mut app = common::test_app();
    let (s, _) = app
        .request("PUT", "/api/v1/zones/1/volume", Some(json!("nonsense")))
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert!(app.drain_zone(1).is_empty(), "no command on bad input");
}

#[tokio::test]
async fn set_volume_unknown_zone_is_404() {
    let app = common::test_app();
    let (s, _) = app
        .request("PUT", "/api/v1/zones/99/volume", Some(json!(50)))
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn transport_endpoints_emit_one_correct_command_each() {
    let mut app = common::test_app();
    #[allow(clippy::type_complexity)]
    let cases: &[(&str, fn(&ZoneCommand) -> bool)] = &[
        ("play", |c| matches!(c, ZoneCommand::Play)),
        ("pause", |c| matches!(c, ZoneCommand::Pause)),
        ("stop", |c| matches!(c, ZoneCommand::Stop)),
        ("next", |c| matches!(c, ZoneCommand::Next)),
        ("previous", |c| matches!(c, ZoneCommand::Previous)),
    ];
    for (path, pred) in cases {
        let (s, _) = app
            .request("POST", &format!("/api/v1/zones/1/{path}"), None)
            .await;
        assert!(s.is_success(), "POST {path} -> {s}");
        let cmds = app.drain_zone(1);
        assert_eq!(cmds.len(), 1, "{path} emits one command");
        assert!(pred(&cmds[0]), "{path} mapped to {:?}", cmds[0]);
    }
}

#[tokio::test]
async fn set_mute_captures_command() {
    let mut app = common::test_app();
    let (s, _) = app
        .request("PUT", "/api/v1/zones/1/mute", Some(json!(true)))
        .await;
    assert!(s.is_success());
    assert!(matches!(
        app.drain_zone(1).as_slice(),
        [ZoneCommand::SetMute(true)]
    ));
}

#[tokio::test]
async fn zone_cover_returns_placeholder_png_with_etag() {
    let app = common::test_app();
    let router = snapdog::api::build_router(&app.state);
    let resp = router
        .oneshot(
            Request::builder()
                .uri("/api/v1/zones/1/cover")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let etag = resp
        .headers()
        .get("etag")
        .and_then(|v| v.to_str().ok())
        .map(String::from);
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    assert!(!bytes.is_empty(), "cover returns bytes");
    assert_eq!(&bytes[..4], b"\x89PNG", "placeholder is a PNG");
    assert!(etag.is_some(), "cover sets an ETag");
}
