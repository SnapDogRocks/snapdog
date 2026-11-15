// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! Embedded `WebUI` static asset serving.

use axum::body::Body;
use axum::http::{Request, Response, StatusCode, header};
use axum::response::IntoResponse;
use rust_embed::Embed;

use crate::config::WebUi;

#[derive(Embed)]
#[folder = "../webui/out"]
struct WebAssets;

const HTML_CACHE_CONTROL: &str = "no-cache";
const IMMUTABLE_CACHE_CONTROL: &str = "public, max-age=31536000, immutable";

fn mime_for(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" => "application/javascript; charset=utf-8",
        "json" => "application/json; charset=utf-8",
        "webmanifest" => "application/manifest+json; charset=utf-8",
        "png" => "image/png",
        "svg" => "image/svg+xml",
        "ico" => "image/x-icon",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        "txt" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

const fn html_entry(webui: WebUi) -> &'static str {
    match webui {
        WebUi::Current => "index.html",
        WebUi::Legacy => "legacy.html",
    }
}

/// Resolve the two HTML endpoints whose contents depend on the configured `WebUI`.
/// Other paths remain exact static asset paths.
fn asset_path_for_request(path: &str, webui: WebUi) -> &str {
    match path {
        "" | "index.html" => html_entry(webui),
        "legacy" | "legacy/" => "legacy.html",
        _ => path,
    }
}

fn has_hex_hash(filename: &str) -> bool {
    let stem = filename.rsplit_once('.').map_or(filename, |(stem, _)| stem);
    stem.split(['-', '.'])
        .any(|part| part.len() >= 8 && part.bytes().all(|byte| byte.is_ascii_hexdigit()))
}

fn cache_control_for_path(path: &str) -> &'static str {
    if std::path::Path::new(path)
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("html"))
    {
        HTML_CACHE_CONTROL
    } else if path
        .strip_prefix("_next/static/")
        .and_then(|rest| rest.rsplit('/').next())
        .is_some_and(has_hex_hash)
    {
        IMMUTABLE_CACHE_CONTROL
    } else {
        HTML_CACHE_CONTROL
    }
}

fn serve_asset(path: &str) -> Option<Response<Body>> {
    let file = WebAssets::get(path)?;
    Some(
        Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, mime_for(path))
            .header(header::CACHE_CONTROL, cache_control_for_path(path))
            .body(Body::from(file.data.to_vec()))
            .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response()),
    )
}

/// Paths the single-page app must never answer: an unknown API route or a disabled
/// API console is a 404, not the web UI's `index.html` with status 200.
fn is_reserved(path: &str) -> bool {
    ["api", "api-docs", "docs"]
        .iter()
        .any(|root| path == *root || path.strip_prefix(root).is_some_and(|r| r.starts_with('/')))
}

/// Missing Next.js and public assets are not client-side routes.
fn is_asset_path(path: &str) -> bool {
    path == "_next" || path.starts_with("_next/") || path == "assets" || path.starts_with("assets/")
}

/// Axum fallback handler: serves embedded assets or the configured SPA entry.
pub fn fallback(req: &Request<Body>, webui: WebUi) -> Response<Body> {
    let path = req.uri().path().trim_start_matches('/');

    if is_reserved(path) {
        return StatusCode::NOT_FOUND.into_response();
    }

    let asset_path = asset_path_for_request(path, webui);
    if let Some(response) = serve_asset(asset_path) {
        return response;
    }

    if is_asset_path(path) {
        return StatusCode::NOT_FOUND.into_response();
    }

    // SPA fallback: serve the selected entry for client-side routes.
    serve_asset(html_entry(webui)).unwrap_or_else(|| StatusCode::NOT_FOUND.into_response())
}

#[cfg(test)]
mod tests {
    use super::{
        HTML_CACHE_CONTROL, IMMUTABLE_CACHE_CONTROL, WebAssets, asset_path_for_request,
        cache_control_for_path, fallback, has_hex_hash, html_entry, is_asset_path, is_reserved,
        mime_for,
    };
    use crate::config::WebUi;
    use axum::body::{Body, to_bytes};
    use axum::http::{Request, StatusCode, header};

    fn request(uri: &str) -> Request<Body> {
        Request::builder().uri(uri).body(Body::empty()).unwrap()
    }

    #[test]
    fn web_manifest_has_its_own_mime_type() {
        assert_eq!(
            mime_for("manifest.webmanifest"),
            "application/manifest+json; charset=utf-8"
        );
        assert_eq!(mime_for("favicon.ico"), "image/x-icon");
    }

    #[test]
    fn root_and_index_html_follow_the_selected_mode() {
        assert_eq!(asset_path_for_request("", WebUi::Current), "index.html");
        assert_eq!(
            asset_path_for_request("index.html", WebUi::Current),
            "index.html"
        );
        assert_eq!(asset_path_for_request("", WebUi::Legacy), "legacy.html");
        assert_eq!(
            asset_path_for_request("index.html", WebUi::Legacy),
            "legacy.html"
        );
    }

    #[test]
    fn legacy_url_and_exact_files_use_the_shared_asset_bundle() {
        assert_eq!(
            asset_path_for_request("legacy", WebUi::Current),
            "legacy.html"
        );
        assert_eq!(
            asset_path_for_request("legacy/", WebUi::Current),
            "legacy.html"
        );
        assert_eq!(
            asset_path_for_request("_next/static/chunks/app.js", WebUi::Legacy),
            "_next/static/chunks/app.js"
        );
        assert_eq!(
            asset_path_for_request("legacy.html", WebUi::Current),
            "legacy.html"
        );
    }

    #[test]
    fn api_and_docs_paths_are_never_the_spa() {
        for path in [
            "api",
            "api/v1/nothing",
            "api/v1/openapi.json",
            "api-docs",
            "api-docs/openapi.json",
            "docs",
            "docs/x",
        ] {
            assert!(is_reserved(path), "{path}");
        }
        for path in [
            "",
            "index.html",
            "apis",
            "docsify",
            "zones/api",
            "documents",
        ] {
            assert!(!is_reserved(path), "{path}");
        }
    }

    #[test]
    fn absent_asset_prefixes_do_not_fall_back_to_html() {
        for path in [
            "_next",
            "_next/static/chunks/missing.js",
            "assets",
            "assets/icons/missing.svg",
        ] {
            assert!(is_asset_path(path), "{path}");
        }
        assert!(!is_asset_path("zones/kitchen"));
    }

    #[test]
    fn only_hashed_next_static_assets_are_immutable() {
        assert!(has_hex_hash("page-0123456789abcdef.js"));
        assert!(has_hex_hash("0123456789abcdef.css"));
        assert!(!has_hex_hash("page.js"));
        assert!(!has_hex_hash("page-deadbee.js"));
        assert_eq!(cache_control_for_path("index.html"), HTML_CACHE_CONTROL);
        assert_eq!(
            cache_control_for_path("_next/static/chunks/page-0123456789abcdef.js"),
            IMMUTABLE_CACHE_CONTROL
        );
        assert_eq!(
            cache_control_for_path("_next/static/chunks/page.js"),
            HTML_CACHE_CONTROL
        );
        assert_eq!(
            cache_control_for_path("assets/logo.svg"),
            HTML_CACHE_CONTROL
        );
    }

    #[tokio::test]
    async fn fallback_serves_the_configured_entry_and_legacy_alias() {
        for webui in [WebUi::Current, WebUi::Legacy] {
            let selected = html_entry(webui);
            for (uri, expected) in [
                ("/", selected),
                ("/index.html", selected),
                ("/legacy", "legacy.html"),
                ("/legacy/", "legacy.html"),
            ] {
                let response = fallback(&request(uri), webui);
                assert_eq!(response.status(), StatusCode::OK, "{uri} in {webui:?}");
                assert_eq!(
                    response.headers()[header::CACHE_CONTROL],
                    HTML_CACHE_CONTROL,
                    "{uri} in {webui:?}"
                );
                let expected = WebAssets::get(expected).unwrap();
                let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
                assert_eq!(body.as_ref(), expected.data.as_ref(), "{uri} in {webui:?}");
            }
        }
    }

    #[test]
    fn unknown_reserved_and_asset_paths_return_404() {
        for uri in [
            "/api/v1/missing",
            "/docs/missing",
            "/_next/static/chunks/missing.js",
            "/assets/icons/missing.svg",
        ] {
            let response = fallback(&request(uri), WebUi::Current);
            assert_eq!(response.status(), StatusCode::NOT_FOUND, "{uri}");
        }
    }
}
