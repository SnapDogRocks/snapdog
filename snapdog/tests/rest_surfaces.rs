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

/// A two-zone app with one configured radio station and no Subsonic server.
fn app_with_radio() -> common::TestApp {
    let toml = r#"
[[zone]]
name = "Ground Floor"

[[zone]]
name = "1st Floor"

[[client]]
name = "Living Room"
mac = "02:42:ac:11:00:10"
zone = "Ground Floor"

[[radio]]
name = "Test"
url = "http://example.com/stream"
"#;
    let raw: snapdog::config::FileConfig = toml::from_str(toml).expect("parses");
    common::build_test_app(snapdog::config::load_raw(raw).expect("resolves"))
}

/// MED-M1-A1: every playlist has a `playlist_id` and an `index` equal to the deprecated `id`.
#[tokio::test]
async fn media_playlists_carry_playlist_id_and_index() {
    let app = app_with_radio();
    let (s, b) = app.get("/api/v1/media/playlists").await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(b[0]["playlist_id"], "snapdog:radio");
    assert_eq!(b[0]["index"], 0);
    assert_eq!(b[0]["id"], b[0]["index"]);

    let (s, b) = app.get("/api/v1/media/playlists/0").await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(b["playlist_id"], "snapdog:radio");
    assert_eq!(b["index"], 0);
}

/// MED-M1-A2: `by-id` reads answer like the index paths and 404 for an unknown id.
#[tokio::test]
async fn media_by_id_reads_and_unknown_ids() {
    let app = app_with_radio();
    for uri in [
        "/api/v1/media/playlists/by-id/snapdog:radio",
        "/api/v1/media/playlists/by-id/snapdog%3Aradio",
    ] {
        let (s, b) = app.get(uri).await;
        assert_eq!(s, StatusCode::OK, "{uri}");
        assert_eq!(b["playlist_id"], "snapdog:radio", "{uri}");
    }
    let (s, b) = app
        .get("/api/v1/media/playlists/by-id/snapdog:radio/tracks")
        .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(b.as_array().map(Vec::len), Some(1));

    for uri in [
        "/api/v1/media/playlists/by-id/subsonic:1",
        "/api/v1/media/playlists/by-id/0",
        "/api/v1/media/playlists/by-id/radio",
        "/api/v1/media/playlists/by-id/subsonic:1/tracks",
    ] {
        let (s, b) = app.get(uri).await;
        assert_eq!(s, StatusCode::NOT_FOUND, "{uri}");
        assert_eq!(b["error"], "not_found", "{uri}");
    }
}

/// MED-M1-A2: the zone `by-id` endpoints start the same playlist as the index endpoints.
#[tokio::test]
async fn zone_by_id_endpoints_send_one_command_and_404_for_unknown_ids() {
    use snapdog::player::ZoneCommand;
    let mut app = app_with_radio();

    let (s, b) = app.get("/api/v1/zones/1/playlist/by-id").await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(b, serde_json::json!(""), "no playlist active yet");

    let (s, _) = app
        .request(
            "PUT",
            "/api/v1/zones/1/playlist/by-id",
            Some(serde_json::json!("snapdog:radio")),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    let cmds = app.drain_zone(1);
    assert_eq!(cmds.len(), 1);
    assert!(matches!(&cmds[0], ZoneCommand::SetPlaylistById(id, 0) if id == "snapdog:radio"));

    let (s, _) = app
        .request(
            "POST",
            "/api/v1/zones/1/play/playlist/by-id",
            Some(serde_json::json!({"playlist_id": "snapdog:radio", "track": 0})),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    let cmds = app.drain_zone(1);
    assert!(matches!(&cmds[..], [ZoneCommand::PlayPlaylistById(id, 0)] if id == "snapdog:radio"));

    // Counter-probes: an unknown id, an index sent as a number, an unknown zone.
    let (s, _) = app
        .request(
            "PUT",
            "/api/v1/zones/1/playlist/by-id",
            Some(serde_json::json!("subsonic:9")),
        )
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, _) = app
        .request(
            "PUT",
            "/api/v1/zones/1/playlist/by-id",
            Some(serde_json::json!(0)),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
    let (s, _) = app
        .request(
            "POST",
            "/api/v1/zones/9/play/playlist/by-id",
            Some(serde_json::json!({"playlist_id": "snapdog:radio"})),
        )
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    assert!(app.drain_zone(1).is_empty(), "rejected calls send nothing");
}

/// MED-M1-A2a: the play body accepts `index` and the deprecated `id`.
#[tokio::test]
async fn play_playlist_body_accepts_index_and_id() {
    use snapdog::player::ZoneCommand;
    let mut app = app_with_radio();
    for (body, want) in [
        (serde_json::json!({"index": 1, "track": 2}), Some(1)),
        (serde_json::json!({"id": 1, "track": 2}), Some(1)),
        (serde_json::json!({"id": 1, "index": 1}), Some(1)),
        (serde_json::json!({"id": 1, "index": 2}), None),
        (serde_json::json!({"track": 2}), None),
    ] {
        let (s, _) = app
            .request("POST", "/api/v1/zones/1/play/playlist", Some(body.clone()))
            .await;
        let cmds = app.drain_zone(1);
        if let Some(i) = want {
            assert_eq!(s, StatusCode::OK, "{body}");
            assert!(
                matches!(cmds[..], [ZoneCommand::PlayPlaylist(n, _)] if n == i),
                "{body}"
            );
        } else {
            assert_eq!(s, StatusCode::BAD_REQUEST, "{body}");
            assert!(cmds.is_empty(), "{body}");
        }
    }
}
