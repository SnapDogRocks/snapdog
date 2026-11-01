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

/// The programming-mode calls wait two seconds for the KNX device task and answer 503 when it does
/// not respond; the contract has to say so.
#[test]
fn the_programming_mode_calls_document_the_503() {
    let doc: serde_json::Value =
        serde_json::to_value(ApiDoc::openapi()).expect("openapi serializes");
    for verb in ["get", "put"] {
        let responses = &doc["paths"]["/api/v1/knx/programming-mode"][verb]["responses"];
        assert!(
            responses.get("503").is_some(),
            "{verb} /api/v1/knx/programming-mode has no 503: {responses}"
        );
    }
}

/// The API key is declared: a bearer scheme, optional because a server without `api_keys` is
/// open, and a 401 on every operation, which the middleware answers with plain text.
#[test]
fn the_contract_declares_the_api_key_and_the_401() {
    let doc: serde_json::Value =
        serde_json::to_value(ApiDoc::openapi()).expect("openapi serializes");
    assert_eq!(
        doc["components"]["securitySchemes"]["bearer"]["scheme"],
        "bearer"
    );
    let security = doc["security"].as_array().expect("global security");
    assert!(
        security.iter().any(|r| r == &serde_json::json!({})),
        "the key is optional"
    );
    assert!(security.iter().any(|r| r.get("bearer").is_some()));

    for (path, item) in doc["paths"].as_object().expect("paths") {
        for (verb, op) in item.as_object().expect("path item") {
            let response = &op["responses"]["401"];
            assert!(response.is_object(), "{verb} {path} has no 401");
            assert!(
                response["content"].get("text/plain").is_some(),
                "{verb} {path}: the 401 body is plain text"
            );
        }
    }
}

/// Zone and client indexes start at 1, whatever the Rust type of the parameter says.
#[test]
fn index_parameters_start_at_one() {
    let doc: serde_json::Value =
        serde_json::to_value(ApiDoc::openapi()).expect("openapi serializes");
    let mut seen = 0;
    for item in doc["paths"].as_object().expect("paths").values() {
        for op in item.as_object().expect("path item").values() {
            for param in op["parameters"].as_array().into_iter().flatten() {
                if matches!(param["name"].as_str(), Some("zone_index" | "client_index")) {
                    seen += 1;
                    assert_eq!(param["schema"]["minimum"], 1, "{param}");
                }
            }
        }
    }
    assert!(seen > 50, "only {seen} index parameters found");
}

/// A client that is not a SnapDog client answers 422 on EQ and speaker correction.
#[test]
fn the_non_snapdog_client_calls_document_the_422() {
    let doc: serde_json::Value =
        serde_json::to_value(ApiDoc::openapi()).expect("openapi serializes");
    for (path, verb) in [
        ("/api/v1/clients/{client_index}/eq", "get"),
        ("/api/v1/clients/{client_index}/eq", "put"),
        ("/api/v1/clients/{client_index}/eq/{band_index}", "put"),
        ("/api/v1/clients/{client_index}/eq/preset", "post"),
        ("/api/v1/clients/{client_index}/speaker", "get"),
        ("/api/v1/clients/{client_index}/speaker", "put"),
    ] {
        let responses = &doc["paths"][path][verb]["responses"];
        assert!(
            responses.get("422").is_some(),
            "{verb} {path} has no 422: {responses}"
        );
    }
}

// ── Registered routes versus the spec ──────────────────────────────────────
//
// axum cannot list the routes of a built `Router`, so the route tables are read
// from the source: every `.route("<path>", get(..).put(..))` under `src/api` becomes
// an operation, prefixed with the path its router is nested under in `api/mod.rs`.
// The spec has to list exactly those operations (minus `UNDOCUMENTED`), so a route
// added without a `#[utoipa::path]` (or documented but no longer served) fails here.
// The scanner refuses what it cannot read: a `.route` in an unlisted file or
// function, a handler chain without a verb it knows (`any(..)`, `on(..)`), and the
// constructs that register routes without `.route` (`route_service`, `on`).

/// `(file under src/api, router function, nest prefix)`. A `.route` in a function
/// missing here fails the test, so a new router cannot slip past it.
const ROUTERS: &[(&str, &str, &str)] = &[
    ("routes/zones.rs", "router", "/api/v1/zones"),
    ("routes/eq.rs", "router", "/api/v1/zones"),
    ("routes/eq.rs", "presets_router", "/api/v1/eq"),
    ("routes/clients.rs", "router", "/api/v1/clients"),
    ("routes/client_eq.rs", "router", "/api/v1/clients"),
    (
        "routes/speakers.rs",
        "client_speaker_router",
        "/api/v1/clients",
    ),
    ("routes/speakers.rs", "speakers_router", "/api/v1/speakers"),
    ("routes/media.rs", "router", "/api/v1/media"),
    ("routes/system.rs", "router", "/api/v1/system"),
    ("routes/knx.rs", "router", "/api/v1/knx"),
    // Merged at the root, not nested.
    ("health.rs", "router", ""),
    ("ws.rs", "router", ""),
    ("console.rs", "router", ""),
    ("mod.rs", "build_router", ""),
];

/// Served on purpose, without an entry in the spec. Each has to exist, so a stale
/// exemption fails as well.
const UNDOCUMENTED: &[(&str, &str, &str)] = &[
    (
        "GET",
        "/health",
        "liveness probes, not part of the REST contract",
    ),
    ("GET", "/health/ready", "liveness probes"),
    ("GET", "/health/live", "liveness probes"),
    ("GET", "/ws", "WebSocket upgrade, described in the README"),
    (
        "GET",
        "/api/v1/openapi.json",
        "the document itself (feature api-docs)",
    ),
    (
        "GET",
        "/docs",
        "the API console page (feature api-docs, opt-in)",
    ),
    (
        "GET",
        "/docs/assets/scalar.js",
        "the console's script (feature api-docs, opt-in)",
    ),
    (
        "GET",
        "/docs/assets/NOTICE.md",
        "licence of the console's script (feature api-docs, opt-in)",
    ),
];

const ROUTE_SOURCES: &[(&str, &str)] = &[
    (
        "routes/zones.rs",
        include_str!("../src/api/routes/zones.rs"),
    ),
    ("routes/eq.rs", include_str!("../src/api/routes/eq.rs")),
    (
        "routes/clients.rs",
        include_str!("../src/api/routes/clients.rs"),
    ),
    (
        "routes/client_eq.rs",
        include_str!("../src/api/routes/client_eq.rs"),
    ),
    (
        "routes/speakers.rs",
        include_str!("../src/api/routes/speakers.rs"),
    ),
    (
        "routes/media.rs",
        include_str!("../src/api/routes/media.rs"),
    ),
    (
        "routes/system.rs",
        include_str!("../src/api/routes/system.rs"),
    ),
    ("routes/knx.rs", include_str!("../src/api/routes/knx.rs")),
    ("health.rs", include_str!("../src/api/health.rs")),
    ("ws.rs", include_str!("../src/api/ws.rs")),
    ("console.rs", include_str!("../src/api/console.rs")),
    ("mod.rs", include_str!("../src/api/mod.rs")),
];

const API_MOD: &str = include_str!("../src/api/mod.rs");

/// Constructs that register a route without `.route(` and so escape the scanner.
const INVISIBLE_ROUTING: &[&str] = &[
    "route_service(",
    ".on(",
    "nest_service(",
    "fallback_service(",
];

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

/// The source without whole-line `//` comments, so a commented-out route is not counted.
fn without_comments(source: &str) -> String {
    source
        .lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Every `(VERB, path)` registered by the `.route(..)` calls in one source file.
fn registered_in(file: &str, source: &str) -> Vec<(String, String)> {
    let source = &without_comments(source);
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
        assert!(
            rest.trim_start().starts_with('"'),
            "{file}: a `.route` whose path is not a string literal cannot be scanned: {}",
            rest.lines().next().unwrap_or_default()
        );
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
        let before = found.len();
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
        assert!(
            found.len() > before,
            "{file}: `.route(\"{path}\", {chain})` has no verb the scanner knows ({VERBS:?}); \
             extend the scanner rather than leave the route unchecked"
        );
    }
    found
}

/// Every operation the routers serve, documented or not.
fn served_operations() -> std::collections::BTreeSet<(String, String)> {
    ROUTE_SOURCES
        .iter()
        .flat_map(|(file, source)| registered_in(file, source))
        .collect()
}

/// The served operations the spec has to describe.
fn registered_operations() -> std::collections::BTreeSet<(String, String)> {
    served_operations()
        .into_iter()
        .filter(|(verb, path)| {
            !UNDOCUMENTED
                .iter()
                .any(|(v, p, _)| v == verb && normalise(p) == *path)
        })
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
    // Every file under src/api that registers a route is scanned...
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/src/api");
    let mut with_routes = Vec::new();
    let mut pending = vec![std::path::PathBuf::from(root)];
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                let text = without_comments(&std::fs::read_to_string(&path).unwrap());
                if text.contains(".route(") {
                    let relative = path.strip_prefix(root).unwrap();
                    with_routes.push(relative.to_string_lossy().replace('\\', "/"));
                }
                for construct in INVISIBLE_ROUTING {
                    assert!(
                        !text.contains(construct),
                        "{}: `{construct}` registers routes the scanner cannot see",
                        path.display()
                    );
                }
            }
        }
    }
    with_routes.sort();
    let mut scanned: Vec<String> = ROUTE_SOURCES
        .iter()
        .map(|(f, _)| (*f).to_string())
        .collect();
    scanned.sort();
    assert_eq!(
        with_routes, scanned,
        "a file with routes is missing from ROUTE_SOURCES (or the reverse)"
    );

    // ...and the prefixes are the ones api/mod.rs nests the routers under.
    let compact: String = API_MOD.chars().filter(|c| !c.is_whitespace()).collect();
    for (file, func, prefix) in ROUTERS {
        if *func == "build_router" {
            continue;
        }
        let module = file.trim_end_matches(".rs").rsplit('/').next().unwrap();
        let qualified = if file.starts_with("routes/") {
            format!("routes::{module}::{func}(")
        } else {
            format!("{module}::{func}(")
        };
        assert!(
            compact.contains(&qualified),
            "api/mod.rs no longer mounts {qualified}"
        );
        if !prefix.is_empty() {
            assert!(
                compact.contains(&format!(".nest(\"{prefix}\"")),
                "api/mod.rs no longer nests anything under {prefix}"
            );
        }
    }
}

#[test]
fn every_undocumented_route_is_still_served() {
    let served = served_operations();
    for (verb, path, why) in UNDOCUMENTED {
        assert!(
            served.contains(&((*verb).to_string(), normalise(path))),
            "{verb} {path} ({why}) is no longer served: remove it from UNDOCUMENTED"
        );
    }
}

#[test]
fn the_scanner_refuses_a_route_it_cannot_classify() {
    // The mutation that used to pass: an undocumented route in api/mod.rs.
    let extra = "pub fn build_router() { Router::new().route(\"/api/v1/secret\", get(h)) }";
    let found = registered_in("mod.rs", extra);
    assert_eq!(found, [("GET".to_string(), "/api/v1/secret".to_string())]);
    assert!(
        !UNDOCUMENTED
            .iter()
            .any(|(v, p, _)| *v == "GET" && *p == "/api/v1/secret"),
        "the extra route must show up as undocumented"
    );

    let caught = std::panic::catch_unwind(|| {
        registered_in(
            "mod.rs",
            "pub fn build_router() { r.route(\"/x\", any(h)) }",
        )
    });
    assert!(caught.is_err(), "a chain without a known verb is refused");
    let caught = std::panic::catch_unwind(|| {
        registered_in("mod.rs", "pub fn build_router() { r.route(PATH, get(h)) }")
    });
    assert!(caught.is_err(), "a path that is not a literal is refused");
    let caught = std::panic::catch_unwind(|| {
        registered_in(
            "routes/zones.rs",
            "pub fn mystery() { r.route(\"/x\", get(h)) }",
        )
    });
    assert!(caught.is_err(), "an unlisted router function is refused");

    // A commented-out route is not a route.
    let commented = "pub fn build_router() {\n    // .route(\"/old\", get(h))\n}";
    assert_eq!(
        registered_in("mod.rs", commented),
        Vec::<(String, String)>::new()
    );
}
