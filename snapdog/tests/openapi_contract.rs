// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! IT-T14 — OpenAPI document contract. Pure (no harness, no socket); asserts the
//! generated spec stays structurally stable, so a dropped route or renamed schema
//! on a refactor/upgrade fails the build — without being brittle to map ordering or
//! pinning an exact operation count.

#![allow(clippy::doc_markdown)] // module doc mentions OpenAPI / REST

use snapdog::api::openapi::ApiDoc;
use utoipa::OpenApi;

#[test]
fn openapi_doc_is_structurally_stable() {
    let doc: serde_json::Value =
        serde_json::to_value(ApiDoc::openapi()).expect("openapi serializes");

    let checked_in: serde_json::Value = serde_json::from_str(include_str!("../../openapi.json"))
        .expect("checked-in openapi.json is valid JSON");
    assert_eq!(
        doc, checked_in,
        "generated OpenAPI contract differs from openapi.json; run \
         `cargo run --package xtask -- gen-api-spec openapi.json` and commit the result"
    );

    assert_eq!(doc["openapi"], "3.1.0");
    assert_eq!(doc["info"]["title"], "SnapDog REST API");
    assert_eq!(doc["info"]["version"], "1.0.0");

    let paths = doc["paths"].as_object().expect("paths object");
    for must in [
        "/api/v1/system/status",
        "/api/v1/zones",
        "/api/v1/clients",
        "/api/v1/media/playlists",
    ] {
        assert!(paths.contains_key(must), "missing path {must}");
    }

    // Floor below the current total (92 at time of writing) so a dropped route
    // surfaces, without pinning the exact number (which churns on every new route).
    let op_count: usize = paths
        .values()
        .map(|item| {
            item.as_object().map_or(0, |m| {
                ["get", "put", "post", "delete", "patch"]
                    .iter()
                    .filter(|verb| m.contains_key(**verb))
                    .count()
            })
        })
        .sum();
    assert!(
        op_count >= 85,
        "OpenAPI operation count regressed to {op_count} (< 85)"
    );

    let schemas = doc["components"]["schemas"]
        .as_object()
        .expect("component schemas");
    for s in [
        "SystemStatus",
        "VersionInfo",
        "ErrorBody",
        "EqConfig",
        "ZoneInfo",
        "ClientInfo",
        "TrackMetadata",
    ] {
        assert!(schemas.contains_key(s), "missing schema {s}");
    }
}

// ── Registered routes versus the spec ──────────────────────────────────────
//
// axum cannot list the routes of a built `Router`, so the route tables are read
// from the source: every `.route("<path>", get(..).put(..))` in `api/routes/*.rs`
// becomes an operation, prefixed with the path the router is nested under in
// `api/mod.rs`. The spec has to list exactly those operations, so a route added
// without a `#[utoipa::path]` (or documented but no longer served) fails here.

/// `(file, router function, nest prefix)`. A `.route` in a function missing here
/// fails the test, so a new router cannot slip past it.
const ROUTERS: &[(&str, &str, &str)] = &[
    ("zones.rs", "router", "/api/v1/zones"),
    ("eq.rs", "router", "/api/v1/zones"),
    ("clients.rs", "router", "/api/v1/clients"),
    ("client_eq.rs", "router", "/api/v1/clients"),
    ("speakers.rs", "client_speaker_router", "/api/v1/clients"),
    ("speakers.rs", "speakers_router", "/api/v1/speakers"),
    ("media.rs", "router", "/api/v1/media"),
    ("system.rs", "router", "/api/v1/system"),
    ("knx.rs", "router", "/api/v1/knx"),
];

const ROUTE_SOURCES: &[(&str, &str)] = &[
    ("zones.rs", include_str!("../src/api/routes/zones.rs")),
    ("eq.rs", include_str!("../src/api/routes/eq.rs")),
    ("clients.rs", include_str!("../src/api/routes/clients.rs")),
    (
        "client_eq.rs",
        include_str!("../src/api/routes/client_eq.rs"),
    ),
    ("speakers.rs", include_str!("../src/api/routes/speakers.rs")),
    ("media.rs", include_str!("../src/api/routes/media.rs")),
    ("system.rs", include_str!("../src/api/routes/system.rs")),
    ("knx.rs", include_str!("../src/api/routes/knx.rs")),
];

const API_MOD: &str = include_str!("../src/api/mod.rs");

const VERBS: [&str; 5] = ["get", "post", "put", "delete", "patch"];

/// `{zone_index}` and `{client_index}` are the same route shape for this comparison.
fn normalise(path: &str) -> String {
    let mut out = String::new();
    let mut in_param = false;
    for c in path.chars() {
        match c {
            '{' => {
                in_param = true;
                out.push_str("{}");
            }
            '}' => in_param = false,
            _ if in_param => {}
            _ => out.push(c),
        }
    }
    out
}

/// Every `(VERB, path)` registered by the `.route(..)` calls in one source file.
fn registered_in(file: &str, source: &str) -> Vec<(String, String)> {
    let mut found = Vec::new();
    let bytes = source.as_bytes();
    let mut search_from = 0;

    while let Some(offset) = source[search_from..].find(".route(") {
        let call = search_from + offset;
        search_from = call + ".route(".len();

        // Which router function is this call in? The nearest `fn` header above it.
        let header = source[..call]
            .lines()
            .rev()
            .find_map(|l| {
                let l = l.trim_start();
                let l = l.strip_prefix("pub ").unwrap_or(l);
                l.strip_prefix("fn ")
                    .map(|rest| rest.split('(').next().unwrap_or("").to_string())
            })
            .unwrap_or_default();
        let prefix = ROUTERS
            .iter()
            .find(|(f, func, _)| *f == file && *func == header)
            .unwrap_or_else(|| panic!("{file}: `.route` in fn `{header}` is not listed in ROUTERS"))
            .2;

        // `"<path>"`, then the handler chain up to the call's closing parenthesis.
        let rest = &source[search_from..];
        let open = rest.find('"').expect("route path literal") + 1;
        let close = open + rest[open..].find('"').expect("end of route path");
        let path = &rest[open..close];

        let mut depth = 1;
        let mut end = search_from;
        for (i, b) in bytes[search_from..].iter().enumerate() {
            match b {
                b'(' => depth += 1,
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        end = search_from + i;
                        break;
                    }
                }
                _ => {}
            }
        }
        let chain = &source[search_from + close + 1..end];

        // Verbs are the identifiers `get(`, `post(`, ... at call depth 1.
        let mut depth = 0;
        let chars: Vec<char> = chain.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            match chars[i] {
                '(' => depth += 1,
                ')' => depth -= 1,
                c if depth == 0 && c.is_ascii_lowercase() => {
                    let start = i;
                    while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                        i += 1;
                    }
                    let word: String = chars[start..i].iter().collect();
                    if chars.get(i) == Some(&'(') && VERBS.contains(&word.as_str()) {
                        let full = if path == "/" {
                            prefix.to_string()
                        } else {
                            format!("{prefix}{path}")
                        };
                        found.push((word.to_uppercase(), normalise(&full)));
                    }
                    continue;
                }
                _ => {}
            }
            i += 1;
        }
    }
    found
}

fn registered_operations() -> std::collections::BTreeSet<(String, String)> {
    ROUTE_SOURCES
        .iter()
        .flat_map(|(file, source)| registered_in(file, source))
        .collect()
}

fn documented_operations() -> std::collections::BTreeSet<(String, String)> {
    let doc = serde_json::to_value(ApiDoc::openapi()).unwrap();
    doc["paths"]
        .as_object()
        .unwrap()
        .iter()
        .flat_map(|(path, item)| {
            item.as_object()
                .unwrap()
                .keys()
                .filter(|verb| VERBS.contains(&verb.as_str()))
                .map(|verb| (verb.to_uppercase(), normalise(path)))
                .collect::<Vec<_>>()
        })
        .collect()
}

#[test]
fn every_registered_route_is_in_the_spec_and_vice_versa() {
    let registered = registered_operations();
    let documented = documented_operations();

    let undocumented: Vec<_> = registered.difference(&documented).collect();
    let phantom: Vec<_> = documented.difference(&registered).collect();
    assert!(
        undocumented.is_empty(),
        "routes served but missing from the OpenAPI spec (add a #[utoipa::path] and list it \
         in api/openapi.rs): {undocumented:#?}"
    );
    assert!(
        phantom.is_empty(),
        "operations in the OpenAPI spec that no router serves: {phantom:#?}"
    );
    assert!(
        registered.len() >= 95,
        "scanner found only {}",
        registered.len()
    );
}

#[test]
fn the_route_scanner_sees_the_update_routes_and_multi_verb_chains() {
    let registered = registered_operations();
    for (verb, path) in [
        ("GET", "/api/v1/system/updates"),
        ("POST", "/api/v1/system/updates/check"),
        ("POST", "/api/v1/system/updates/apply"),
        ("POST", "/api/v1/system/updates/config"),
        ("GET", "/api/v1/zones/{}/volume"),
        ("PUT", "/api/v1/zones/{}/volume"),
        ("GET", "/api/v1/zones"),
        ("PUT", "/api/v1/clients/{}/speaker"),
    ] {
        assert!(
            registered.contains(&(verb.to_string(), path.to_string())),
            "{verb} {path} not found by the scanner"
        );
    }
}

#[test]
fn router_table_matches_the_files_and_the_nesting() {
    // Every route file is scanned...
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/src/api/routes");
    let mut files: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .filter(|f| f != "mod.rs")
        .collect();
    files.sort();
    let mut scanned: Vec<String> = ROUTE_SOURCES
        .iter()
        .map(|(f, _)| (*f).to_string())
        .collect();
    scanned.sort();
    assert_eq!(files, scanned, "a route file is missing from ROUTE_SOURCES");

    // ...and the prefixes are the ones api/mod.rs nests the routers under.
    let compact: String = API_MOD.chars().filter(|c| !c.is_whitespace()).collect();
    for (file, func, prefix) in ROUTERS {
        let module = file.trim_end_matches(".rs");
        assert!(
            compact.contains(&format!("routes::{module}::{func}(")),
            "api/mod.rs no longer mounts routes::{module}::{func}"
        );
        assert!(
            compact.contains(&format!(".nest(\"{prefix}\"")),
            "api/mod.rs no longer nests anything under {prefix}"
        );
    }
}
