// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! The interactive API console (Scalar), served from the binary.
//!
//! The page and its script come from `SnapDog` itself: the Scalar browser bundle is embedded
//! (gzip-compressed, see `assets/scalar/NOTICE.md` for version, hashes and licence) and no
//! other host is contacted for it. A `Content-Security-Policy` keeps it that way for
//! everything the bundle itself might fetch. Only mounted when `[http].api_console` is set.

use std::sync::OnceLock;

use axum::Router;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::IntoResponse;
use axum::routing::get;
use utoipa::OpenApi;
use utoipa_scalar::Scalar;

use super::openapi::ApiDoc;

/// Version of the embedded `@scalar/api-reference` bundle.
pub const SCALAR_VERSION: &str = "1.74.0";

/// The bundle, `gzip -9 -n`. Its SHA-256 is pinned in `tests/api_console.rs`.
const SCRIPT_GZ: &[u8] = include_bytes!("../../assets/scalar/standalone-1.74.0.js.gz");

/// Licence and provenance of the bundle.
const NOTICE: &str = include_str!("../../assets/scalar/NOTICE.md");

/// Page template: `$title` and `$spec` are filled in by `utoipa-scalar`.
const TEMPLATE: &str = include_str!("../../assets/scalar/console.html");

/// What the page may load and contact: itself, nothing else. `unsafe-inline` is needed for
/// the inline `OpenAPI` document and for the styles the bundle injects.
const CONTENT_SECURITY_POLICY: &str = "default-src 'none'; script-src 'self' 'unsafe-inline'; \
    style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; font-src 'self' data:; \
    connect-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'";

/// `/docs` and the files it needs. Paths are literals: the route scanner in
/// `tests/openapi_contract.rs` reads them.
pub fn router() -> Router {
    Router::new()
        .route("/docs", get(page))
        .route("/docs/assets/scalar.js", get(script))
        .route("/docs/assets/NOTICE.md", get(notice))
}

fn hardened(mut headers: HeaderMap) -> HeaderMap {
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(CONTENT_SECURITY_POLICY),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    headers
}

/// The console page with the `OpenAPI` document embedded.
async fn page() -> impl IntoResponse {
    static HTML: OnceLock<String> = OnceLock::new();
    let html = HTML.get_or_init(|| {
        Scalar::new(ApiDoc::openapi())
            .custom_html(TEMPLATE)
            .title("SnapDog API")
            .to_html()
    });
    let mut headers = hardened(HeaderMap::new());
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"),
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    (headers, html.clone())
}

/// Whether an `Accept-Encoding` value allows gzip (`gzip`, `*`, not `gzip;q=0`).
fn accepts_gzip(value: &str) -> bool {
    value.split(',').any(|part| {
        let mut pieces = part.split(';');
        let coding = pieces.next().unwrap_or("").trim().to_ascii_lowercase();
        let refused = pieces.any(|p| {
            p.trim()
                .strip_prefix("q=")
                .is_some_and(|q| q.trim().parse::<f32>().is_ok_and(|q| q <= 0.0))
        });
        (coding == "gzip" || coding == "*") && !refused
    })
}

/// The embedded bundle, still gzip-compressed. Every browser accepts gzip, so it is sent
/// as it is; a client that does not gets 406 rather than a decompressed copy, because
/// decompressing would need a dependency for a client no console user has.
async fn script(request_headers: HeaderMap) -> impl IntoResponse {
    let allowed = request_headers
        .get(header::ACCEPT_ENCODING)
        .and_then(|v| v.to_str().ok())
        .is_some_and(accepts_gzip);
    let mut headers = hardened(HeaderMap::new());
    headers.insert(header::VARY, HeaderValue::from_static("Accept-Encoding"));
    if !allowed {
        return (
            StatusCode::NOT_ACCEPTABLE,
            headers,
            "The API console script is served gzip-compressed only; send Accept-Encoding: gzip.\n"
                .as_bytes()
                .to_vec(),
        );
    }
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/javascript; charset=utf-8"),
    );
    headers.insert(header::CONTENT_ENCODING, HeaderValue::from_static("gzip"));
    // The file only changes with the binary.
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=3600"),
    );
    (StatusCode::OK, headers, SCRIPT_GZ.to_vec())
}

/// Licence and provenance of the bundle (the MIT licence asks for the notice to travel
/// with the software).
async fn notice() -> impl IntoResponse {
    let mut headers = hardened(HeaderMap::new());
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/plain; charset=utf-8"),
    );
    (headers, NOTICE)
}

/// The embedded, still compressed bundle, for tests.
#[must_use]
pub const fn script_gz() -> &'static [u8] {
    SCRIPT_GZ
}

#[cfg(test)]
mod tests {
    use super::accepts_gzip;

    #[test]
    fn accept_encoding_is_read_like_http_says() {
        for yes in [
            "gzip",
            "gzip, deflate, br",
            "br;q=1, gzip;q=0.5",
            "GZIP",
            "*",
            "deflate, *;q=0.1",
        ] {
            assert!(accepts_gzip(yes), "{yes}");
        }
        for no in [
            "",
            "identity",
            "br, deflate",
            "gzip;q=0",
            "gzip; q=0.0, br",
            "x-gzip",
        ] {
            assert!(!accepts_gzip(no), "{no}");
        }
    }
}
