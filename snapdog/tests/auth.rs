// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! IT-T14 — API auth middleware: protected routes require a Bearer API key (401
//! without/with a wrong key), and `/health` stays unauthenticated. Tier-1,
//! in-process (build_router + tower::oneshot, with a custom Authorization header).

#![allow(clippy::doc_markdown)] // doc mentions API / Bearer / SecretString

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use snapdog::api::{self, SharedState};
use snapdog::config::{self, FileConfig};
use tower::ServiceExt;

/// A 2-zone/2-client app whose config requires the API key `s3cret`. (api_keys is
/// `Vec<SecretString>`; building via TOML lets serde construct it.)
fn app_with_key() -> common::TestApp {
    let toml = r#"
[http]
api_keys = ["s3cret"]

[[zone]]
name = "Ground Floor"
[[zone]]
name = "1st Floor"

[[client]]
name = "Living Room"
mac = "02:42:ac:11:00:10"
zone = "Ground Floor"

[[client]]
name = "Kitchen"
mac = "02:42:ac:11:00:11"
zone = "1st Floor"
"#;
    let raw: FileConfig = toml::from_str(toml).expect("auth test TOML parses");
    common::build_test_app(config::load_raw(raw).expect("config resolves"))
}

async fn zones_status(state: &SharedState, bearer: Option<&str>) -> StatusCode {
    let mut req = Request::get("/api/v1/zones");
    if let Some(token) = bearer {
        req = req.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    api::build_router(state)
        .oneshot(req.body(Body::empty()).unwrap())
        .await
        .unwrap()
        .status()
}

#[tokio::test]
async fn protected_route_requires_api_key() {
    let app = app_with_key();
    assert_eq!(
        zones_status(&app.state, None).await,
        StatusCode::UNAUTHORIZED,
        "no key → 401"
    );
    assert_eq!(
        zones_status(&app.state, Some("wrong")).await,
        StatusCode::UNAUTHORIZED,
        "wrong key → 401"
    );
    assert_eq!(
        zones_status(&app.state, Some("s3cret")).await,
        StatusCode::OK,
        "correct Bearer key → 200"
    );
}

#[tokio::test]
async fn health_is_unauthenticated() {
    // /health is merged at the root, outside the auth layer.
    let app = app_with_key();
    let resp = api::build_router(&app.state)
        .oneshot(Request::get("/health").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}
