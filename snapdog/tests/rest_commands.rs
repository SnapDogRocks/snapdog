// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! IT-T12 — REST → captured-command contract + EQ validation. Tier-1, in-process:
//! drives the real router and asserts the `ZoneCommand`/`SnapcastCmd` each handler
//! emits (via the captured channels) plus the 400/422/404 validation boundaries.
//! (`ZoneCommand`/`SnapcastCmd`/`ClientAction` derive only `Debug`, so assertions
//! use `matches!`, not `assert_eq!`.)

mod common;

use axum::http::StatusCode;
use serde_json::json;
use snapdog::player::{ClientAction, SnapcastCmd, ZoneCommand};
use snapdog_common::RepeatMode;

/// `n` identical valid peaking bands.
fn bands(n: usize) -> serde_json::Value {
    json!(
        (0..n)
            .map(|_| json!({"freq": 1000.0, "gain": 0.0, "q": 1.0, "type": "peaking"}))
            .collect::<Vec<_>>()
    )
}

// ── Zone-action command capture ───────────────────────────────

#[tokio::test]
async fn zone_actions_emit_expected_commands() {
    let mut app = common::test_app();

    let (s, _) = app
        .request("PUT", "/api/v1/zones/1/shuffle", Some(json!(true)))
        .await;
    assert!(s.is_success());
    assert!(matches!(
        app.drain_zone(1).as_slice(),
        [ZoneCommand::SetShuffle(true)]
    ));

    let (s, _) = app
        .request("PUT", "/api/v1/zones/1/repeat", Some(json!("playlist")))
        .await;
    assert!(s.is_success());
    assert!(matches!(
        app.drain_zone(1).as_slice(),
        [ZoneCommand::SetRepeat(RepeatMode::Playlist)]
    ));

    let (s, _) = app
        .request("POST", "/api/v1/zones/1/repeat/toggle", None)
        .await;
    assert!(s.is_success());
    assert!(matches!(
        app.drain_zone(1).as_slice(),
        [ZoneCommand::CycleRepeat]
    ));

    let (s, _) = app
        .request("POST", "/api/v1/zones/1/mute/toggle", None)
        .await;
    assert!(s.is_success());
    assert!(matches!(
        app.drain_zone(1).as_slice(),
        [ZoneCommand::ToggleMute]
    ));
}

#[tokio::test]
async fn track_position_absolute_and_relative_seek() {
    let mut app = common::test_app();

    let (s, _) = app
        .request(
            "PUT",
            "/api/v1/zones/1/track/position",
            Some(json!({"position_ms": 45000})),
        )
        .await;
    assert!(s.is_success());
    assert!(matches!(
        app.drain_zone(1).as_slice(),
        [ZoneCommand::Seek(45000)]
    ));

    let (s, _) = app
        .request(
            "PUT",
            "/api/v1/zones/1/track/position",
            Some(json!({"offset_ms": -5000})),
        )
        .await;
    assert!(s.is_success());
    assert!(matches!(
        app.drain_zone(1).as_slice(),
        [ZoneCommand::SeekRelative(-5000)]
    ));
}

#[tokio::test]
async fn track_position_rejects_empty_or_ambiguous_body() {
    let mut app = common::test_app();

    for body in [json!({}), json!({"position_ms": 1, "offset_ms": 2})] {
        let (s, _) = app
            .request("PUT", "/api/v1/zones/1/track/position", Some(body))
            .await;
        assert_eq!(s, StatusCode::BAD_REQUEST);
        assert!(app.drain_zone(1).is_empty(), "no command on bad seek body");
    }
}

#[tokio::test]
async fn zone_volume_clamps_low_at_api() {
    // -10 is a valid absolute value that clamps to 0 (not a 400).
    let mut app = common::test_app();
    let (s, b) = app
        .request("PUT", "/api/v1/zones/1/volume", Some(json!(-10)))
        .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(b, json!(0));
    assert!(matches!(
        app.drain_zone(1).as_slice(),
        [ZoneCommand::SetVolume(0)]
    ));
}

// ── Client-action command capture (needs a resolved snapcast_id) ───

#[tokio::test]
async fn client_actions_404_until_snapcast_id_resolved() {
    // Default harness clients have snapcast_id: None → actions resolve to 404.
    let app = common::test_app();
    assert_eq!(
        app.request("PUT", "/api/v1/clients/1/volume", Some(json!(50)))
            .await
            .0,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn client_actions_emit_snapcast_commands() {
    let mut app = common::test_app();
    {
        let mut s = app.store.write().await;
        s.clients.get_mut(&1).unwrap().snapcast_id = Some("snap-1".into());
    }

    let (s, b) = app
        .request("PUT", "/api/v1/clients/1/volume", Some(json!(80)))
        .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(b, json!(80));
    assert!(matches!(
        app.drain_snap().as_slice(),
        [SnapcastCmd::Client {
            action: ClientAction::Volume(80),
            ..
        }]
    ));

    let (s, _) = app
        .request("PUT", "/api/v1/clients/1/mute", Some(json!(true)))
        .await;
    assert_eq!(s, StatusCode::OK);
    assert!(matches!(
        app.drain_snap().as_slice(),
        [SnapcastCmd::Client {
            action: ClientAction::Mute(true),
            ..
        }]
    ));

    let (s, _) = app
        .request("PUT", "/api/v1/clients/1/latency", Some(json!(-20)))
        .await;
    assert_eq!(s, StatusCode::OK);
    assert!(matches!(
        app.drain_snap().as_slice(),
        [SnapcastCmd::Client {
            action: ClientAction::Latency(-20),
            ..
        }]
    ));
}

// ── EQ validation: 400 (band overflow) vs 422 (shape) vs 404 ───────

#[tokio::test]
async fn zone_eq_band_overflow_is_400_max_is_ok() {
    let mut app = common::test_app();

    // 11 bands > MAX_EQ_BANDS(10) → handler-level 400 (not a serde 422).
    let (s, b) = app
        .request(
            "PUT",
            "/api/v1/zones/1/eq",
            Some(json!({"enabled": true, "bands": bands(11)})),
        )
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert_eq!(b["error"], "bad_request");
    assert!(app.drain_zone(1).is_empty());

    // Exactly 10 bands is accepted (boundary is `>`), emitting SetEq.
    let (s, _) = app
        .request(
            "PUT",
            "/api/v1/zones/1/eq",
            Some(json!({"enabled": true, "bands": bands(10)})),
        )
        .await;
    assert!(s.is_success());
    assert!(matches!(
        app.drain_zone(1).as_slice(),
        [ZoneCommand::SetEq(_)]
    ));
}

#[tokio::test]
async fn zone_eq_malformed_shape_is_422() {
    let app = common::test_app();
    for body in [
        json!({"enabled": true, "bands": [{"freq": 1000, "gain": 3, "q": 1, "type": "notch"}]}), // unknown filter type
        json!({"enabled": true, "bands": [{"freq": 1000, "gain": 3, "type": "peaking"}]}), // missing q
        json!({"bands": []}),                   // missing enabled
        json!({"enabled": "yes", "bands": []}), // wrong type for enabled
    ] {
        let (s, _) = app.request("PUT", "/api/v1/zones/1/eq", Some(body)).await;
        assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
    }
}

#[tokio::test]
async fn zone_eq_not_found_and_presets() {
    let mut app = common::test_app();

    // Unknown zone.
    assert_eq!(
        app.request(
            "PUT",
            "/api/v1/zones/99/eq",
            Some(json!({"enabled": true, "bands": []}))
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );

    // Band index out of range on a fresh (0-band) EQ.
    assert_eq!(
        app.request(
            "PUT",
            "/api/v1/zones/1/eq/0",
            Some(json!({"freq": 1000.0, "gain": 0.0, "q": 1.0, "type": "peaking"}))
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );

    // Valid preset → SetEq.
    let (s, _) = app
        .request("POST", "/api/v1/zones/1/eq/preset", Some(json!("flat")))
        .await;
    assert!(s.is_success());
    assert!(matches!(
        app.drain_zone(1).as_slice(),
        [ZoneCommand::SetEq(_)]
    ));

    // Unknown preset name → 400; wrong JSON type → 422.
    assert_eq!(
        app.request(
            "POST",
            "/api/v1/zones/1/eq/preset",
            Some(json!("super_bass"))
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        app.request("POST", "/api/v1/zones/1/eq/preset", Some(json!(123)))
            .await
            .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
}

#[tokio::test]
async fn client_eq_requires_snapdog_client() {
    let app = common::test_app();

    // Known client that isn't a SnapDog client → 422 (require_snapdog).
    let (s, b) = app
        .request(
            "PUT",
            "/api/v1/clients/1/eq",
            Some(json!({"enabled": true, "bands": []})),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(b["error"], "unprocessable");

    // Unknown client → 404.
    assert_eq!(
        app.request(
            "PUT",
            "/api/v1/clients/99/eq",
            Some(json!({"enabled": true, "bands": []}))
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
}
