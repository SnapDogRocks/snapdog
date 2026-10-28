// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! The interactive API console is served entirely from the binary: its script is the pinned,
//! vendored Scalar bundle (`assets/scalar/NOTICE.md`), the page names no other host, and a
//! Content-Security-Policy stops the page from contacting one. In-process, no socket.

#![cfg(feature = "api-docs")]
#![allow(clippy::doc_markdown)] // module doc mentions OpenAPI / Scalar

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use snapdog::api;
use snapdog::api::console;
use snapdog::config;
use tower::ServiceExt;

/// SHA-256 of the file as served, the gzip-compressed bundle in `assets/scalar`.
const GZ_SHA256: &str = "a7222f8c72a0ec4eed38cafff7120449a060fce83103c6c0936fd2fefe56afb5";
/// SHA-256 of the original `standalone.js` of `@scalar/api-reference` 1.74.0 on cdn.jsdelivr.net.
const UPSTREAM_SHA256: &str = "ea680608a3edc8fc3a27d6259d99ae8884d7061ee0149b166c01b45edb17bb73";
const VERSION: &str = "1.74.0";

/// SHA-256 through the TLS library already in the binary, so the test needs no new dependency.
fn sha256_hex(data: &[u8]) -> String {
    let suite = rustls::crypto::aws_lc_rs::cipher_suite::TLS13_AES_128_GCM_SHA256
        .tls13()
        .expect("a TLS 1.3 suite");
    let digest = suite.common.hash_provider.hash(data);
    digest.as_ref().iter().fold(String::new(), |mut hex, byte| {
        use std::fmt::Write;
        let _ = write!(hex, "{byte:02x}");
        hex
    })
}

struct Reply {
    status: StatusCode,
    headers: axum::http::HeaderMap,
    body: Vec<u8>,
}

async fn get(app: &common::TestApp, path: &str, accept_encoding: Option<&str>) -> Reply {
    let mut request = Request::get(path);
    if let Some(value) = accept_encoding {
        request = request.header(header::ACCEPT_ENCODING, value);
    }
    let response = api::build_router(&app.state)
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap()
        .to_vec();
    Reply {
        status,
        headers,
        body,
    }
}

fn app(http: &str) -> common::TestApp {
    let toml = format!("[http]\n{http}\n[[zone]]\nname = \"Ground Floor\"\n");
    common::build_test_app(config::load_str(&toml).expect("test config resolves"))
}

fn text(reply: &Reply) -> String {
    String::from_utf8_lossy(&reply.body).into_owned()
}

#[test]
fn the_vendored_bundle_is_the_pinned_one() {
    assert_eq!(console::SCALAR_VERSION, VERSION);
    assert_eq!(sha256_hex(console::script_gz()), GZ_SHA256);
    assert_eq!(console::script_gz().len(), 1_318_345);
    assert_eq!(
        console::script_gz()[..2],
        [0x1f, 0x8b],
        "the embedded file is gzip"
    );
}

/// The compressed file really is the upstream file: decompress it and hash that.
#[cfg(unix)]
#[test]
fn the_bundle_decompresses_to_the_upstream_file() {
    use std::io::Write;
    use std::process::{Command, Stdio};

    let mut child = Command::new("gzip")
        .args(["-dc"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("gzip is installed");
    let mut stdin = child.stdin.take().unwrap();
    let compressed = console::script_gz();
    let feeder = std::thread::spawn(move || stdin.write_all(compressed).unwrap());
    let output = child.wait_with_output().unwrap();
    feeder.join().unwrap();
    assert!(output.status.success());
    assert_eq!(output.stdout.len(), 4_571_209);
    assert_eq!(sha256_hex(&output.stdout), UPSTREAM_SHA256);

    // Nothing in the script points at the CDN the stock template used.
    let script = String::from_utf8_lossy(&output.stdout);
    assert!(!script.contains("cdn.jsdelivr.net"));
}

#[tokio::test]
async fn the_script_is_served_from_the_binary_with_the_pinned_hash() {
    let app = app("api_console = true");
    let reply = get(&app, "/docs/assets/scalar.js", Some("gzip, deflate, br")).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.headers[header::CONTENT_ENCODING], "gzip");
    assert!(
        reply.headers[header::CONTENT_TYPE]
            .to_str()
            .unwrap()
            .starts_with("text/javascript")
    );
    assert_eq!(reply.headers[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
    assert_eq!(
        sha256_hex(&reply.body),
        GZ_SHA256,
        "what is sent is the pinned file"
    );
}

#[tokio::test]
async fn a_client_without_gzip_is_told_so() {
    let app = app("api_console = true");
    for accept in [None, Some("identity"), Some("br"), Some("gzip;q=0")] {
        let reply = get(&app, "/docs/assets/scalar.js", accept).await;
        assert_eq!(reply.status, StatusCode::NOT_ACCEPTABLE, "{accept:?}");
        // The body of the refusal is itself compressed when the client asks for brotli.
        if reply.headers.get(header::CONTENT_ENCODING).is_none() {
            assert!(text(&reply).contains("gzip"), "{accept:?}");
        }
        assert_ne!(
            sha256_hex(&reply.body),
            GZ_SHA256,
            "{accept:?}: the bundle is not sent"
        );
    }
}

/// The page with the embedded OpenAPI document (a script element of type
/// application/json) cut out, which is the part that has to be free of other hosts.
fn page_without_document(html: &str) -> String {
    let start = html
        .find("type=\"application/json\"")
        .expect("the document element");
    let after = html[start..].find("</script>").expect("its end") + start;
    let open_end = html[start..].find('>').unwrap() + start + 1;
    format!("{}{}", &html[..open_end], &html[after..])
}

#[tokio::test]
async fn the_page_names_no_other_host_and_the_policy_forbids_them() {
    let app = app("api_console = true");
    let reply = get(&app, "/docs", None).await;
    assert_eq!(reply.status, StatusCode::OK);
    let html = text(&reply);
    let page = page_without_document(&html);

    // No absolute URL anywhere in the page itself, so no script, stylesheet, font or
    // preconnect that points at another host.
    assert!(
        !page.contains("http://") && !page.contains("https://"),
        "{page}"
    );
    assert!(
        !page.contains("//cdn.") && !page.contains("jsdelivr"),
        "{page}"
    );
    // Every src and href is a path on this server.
    for attribute in ["src=\"", "href=\""] {
        for (at, _) in page.match_indices(attribute) {
            let value = &page[at + attribute.len()..];
            assert!(
                value.starts_with('/'),
                "{attribute}{}",
                &value[..value.len().min(40)]
            );
        }
    }
    assert!(page.contains("src=\"/docs/assets/scalar.js\""), "{page}");
    // Scalar's own outgoing requests: fonts and telemetry are off.
    assert!(page.contains("\"withDefaultFonts\":false"), "{page}");
    assert!(page.contains("\"telemetry\":false"), "{page}");
    // ... and so are the parts that talk to Scalar's services or link to them.
    assert!(page.contains("\"agent\":{\"disabled\":true}"), "{page}");
    assert!(page.contains("\"mcp\":{\"disabled\":true}"), "{page}");
    assert!(page.contains("\"showDeveloperTools\":\"never\""), "{page}");
    // The document is in the page once, and the loader script is the only other script.
    assert_eq!(
        html.matches("</script>").count(),
        2,
        "a stray </script> in the document?"
    );

    let policy = reply.headers[header::CONTENT_SECURITY_POLICY]
        .to_str()
        .unwrap();
    assert!(policy.contains("default-src 'none'"), "{policy}");
    assert!(policy.contains("connect-src 'self'"), "{policy}");
    assert!(policy.contains("script-src 'self'"), "{policy}");
    assert!(!policy.contains("http"), "{policy}");
    assert!(!policy.contains('*'), "{policy}");
    assert_eq!(reply.headers[header::REFERRER_POLICY], "no-referrer");
    assert_eq!(reply.headers[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
}

#[tokio::test]
async fn the_licence_notice_is_served_next_to_the_script() {
    let app = app("api_console = true");
    let reply = get(&app, "/docs/assets/NOTICE.md", None).await;
    assert_eq!(reply.status, StatusCode::OK);
    let notice = text(&reply);
    for needle in [
        "MIT License",
        "Copyright (c) 2023-present Scalar",
        VERSION,
        GZ_SHA256,
        UPSTREAM_SHA256,
    ] {
        assert!(notice.contains(needle), "{needle}");
    }
}

#[tokio::test]
async fn nothing_of_the_console_exists_while_it_is_off() {
    for http in [
        "",
        "api_console = false",
        "api_docs = false\napi_console = true",
    ] {
        let app = app(http);
        for path in [
            "/docs",
            "/docs/",
            "/docs/assets/scalar.js",
            "/docs/assets/NOTICE.md",
        ] {
            let reply = get(&app, path, Some("gzip")).await;
            assert_eq!(reply.status, StatusCode::NOT_FOUND, "{http:?} {path}");
            assert!(
                reply.body.len() < 1000,
                "{http:?} {path}: {} bytes",
                reply.body.len()
            );
        }
    }
}

#[test]
fn the_pinned_version_is_the_same_everywhere() {
    let notice = include_str!("../assets/scalar/NOTICE.md");
    assert!(notice.contains(&format!("standalone-{VERSION}.js.gz")));
    assert!(notice.contains(&format!("@scalar/api-reference@{VERSION}/")));
    assert!(
        std::path::Path::new(&format!(
            "{}/assets/scalar/standalone-{VERSION}.js.gz",
            env!("CARGO_MANIFEST_DIR")
        ))
        .is_file()
    );
    assert!(include_str!("../src/api/console.rs").contains(&format!("standalone-{VERSION}.js.gz")));
}
