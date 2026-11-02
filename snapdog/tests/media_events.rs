// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! MED-0011 M1: the playlist list keeps its last good state across a Subsonic outage
//! (A3), a change in Subsonic is announced as `playlists_changed` (A4) and track lists
//! page with `X-Total-Count` (A5).

mod common;

use std::time::Duration;

use axum::http::StatusCode;
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn playlists(entries: &[(&str, &str, u32)]) -> serde_json::Value {
    let list: Vec<_> = entries
        .iter()
        .map(|(id, name, n)| json!({"id": id, "name": name, "songCount": n, "duration": 60}))
        .collect();
    json!({"subsonic-response": {"status": "ok", "version": "1.16.1",
        "playlists": {"playlist": list}}})
}

/// Wait for a notification of the given type, or `None` after `within`.
async fn next_of_type(
    rx: &mut tokio::sync::broadcast::Receiver<std::sync::Arc<str>>,
    kind: &str,
    within: Duration,
) -> Option<serde_json::Value> {
    tokio::time::timeout(within, async {
        loop {
            let msg = rx.recv().await.expect("channel open");
            let v: serde_json::Value = serde_json::from_str(&msg).expect("json");
            if v["type"] == kind {
                return v;
            }
        }
    })
    .await
    .ok()
}

/// MED-M1-A4: a change produces `playlists_changed` within `refresh_secs` plus a second,
/// an unchanged list produces none.
#[tokio::test]
async fn a_changed_list_sends_playlists_changed_and_an_unchanged_one_does_not() {
    let server = MockServer::start().await;
    // Two reads of the same list, then a new playlist appears.
    Mock::given(method("GET"))
        .and(path("/rest/getPlaylists"))
        .respond_with(ResponseTemplate::new(200).set_body_json(playlists(&[("1", "A", 3)])))
        .up_to_n_times(2)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/rest/getPlaylists"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(playlists(&[("1", "A", 3), ("2", "B", 1)])),
        )
        .mount(&server)
        .await;

    let mut app = common::app_with_subsonic(&server.uri(), 1);
    let task = tokio::spawn(snapdog::api::run_playlist_refresh(app.state.clone()));

    // Reads 1 and 2 return the same list: no event within the first interval and a half.
    assert!(
        next_of_type(
            &mut app.notify_rx,
            "playlists_changed",
            Duration::from_millis(1500)
        )
        .await
        .is_none(),
        "an unchanged list must not send an event"
    );
    // Read 3 returns the new list.
    let event = next_of_type(
        &mut app.notify_rx,
        "playlists_changed",
        Duration::from_secs(2),
    )
    .await
    .expect("a new playlist sends playlists_changed");
    assert_eq!(event["total"], 2);
    task.abort();
}

/// MED-M1-A3: with Subsonic gone the last good list stays, across refresh runs and requests,
/// and no event is sent.
#[tokio::test]
async fn an_outage_keeps_the_last_good_list_across_a_refresh_run() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/rest/getPlaylists"))
        .respond_with(ResponseTemplate::new(200).set_body_json(playlists(&[("1", "A", 3)])))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/rest/getPlaylists"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;

    let mut app = common::app_with_subsonic(&server.uri(), 1);
    let task = tokio::spawn(snapdog::api::run_playlist_refresh(app.state.clone()));

    // Let at least two refresh runs pass: the first reads the list, the later ones fail.
    tokio::time::sleep(Duration::from_millis(2300)).await;
    let (s, b) = app.get("/api/v1/media/playlists").await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(b.as_array().map(Vec::len), Some(1), "kept the list: {b}");
    assert_eq!(b[0]["playlist_id"], "subsonic:1");
    assert!(
        next_of_type(
            &mut app.notify_rx,
            "playlists_changed",
            Duration::from_millis(50)
        )
        .await
        .is_none(),
        "an outage is not a change"
    );
    task.abort();
}

/// A switched-off refresh (`refresh_secs = 0`) never reads on its own.
#[tokio::test]
async fn refresh_secs_zero_turns_the_background_read_off() {
    let server = MockServer::start().await;
    let app = common::app_with_subsonic(&server.uri(), 0);
    snapdog::api::run_playlist_refresh(app.state.clone()).await;
    assert!(server.received_requests().await.unwrap().is_empty());
}

/// MED-M1-A5: `limit` and `offset` cut the array and `X-Total-Count` has the full size.
/// Without them the body is the whole array and there is no header.
#[tokio::test]
async fn tracks_page_with_limit_and_offset() {
    let toml = r#"
[[zone]]
name = "Z"

[[radio]]
name = "A"
url = "http://example.com/a"
[[radio]]
name = "B"
url = "http://example.com/b"
[[radio]]
name = "C"
url = "http://example.com/c"
"#;
    let raw: snapdog::config::FileConfig = toml::from_str(toml).unwrap();
    let app = common::build_test_app(snapdog::config::load_raw(raw).unwrap());
    let uri = "/api/v1/media/playlists/0/tracks";

    let (s, all, total) = app.get_with_total(uri).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(all.as_array().map(Vec::len), Some(3));
    assert_eq!(total, None, "no header without paging");

    let (_, page, total) = app.get_with_total(&format!("{uri}?limit=2&offset=1")).await;
    assert_eq!(total.as_deref(), Some("3"));
    assert_eq!(page, json!([all[1], all[2]]));

    let (_, page, total) = app.get_with_total(&format!("{uri}?limit=1")).await;
    assert_eq!(
        (page.as_array().map(Vec::len), total.as_deref()),
        (Some(1), Some("3"))
    );

    let (_, page, total) = app.get_with_total(&format!("{uri}?offset=5")).await;
    assert_eq!((page, total.as_deref()), (json!([]), Some("3")));

    // The by-id path pages the same way.
    let (_, page, total) = app
        .get_with_total("/api/v1/media/playlists/by-id/snapdog:radio/tracks?limit=1&offset=2")
        .await;
    assert_eq!(page, json!([all[2]]));
    assert_eq!(total.as_deref(), Some("3"));
}
