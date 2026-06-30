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
