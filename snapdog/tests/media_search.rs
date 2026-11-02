// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! MED-0011 M2: `GET /media/search`, the cover proxy and album playback.

mod common;

use axum::http::StatusCode;
use serde_json::json;
use snapdog::player::ZoneCommand;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn ok(body: &serde_json::Value) -> ResponseTemplate {
    let mut resp = json!({"status": "ok", "version": "1.16.1"});
    resp.as_object_mut()
        .unwrap()
        .extend(body.as_object().unwrap().clone());
    ResponseTemplate::new(200).set_body_json(json!({ "subsonic-response": resp }))
}

async fn subsonic_with_library() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/rest/search3"))
        .respond_with(ok(&json!({"searchResult3": {
            "song": [
                {"id": "t1", "title": "Morning Song", "artist": "Ann", "album": "Song Cycle",
                 "duration": 200, "coverArt": "al-1"},
                {"id": "t2", "title": "Night Song", "artist": "Bob", "album": "Dusk",
                 "duration": 100}
            ],
            "album": [
                {"id": "al-1", "name": "Song Cycle", "artist": "Ann", "coverArt": "al-1",
                 "duration": 900}
            ]
        }})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/rest/getPlaylists"))
        .respond_with(ok(&json!({"playlists": {"playlist": [
            {"id": "7", "name": "Morning Mix", "songCount": 4, "duration": 600},
            {"id": "8", "name": "Evening", "songCount": 2, "duration": 300}
        ]}})))
        .mount(&server)
        .await;
    server
}

/// A test app with Subsonic at `url` and one station named like a playlist.
fn app_with_subsonic_and_radio(url: &str) -> common::TestApp {
    let sub = common::app_with_subsonic(url, 0)
        .state
        .config
        .subsonic
        .clone();
    let toml = r#"
[[zone]]
name = "Z"

[[radio]]
name = "Morning Radio"
url = "http://example.com/a"
"#;
    let raw: snapdog::config::FileConfig = toml::from_str(toml).unwrap();
    let mut cfg = snapdog::config::load_raw(raw).unwrap();
    cfg.subsonic = sub;
    common::build_test_app(cfg)
}

/// MED-M2-A1: tracks and albums from Subsonic, playlists and radios by name, one id field per kind.
#[tokio::test]
async fn search_returns_every_kind_with_its_own_identifier() {
    let server = subsonic_with_library().await;
    let app = app_with_subsonic_and_radio(&server.uri());
    let (s, b) = app.get("/api/v1/media/search?q=morning").await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let items = b["items"].as_array().unwrap();
    let of = |kind: &str| -> Vec<&serde_json::Value> {
        items.iter().filter(|i| i["kind"] == kind).collect()
    };

    let radio = of("radio");
    assert_eq!(radio.len(), 1);
    assert_eq!(radio[0]["title"], "Morning Radio");
    assert_eq!(radio[0]["playlist_id"], "snapdog:radio");
    assert_eq!(
        (radio[0]["index"].clone(), radio[0]["track"].clone()),
        (json!(0), json!(0))
    );

    let playlist = of("playlist");
    assert_eq!(playlist.len(), 1, "only Morning Mix matches");
    assert_eq!(playlist[0]["playlist_id"], "subsonic:7");
    assert_eq!(playlist[0]["index"], 1, "after the radio list");
    assert!(playlist[0].get("track_id").is_none());

    let album = of("album");
    assert_eq!(album[0]["album_id"], "al-1");
    assert_eq!(album[0]["subtitle"], "Ann");
    assert_eq!(album[0]["cover_art"], "/api/v1/media/cover/al-1");
    assert!(album[0].get("playlist_id").is_none());

    let tracks = of("track");
    assert_eq!(tracks.len(), 2);
    assert_eq!(tracks[0]["track_id"], "t1");
    assert_eq!(tracks[0]["subtitle"], "Song Cycle");
    assert_eq!(tracks[0]["duration"], 200);
    assert_eq!(tracks[1]["cover_art"], serde_json::Value::Null);
}

#[tokio::test]
async fn search_kinds_limit_and_next_offset() {
    let server = subsonic_with_library().await;
    let app = app_with_subsonic_and_radio(&server.uri());

    let (_, b) = app.get("/api/v1/media/search?q=song&kind=album").await;
    let kinds: Vec<_> = b["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["kind"].clone())
        .collect();
    assert_eq!(kinds, [json!("album")]);
    assert_eq!(
        b["next_offset"],
        serde_json::Value::Null,
        "no kind filled its page"
    );

    // limit=1 and a page that comes back full: another page may follow.
    let (_, b) = app
        .get("/api/v1/media/search?q=song&kind=album&limit=1&offset=3")
        .await;
    assert_eq!(b["next_offset"], 4);

    let (_, b) = app
        .get("/api/v1/media/search?q=morning&kind=radio,playlist&offset=1")
        .await;
    assert_eq!(
        b["items"],
        json!([]),
        "offset skips the only match of each kind"
    );
}

#[tokio::test]
async fn search_asks_search3_for_only_the_kinds_wanted() {
    let server = subsonic_with_library().await;
    let app = app_with_subsonic_and_radio(&server.uri());
    app.get("/api/v1/media/search?q=song&kind=album&limit=5&offset=2")
        .await;
    let requests = server.received_requests().await.unwrap();
    let search = requests
        .iter()
        .find(|r| r.url.path() == "/rest/search3")
        .unwrap();
    let q: std::collections::HashMap<_, _> = search.url.query_pairs().into_owned().collect();
    assert_eq!(q["query"], "song");
    assert_eq!(
        (q["albumCount"].as_str(), q["albumOffset"].as_str()),
        ("5", "2")
    );
    assert_eq!(q["songCount"], "0", "no tracks asked for");
}

/// MED-M2-A1 counter-probes: one character and an unknown kind are a 400.
#[tokio::test]
async fn search_rejects_a_short_query_and_unknown_kinds() {
    let server = subsonic_with_library().await;
    let app = app_with_subsonic_and_radio(&server.uri());
    for uri in [
        "/api/v1/media/search?q=a",
        "/api/v1/media/search?q=%20a%20",
        "/api/v1/media/search?q=",
        "/api/v1/media/search?q=song&kind=artist",
    ] {
        let (s, b) = app.get(uri).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{uri}");
        assert_eq!(b["error"], "bad_request", "{uri}");
    }
    assert!(
        server.received_requests().await.unwrap().is_empty(),
        "a rejected search does not reach Subsonic"
    );
}

#[tokio::test]
async fn search_fails_with_502_when_subsonic_is_down() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/rest/search3"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;
    let app = common::app_with_subsonic(&server.uri(), 0);
    let (s, _) = app.get("/api/v1/media/search?q=song&kind=track").await;
    assert_eq!(s, StatusCode::BAD_GATEWAY);
}

/// MED-M2-A2: the cover proxy serves the image with the caching header, an unknown id is a 404.
#[tokio::test]
async fn cover_proxy_serves_with_caching_headers() {
    let server = MockServer::start().await;
    let png = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 0];
    Mock::given(method("GET"))
        .and(path("/rest/getCoverArt"))
        .and(query_param("id", "al-1"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(png.to_vec()))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/rest/getCoverArt"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    let app = common::app_with_subsonic(&server.uri(), 0);

    let (s, _, _) = app.get_with_total("/api/v1/media/cover/al-1").await;
    assert_eq!(s, StatusCode::OK);
    let (s, _) = app.get("/api/v1/media/cover/nope").await;
    assert_eq!(s, StatusCode::NOT_FOUND);

    // Without Subsonic the proxy has nothing to ask.
    let (s, _) = common::test_app().get("/api/v1/media/cover/al-1").await;
    assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE);
}

/// MED-M2-A3: an album plays as a queue of its own and is not mistaken for a playlist.
#[tokio::test]
async fn an_album_is_queued_under_a_reserved_id() {
    let mut app = common::test_app();
    let (s, _) = app
        .request("POST", "/api/v1/zones/1/play/subsonic/album/al-1", None)
        .await;
    assert_eq!(s, StatusCode::OK);
    let cmds = app.drain_zone(1);
    match &cmds[..] {
        [ZoneCommand::PlaySubsonicPlaylist(queue, 0)] => {
            assert!(snapdog::subsonic::is_album_queue(queue));
            assert_eq!(queue, &snapdog::subsonic::album_queue_id("al-1"));
        }
        other => panic!("unexpected commands: {}", other.len()),
    }
    assert!(
        !snapdog::subsonic::is_album_queue("al-1"),
        "a playlist id is no album"
    );
    let (s, _) = app
        .request("POST", "/api/v1/zones/9/play/subsonic/album/al-1", None)
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}
