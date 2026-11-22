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
async fn a_configured_client_that_is_not_connected_answers_409_an_unknown_one_404() {
    // Default harness clients have snapcast_id: None: they exist but are not connected.
    let app = common::test_app();
    for (method, uri, body) in [
        ("PUT", "/api/v1/clients/1/volume", Some(json!(50))),
        ("PUT", "/api/v1/clients/1/mute", Some(json!(true))),
        ("POST", "/api/v1/clients/1/mute/toggle", None),
        ("PUT", "/api/v1/clients/1/latency", Some(json!(10))),
    ] {
        let (status, body_out) = app.request(method, uri, body.clone()).await;
        assert_eq!(status, StatusCode::CONFLICT, "{method} {uri}");
        assert_eq!(body_out["error"], "conflict");
        let unknown = uri.replace("/clients/1/", "/clients/99/");
        assert_eq!(
            app.request(method, &unknown, body).await.0,
            StatusCode::NOT_FOUND,
            "{method} {unknown}"
        );
    }
}

#[tokio::test]
async fn renaming_an_unknown_client_is_404() {
    let app = common::test_app();
    assert_eq!(
        app.request("PUT", "/api/v1/clients/99/name", Some(json!("Kitchen")))
            .await
            .0,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn a_closed_command_channel_is_a_server_error_not_a_success() {
    let mut app = common::test_app();
    {
        let mut s = app.store.write().await;
        s.clients.get_mut(&1).unwrap().snapcast_id = Some("snap-1".into());
    }
    // Drop the receivers the handlers send to.
    drop(std::mem::replace(
        &mut app.snap_rx,
        tokio::sync::mpsc::channel(1).1,
    ));
    let zone_rx = app.zone_rx.get_mut(&1).unwrap();
    drop(std::mem::replace(
        zone_rx,
        std::sync::Mutex::new(tokio::sync::mpsc::channel(1).1),
    ));

    for (method, uri, body) in [
        ("PUT", "/api/v1/clients/1/volume", Some(json!(50))),
        ("PUT", "/api/v1/clients/1/mute", Some(json!(true))),
        ("POST", "/api/v1/clients/1/mute/toggle", None),
        ("PUT", "/api/v1/clients/1/latency", Some(json!(10))),
        ("PUT", "/api/v1/clients/1/zone", Some(json!(1))),
        (
            "PUT",
            "/api/v1/zones/1/eq",
            Some(json!({"enabled": true, "bands": []})),
        ),
        ("POST", "/api/v1/zones/1/eq/preset", Some(json!("flat"))),
    ] {
        assert_eq!(
            app.request(method, uri, body).await.0,
            StatusCode::INTERNAL_SERVER_ERROR,
            "{method} {uri}"
        );
    }
}

#[tokio::test]
async fn eq_and_speaker_of_a_client_that_is_not_connected_are_stored_and_succeed() {
    let mut app = common::test_app();
    app.store
        .write()
        .await
        .clients
        .get_mut(&1)
        .unwrap()
        .is_snapdog = true;
    let (s, _) = app
        .request(
            "PUT",
            "/api/v1/clients/1/eq",
            Some(json!({"enabled": true, "bands": bands(2)})),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    let (s, _) = app
        .request(
            "PUT",
            "/api/v1/clients/1/speaker",
            Some(json!({"custom": {"enabled": true, "bands": bands(1)}})),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    // Nothing to send to; the client gets both when it connects.
    assert!(app.drain_snap().is_empty());
    let (_, eq) = app.get("/api/v1/clients/1/eq").await;
    assert_eq!(eq["bands"].as_array().map(Vec::len), Some(2));
}

#[tokio::test]
async fn two_quick_band_changes_of_a_client_both_survive() {
    let app = common::test_app();
    app.store
        .write()
        .await
        .clients
        .get_mut(&1)
        .unwrap()
        .is_snapdog = true;
    app.request(
        "PUT",
        "/api/v1/clients/1/eq",
        Some(json!({"enabled": true, "bands": bands(2)})),
    )
    .await;
    let band = |gain: f64| json!({"freq": 1000.0, "gain": gain, "q": 1.0, "type": "peaking"});
    let (a, b) = tokio::join!(
        app.request("PUT", "/api/v1/clients/1/eq/0", Some(band(3.0))),
        app.request("PUT", "/api/v1/clients/1/eq/1", Some(band(-4.0))),
    );
    assert_eq!((a.0, b.0), (StatusCode::OK, StatusCode::OK));
    let (_, eq) = app.get("/api/v1/clients/1/eq").await;
    assert_eq!(eq["bands"][0]["gain"], 3.0);
    assert_eq!(eq["bands"][1]["gain"], -4.0);
}

#[tokio::test]
async fn a_zone_band_change_sends_only_the_band() {
    let mut app = common::test_app();
    app.state.eq_store.lock().unwrap().set(
        1,
        serde_json::from_value(json!({"enabled": true, "bands": bands(2)})).unwrap(),
    );
    let (s, b) = app
        .request(
            "PUT",
            "/api/v1/zones/1/eq/1",
            Some(json!({"freq": 500.0, "gain": 2.0, "q": 1.0, "type": "peaking"})),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(b["bands"][1]["gain"], 2.0);
    assert!(matches!(
        app.drain_zone(1).as_slice(),
        [ZoneCommand::SetEqBand { band_index: 1, .. }]
    ));
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

// ── A renamed client shows its new name everywhere ────────────

#[tokio::test]
async fn a_renamed_client_has_its_new_name_in_the_list_and_the_details() {
    let app = common::test_app();
    let (_, before) = app.get("/api/v1/clients/1").await;
    assert_ne!(before["name"], "Kitchen");

    let (s, _) = app
        .request("PUT", "/api/v1/clients/1/name", Some(json!("Kitchen")))
        .await;
    assert!(s.is_success());

    let (_, one) = app.get("/api/v1/clients/1").await;
    assert_eq!(one["name"], "Kitchen");
    let (_, all) = app.get("/api/v1/clients").await;
    assert_eq!(all[0]["name"], "Kitchen");
    let (_, name) = app.get("/api/v1/clients/1/name").await;
    assert_eq!(name, "Kitchen");
}

// ── Client EQ and speaker correction announce themselves ──────

/// Nothing else tells a consumer that the equalizer or the speaker correction of a client
/// changed, because the web interface and Home Assistant read them once and then listen.
#[tokio::test]
async fn changing_the_eq_or_the_speaker_of_a_client_sends_an_event() {
    let mut app = common::test_app();
    let mut store = app.store.write().await;
    let client = store.clients.get_mut(&1).expect("client 1");
    client.is_snapdog = true;
    client.snapcast_id = Some("snap-1".into());
    drop(store);

    let (s, _) = app
        .request(
            "PUT",
            "/api/v1/clients/1/eq",
            Some(json!({"enabled": false, "bands": bands(2)})),
        )
        .await;
    assert!(s.is_success());
    let event: serde_json::Value =
        serde_json::from_str(&app.notify_rx.recv().await.expect("an event")).unwrap();
    assert_eq!(event["type"], "client_eq_changed");
    assert_eq!(event["client"], 1);
    assert_eq!(event["enabled"], false);
    assert_eq!(event["bands"].as_array().map(Vec::len), Some(2));

    let (s, _) = app
        .request(
            "PUT",
            "/api/v1/clients/1/speaker",
            Some(json!({"custom": {"enabled": true, "bands": bands(1)}})),
        )
        .await;
    assert!(s.is_success());
    let event: serde_json::Value =
        serde_json::from_str(&app.notify_rx.recv().await.expect("an event")).unwrap();
    assert_eq!(event["type"], "client_speaker_changed");
    assert_eq!(event["client"], 1);
    assert_eq!(event["bands"].as_array().map(Vec::len), Some(1));
}

// ── Value bounds shared with MQTT ─────────────────────────────

#[tokio::test]
async fn out_of_range_eq_values_are_rejected_on_every_write_path() {
    let app = common::test_app();
    let bad = json!({"freq": 1000.0, "gain": 120.0, "q": 1.0, "type": "peaking"});
    let huge = json!({"freq": 1e39, "gain": 0.0, "q": 1.0, "type": "peaking"});
    for (method, uri, body) in [
        (
            "PUT",
            "/api/v1/zones/1/eq",
            json!({"enabled": true, "bands": [bad.clone()]}),
        ),
        (
            "PUT",
            "/api/v1/zones/1/eq",
            json!({"enabled": true, "bands": [huge.clone()]}),
        ),
        ("PUT", "/api/v1/zones/1/eq/0", bad.clone()),
    ] {
        let (s, _) = app.request(method, uri, Some(body)).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{method} {uri}");
    }
    let (s, _) = app
        .request(
            "PUT",
            "/api/v1/zones/1/eq",
            Some(json!({"enabled": true, "bands": bands(3)})),
        )
        .await;
    assert!(s.is_success(), "valid bands still pass");
}

#[tokio::test]
async fn latency_and_seek_position_outside_the_shared_bounds_are_rejected() {
    let app = common::test_app();
    let (s, _) = app
        .request(
            "PUT",
            "/api/v1/clients/1/latency",
            Some(json!(2_000_000_000)),
        )
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    for body in [
        json!({"position_ms": -5000}),
        json!({"position_ms": snapdog_common::MAX_SEEK_MS + 1}),
        json!({"offset_ms": i64::MAX}),
    ] {
        let (s, _) = app
            .request("PUT", "/api/v1/zones/1/track/position", Some(body.clone()))
            .await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{body}");
    }
}
