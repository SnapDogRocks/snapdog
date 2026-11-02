// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! MED-0011 M3: managing Subsonic playlists through the API, behind `[subsonic].playlist_write`.

mod common;

use std::time::Duration;

use axum::http::StatusCode;
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn ok(extra: &serde_json::Value) -> ResponseTemplate {
    let mut resp = json!({"status": "ok", "version": "1.16.1"});
    resp.as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    ResponseTemplate::new(200).set_body_json(json!({ "subsonic-response": resp }))
}

fn failed(code: i64, message: &str) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({"subsonic-response": {
        "status": "failed", "version": "1.16.1",
        "error": {"code": code, "message": message}}}))
}

fn list(entries: &[(&str, &str)]) -> serde_json::Value {
    let list: Vec<_> = entries
        .iter()
        .map(|(id, name)| json!({"id": id, "name": name, "songCount": 1, "duration": 60}))
        .collect();
    json!({"playlists": {"playlist": list}})
}

async fn mock(server: &MockServer, rpc: &str, response: ResponseTemplate) {
    Mock::given(method("GET"))
        .and(path(format!("/rest/{rpc}")))
        .respond_with(response)
        .mount(server)
        .await;
}

async fn calls(server: &MockServer, rpc: &str) -> Vec<Vec<(String, String)>> {
    server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.url.path() == format!("/rest/{rpc}"))
        .map(|r| r.url.query_pairs().into_owned().collect())
        .collect()
}

fn param<'a>(call: &'a [(String, String)], key: &str) -> Vec<&'a str> {
    call.iter()
        .filter(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
        .collect()
}

async fn event(
    rx: &mut tokio::sync::broadcast::Receiver<std::sync::Arc<str>>,
) -> Option<serde_json::Value> {
    tokio::time::timeout(Duration::from_millis(500), async {
        loop {
            let v: serde_json::Value = serde_json::from_str(&rx.recv().await.unwrap()).unwrap();
            if v["type"] == "playlists_changed" {
                return v;
            }
        }
    })
    .await
    .ok()
}

/// MED-M3-A1: with the switch off every write is a 403 `read_only` and never reaches Subsonic.
#[tokio::test]
async fn every_write_is_refused_while_the_switch_is_off() {
    let server = MockServer::start().await;
    let app = common::app_with_subsonic_writing(&server.uri(), 0, false);
    for (verb, uri, body) in [
        (
            "POST",
            "/api/v1/media/playlists",
            Some(json!({"name": "New"})),
        ),
        (
            "PATCH",
            "/api/v1/media/playlists/by-id/subsonic:1",
            Some(json!({"name": "X"})),
        ),
        (
            "PUT",
            "/api/v1/media/playlists/by-id/subsonic:1/tracks",
            Some(json!({"track_ids": []})),
        ),
        ("DELETE", "/api/v1/media/playlists/by-id/subsonic:1", None),
    ] {
        let (s, b) = app.request(verb, uri, body).await;
        assert_eq!(s, StatusCode::FORBIDDEN, "{verb} {uri}");
        assert_eq!(b["error"], "read_only", "{verb} {uri}");
    }
    assert!(server.received_requests().await.unwrap().is_empty());
}

/// MED-M3-A1: create, rename, add, remove, reorder and delete each work and announce a change.
#[tokio::test]
async fn writes_reach_subsonic_and_announce_a_change() {
    let server = MockServer::start().await;
    mock(&server, "getPlaylists", ok(&list(&[("9", "Mine")]))).await;
    mock(
        &server,
        "getPlaylist",
        ok(&json!({"playlist": {"id": "9", "name": "Mine", "entry": []}})),
    )
    .await;
    mock(
        &server,
        "createPlaylist",
        ok(&json!({"playlist": {"id": "9", "name": "Mine", "entry": []}})),
    )
    .await;
    mock(&server, "updatePlaylist", ok(&json!({}))).await;
    mock(&server, "deletePlaylist", ok(&json!({}))).await;
    let mut app = common::app_with_subsonic_writing(&server.uri(), 0, true);

    // Create.
    let (s, b) = app
        .request(
            "POST",
            "/api/v1/media/playlists",
            Some(json!({"name": " Mine ", "track_ids": ["t1", "t2"]})),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    assert_eq!(b["playlist_id"], "subsonic:9");
    assert_eq!(b["name"], "Mine");
    let create = &calls(&server, "createPlaylist").await[0];
    assert_eq!(param(create, "name"), ["Mine"], "trimmed");
    assert_eq!(param(create, "songId"), ["t1", "t2"]);
    assert_eq!(event(&mut app.notify_rx).await.unwrap()["total"], 1);

    // Rename, add and remove in one call.
    let (s, b) = app
        .request(
            "PATCH",
            "/api/v1/media/playlists/by-id/subsonic:9",
            Some(json!({"name": "Renamed", "add_track_ids": ["t3"], "remove_positions": [0, 2]})),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["playlist_id"], "subsonic:9");
    let update = &calls(&server, "updatePlaylist").await[0];
    assert_eq!(param(update, "playlistId"), ["9"]);
    assert_eq!(param(update, "name"), ["Renamed"]);
    assert_eq!(param(update, "songIdToAdd"), ["t3"]);
    assert_eq!(param(update, "songIndexToRemove"), ["0", "2"]);
    assert!(
        event(&mut app.notify_rx).await.is_some(),
        "event after a rename"
    );

    // Replace content and order: one createPlaylist with the playlistId.
    let (s, _) = app
        .request(
            "PUT",
            "/api/v1/media/playlists/by-id/subsonic:9/tracks",
            Some(json!({"track_ids": ["t2", "t1"]})),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    let replace = &calls(&server, "createPlaylist").await[1];
    assert_eq!(param(replace, "playlistId"), ["9"]);
    assert_eq!(param(replace, "songId"), ["t2", "t1"]);
    assert_eq!(param(replace, "name"), Vec::<&str>::new());
    assert!(
        event(&mut app.notify_rx).await.is_some(),
        "the event goes out although the list looks the same"
    );

    // Delete.
    let (s, _) = app
        .request("DELETE", "/api/v1/media/playlists/by-id/subsonic:9", None)
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    assert_eq!(
        param(&calls(&server, "deletePlaylist").await[0], "id"),
        ["9"]
    );
    assert!(event(&mut app.notify_rx).await.is_some());
}

/// MED-M3-A2: the radio list is read-only with the switch on.
#[tokio::test]
async fn the_radio_list_is_never_writable() {
    let server = MockServer::start().await;
    let app = common::app_with_subsonic_writing(&server.uri(), 0, true);
    for (verb, uri, body) in [
        (
            "PATCH",
            "/api/v1/media/playlists/by-id/snapdog:radio",
            Some(json!({"name": "X"})),
        ),
        (
            "PUT",
            "/api/v1/media/playlists/by-id/snapdog:radio/tracks",
            Some(json!({"track_ids": []})),
        ),
        (
            "DELETE",
            "/api/v1/media/playlists/by-id/snapdog:radio",
            None,
        ),
    ] {
        let (s, b) = app.request(verb, uri, body).await;
        assert_eq!(s, StatusCode::FORBIDDEN, "{verb}");
        assert_eq!(b["error"], "read_only");
    }
    assert!(server.received_requests().await.unwrap().is_empty());
    // An id that is no Subsonic playlist at all is a 404, not a write.
    let (s, _) = app
        .request("DELETE", "/api/v1/media/playlists/by-id/other:1", None)
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

/// MED-M3-A3: a refusal by Subsonic is a 502 with its message, a missing playlist a 404.
#[tokio::test]
async fn a_refusal_by_subsonic_is_a_502_with_its_message() {
    let server = MockServer::start().await;
    mock(
        &server,
        "deletePlaylist",
        failed(50, "User is not authorized"),
    )
    .await;
    mock(&server, "updatePlaylist", failed(70, "Playlist not found")).await;
    mock(
        &server,
        "createPlaylist",
        failed(50, "User is not authorized"),
    )
    .await;
    let mut app = common::app_with_subsonic_writing(&server.uri(), 0, true);

    let (s, b) = app
        .request("DELETE", "/api/v1/media/playlists/by-id/subsonic:1", None)
        .await;
    assert_eq!(s, StatusCode::BAD_GATEWAY);
    assert_eq!(b["message"], "User is not authorized");

    let (s, _) = app
        .request(
            "PATCH",
            "/api/v1/media/playlists/by-id/subsonic:1",
            Some(json!({"name": "X"})),
        )
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND);

    let (s, b) = app
        .request(
            "POST",
            "/api/v1/media/playlists",
            Some(json!({"name": "N"})),
        )
        .await;
    assert_eq!(s, StatusCode::BAD_GATEWAY);
    assert_eq!(b["message"], "User is not authorized");
    assert!(
        event(&mut app.notify_rx).await.is_none(),
        "a refused write is no change"
    );
}

#[tokio::test]
async fn invalid_bodies_are_rejected_before_subsonic_is_asked() {
    let server = MockServer::start().await;
    let app = common::app_with_subsonic_writing(&server.uri(), 0, true);
    for (verb, uri, body) in [
        ("POST", "/api/v1/media/playlists", json!({"name": "  "})),
        (
            "PATCH",
            "/api/v1/media/playlists/by-id/subsonic:1",
            json!({}),
        ),
        (
            "PATCH",
            "/api/v1/media/playlists/by-id/subsonic:1",
            json!({"name": ""}),
        ),
    ] {
        let (s, _) = app.request(verb, uri, Some(body)).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{verb} {uri}");
    }
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn without_subsonic_a_write_is_a_503() {
    let app = common::test_app();
    let (s, _) = app
        .request(
            "POST",
            "/api/v1/media/playlists",
            Some(json!({"name": "N"})),
        )
        .await;
    assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE);
}
