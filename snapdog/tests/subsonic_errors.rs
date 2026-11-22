// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! A failing Subsonic request never hands the authenticated request URL to an API client: the
//! answer carries a fixed text, and the error the server logs has no URL in it.

mod common;

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

async fn mock(server: &MockServer, rpc: &str, response: ResponseTemplate) {
    Mock::given(method("GET"))
        .and(path(format!("/rest/{rpc}")))
        .respond_with(response)
        .mount(server)
        .await;
}

#[tokio::test]
async fn a_failed_cover_fetch_answers_502_without_the_subsonic_credentials() {
    let server = MockServer::start().await;
    mock(
        &server,
        "getPlaylists",
        ok(&json!({"playlists": {"playlist": [
            {"id": "9", "name": "Mine", "songCount": 1, "duration": 60}
        ]}})),
    )
    .await;
    mock(
        &server,
        "getPlaylist",
        ok(&json!({"playlist": {"id": "9", "name": "Mine", "entry": [
            {"id": "t1", "title": "One", "coverArt": "c1", "duration": 60}
        ]}})),
    )
    .await;
    mock(&server, "getCoverArt", ResponseTemplate::new(500)).await;

    let app = common::app_with_subsonic(&server.uri(), 0);
    // Index 0 is the first Subsonic playlist (the test config has no radio).
    let (status, body) = app.get("/api/v1/media/playlists/0/tracks/0/cover").await;

    assert_eq!(status, StatusCode::BAD_GATEWAY);
    let text = body.to_string();
    for secret in ["t=", "s=", "u=", "getCoverArt", &server.uri()] {
        assert!(
            !text.contains(secret),
            "the answer must not contain {secret:?}: {text}"
        );
    }
}
