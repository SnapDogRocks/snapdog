// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! System endpoints: /api/v1/system

use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};
use serde::Serialize;

use crate::api::SharedState;

#[derive(Serialize)]
struct SystemStatus {
    version: &'static str,
    zones: usize,
    clients: usize,
    radios: usize,
}

#[derive(Serialize)]
struct VersionInfo {
    version: &'static str,
    rust_version: &'static str,
    name: String,
}

pub fn router(state: SharedState) -> Router {
    Router::new()
        .route("/status", get(get_status))
        .route("/version", get(get_version))
        .with_state(state)
}

async fn get_status(State(state): State<SharedState>) -> Json<SystemStatus> {
    Json(SystemStatus {
        version: env!("CARGO_PKG_VERSION"),
        zones: state.config.zones.len(),
        clients: state.config.clients.len(),
        radios: state.config.radios.len(),
    })
}

async fn get_version(State(state): State<SharedState>) -> Json<VersionInfo> {
    Json(VersionInfo {
        version: env!("CARGO_PKG_VERSION"),
        rust_version: env!("CARGO_PKG_RUST_VERSION"),
        name: state.config.name.clone(),
    })
}
