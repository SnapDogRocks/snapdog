// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! Exposure of the API documentation. The `OpenAPI` document lists every route, so it
//! is served behind the API key like `/api/v1`. The interactive console (`/docs`) is
//! off unless `[http].api_console` is set, and a build without the `api-docs` feature
//! serves neither. In-process (`build_router` + `oneshot`), no socket.

#![allow(clippy::doc_markdown)] // module doc mentions OpenAPI / Bearer

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use snapdog::api;
use snapdog::config;
use tower::ServiceExt;

const OPENAPI: &str = "/api/v1/openapi.json";

fn app(http: &str) -> common::TestApp {
    let toml = format!("[http]\n{http}\n[[zone]]\nname = \"Ground Floor\"\n");
    common::build_test_app(config::load_str(&toml).expect("test config resolves"))
}

/// Status, content type and body of `GET path`, with an optional Bearer key.
async fn get(
    app: &common::TestApp,
    path: &str,
    bearer: Option<&str>,
) -> (StatusCode, String, String) {
    let mut request = Request::get(path);
    if let Some(key) = bearer {
        request = request.header(header::AUTHORIZATION, format!("Bearer {key}"));
    }
    let response = api::build_router(&app.state)
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (
        status,
        content_type,
        String::from_utf8_lossy(&body).into_owned(),
    )
}

#[cfg(feature = "api-docs")]
mod compiled_in {
    use super::*;

    #[tokio::test]
    async fn openapi_document_needs_the_api_key() {
        let app = app("api_keys = [\"s3cret\"]");

        let (status, _, body) = get(&app, OPENAPI, None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert!(
            !body.contains("openapi"),
            "no document without the key: {body}"
        );

        let (status, _, _) = get(&app, OPENAPI, Some("wrong")).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        // A key of the right length is not enough: the compare is on the value.
        let (status, _, _) = get(&app, OPENAPI, Some("s3creT")).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);

        let (status, content_type, body) = get(&app, OPENAPI, Some("s3cret")).await;
        assert_eq!(status, StatusCode::OK);
        assert!(
            content_type.starts_with("application/json"),
            "{content_type}"
        );
        let document: serde_json::Value = serde_json::from_str(&body).expect("valid JSON");
        assert_eq!(document["info"]["title"], "SnapDog REST API");
        assert!(document["paths"]["/api/v1/zones"].is_object());
    }

    #[tokio::test]
    async fn served_document_is_the_checked_in_contract() {
        let app = app("api_keys = [\"s3cret\"]");
        let (_, _, body) = get(&app, OPENAPI, Some("s3cret")).await;
        let served: serde_json::Value = serde_json::from_str(&body).unwrap();
        let checked_in: serde_json::Value =
            serde_json::from_str(include_str!("../../openapi.json")).unwrap();
        assert_eq!(served, checked_in);
    }

    #[tokio::test]
    async fn without_configured_keys_the_document_is_open_like_the_api() {
        let app = app("");
        let (status, _, _) = get(&app, "/api/v1/zones", None).await;
        assert_eq!(
            status,
            StatusCode::OK,
            "the API itself is open without keys"
        );
        let (status, _, body) = get(&app, OPENAPI, None).await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.contains("SnapDog REST API"));
    }

    #[tokio::test]
    async fn api_docs_false_serves_no_document() {
        let app = app("api_docs = false\napi_keys = [\"s3cret\"]\napi_console = true");
        let (status, _, _) = get(&app, OPENAPI, Some("s3cret")).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        // The console needs the document, so it is off as well.
        let (status, _, _) = get(&app, "/docs", None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn the_console_is_off_by_default() {
        let app = app("api_keys = [\"s3cret\"]");
        for path in ["/docs", "/docs/"] {
            let (status, _, body) = get(&app, path, None).await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{path}");
            assert!(!body.contains("scalar"), "{path}: {body}");
        }
        assert!(!config::HttpConfig::default().api_console);
    }

    #[tokio::test]
    async fn the_console_is_served_when_it_is_switched_on() {
        let app = app("api_console = true");
        let (status, content_type, body) = get(&app, "/docs", None).await;
        assert_eq!(status, StatusCode::OK);
        assert!(content_type.starts_with("text/html"), "{content_type}");
        assert!(
            body.contains("SnapDog REST API"),
            "the page embeds the document: {}",
            &body[..body.len().min(200)]
        );
    }
}

#[cfg(not(feature = "api-docs"))]
mod compiled_out {
    use super::*;

    #[tokio::test]
    async fn nothing_is_served_without_the_api_docs_feature() {
        // Even with every switch on, a build without the feature serves no documentation.
        let app = app("api_docs = true\napi_console = true\napi_keys = [\"s3cret\"]");
        for (path, bearer) in [(OPENAPI, Some("s3cret")), ("/docs", None)] {
            let (status, _, body) = get(&app, path, bearer).await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{path}");
            assert!(!body.contains("openapi"), "{path}: {body}");
        }
    }
}

/// An unknown API path is a 404, not the web UI's index page.
#[tokio::test]
async fn unknown_api_paths_are_not_answered_with_the_web_ui() {
    let app = app("");
    // `/api-docs/openapi.json` is the path utoipa documents for the spec; it is not served here.
    for path in ["/api/v1/no-such-route", "/api-docs/openapi.json"] {
        let (status, _, body) = get(&app, path, None).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{path}");
        assert!(
            !body.contains("<html"),
            "{path} answered with a page: {body}"
        );
    }
}
