// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! System endpoints: /api/v1/system

use axum::extract::State;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;

use crate::api::SharedState;
use crate::api::error::ApiError;

#[derive(Serialize, ToSchema)]
pub struct SystemStatus {
    #[schema(value_type = String)]
    version: &'static str,
    zones: usize,
    clients: usize,
    radios: usize,
}

#[derive(Serialize, ToSchema)]
pub struct VersionInfo {
    #[schema(value_type = String)]
    version: &'static str,
    #[schema(value_type = String)]
    rust_version: &'static str,
    name: String,
}

pub fn router(state: SharedState) -> Router {
    Router::new()
        .route("/status", get(get_status))
        .route("/version", get(get_version))
        .route("/updates", get(update_status))
        .route("/updates/check", post(check_updates))
        .route("/updates/apply", post(apply_update))
        .route("/updates/config", post(configure_updates))
        .with_state(state)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UpdateConfig {
    mode: String,
    #[serde(rename = "maintenanceTime")]
    maintenance_time: String,
    timezone: String,
}

fn updater_config() -> Result<(String, String), ApiError> {
    let url = std::env::var("SNAPDOG_UPDATER_URL")
        .map_err(|_| ApiError::ServiceUnavailable("container updater"))?;
    let token = std::env::var("UPDATER_TOKEN")
        .map_err(|_| ApiError::ServiceUnavailable("container updater token"))?;
    if !url.starts_with("http://") || token.len() < 32 {
        return Err(ApiError::ServiceUnavailable("container updater"));
    }
    Ok((url.trim_end_matches('/').to_owned(), token))
}

async fn updater_request(path: &str, body: Option<Value>) -> Result<Json<Value>, ApiError> {
    let (base, token) = updater_config()?;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|error| ApiError::Internal(error.to_string()))?;
    let builder = body.map_or_else(
        || {
            if path == "/v1/status" {
                client.get(format!("{base}{path}"))
            } else {
                client.post(format!("{base}{path}"))
            }
        },
        |value| client.post(format!("{base}{path}")).json(&value),
    );
    let response = builder
        .bearer_auth(token)
        .send()
        .await
        .map_err(|error| ApiError::BadGateway(error.to_string()))?;
    if !response.status().is_success() {
        return Err(ApiError::BadGateway(format!(
            "container updater returned {}",
            response.status()
        )));
    }
    let value = response
        .json::<Value>()
        .await
        .map_err(|error| ApiError::BadGateway(error.to_string()))?;
    Ok(Json(value))
}

async fn update_status() -> Result<Json<Value>, ApiError> {
    updater_request("/v1/status", None).await
}

async fn check_updates() -> Result<Json<Value>, ApiError> {
    updater_request("/v1/check", None).await
}

async fn apply_update() -> Result<Json<Value>, ApiError> {
    updater_request("/v1/apply", None).await
}

async fn configure_updates(Json(config): Json<UpdateConfig>) -> Result<Json<Value>, ApiError> {
    updater_request(
        "/v1/config",
        Some(serde_json::json!({
            "mode": config.mode,
            "maintenanceTime": config.maintenance_time,
            "timezone": config.timezone,
        })),
    )
    .await
}

/// Retrieve basic system status statistics.
#[utoipa::path(
    get,
    path = "/api/v1/system/status",
    responses(
        (status = 200, description = "System summary statistics", body = SystemStatus)
    )
)]
async fn get_status(State(state): State<SharedState>) -> Json<SystemStatus> {
    Json(SystemStatus {
        version: env!("CARGO_PKG_VERSION"),
        zones: state.config.zones.len(),
        clients: state.config.clients.len(),
        radios: state.config.radios.len(),
    })
}

/// Retrieve application and platform version details.
#[utoipa::path(
    get,
    path = "/api/v1/system/version",
    responses(
        (status = 200, description = "Version information details", body = VersionInfo)
    )
)]
async fn get_version(State(state): State<SharedState>) -> Json<VersionInfo> {
    Json(VersionInfo {
        version: env!("CARGO_PKG_VERSION"),
        rust_version: env!("CARGO_PKG_RUST_VERSION"),
        name: state.config.name.clone(),
    })
}
