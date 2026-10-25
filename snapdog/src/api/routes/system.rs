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
use crate::api::error::{ApiError, ErrorBody};

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

/// New update schedule for the container updater sidecar.
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateConfig {
    /// `manual` only checks and applies when asked, `automatic` applies new releases
    /// at `maintenanceTime`.
    #[schema(example = "automatic")]
    mode: String,
    /// Local time of day for automatic updates, `HH:MM`.
    #[serde(rename = "maintenanceTime")]
    #[schema(example = "02:00")]
    maintenance_time: String,
    /// IANA time zone the maintenance time is read in.
    #[schema(example = "Europe/Berlin")]
    timezone: String,
}

/// Progress of a running update.
#[derive(Serialize, ToSchema)]
pub struct UpdateProgress {
    /// Current phase of the update.
    phase: String,
    /// Further detail about the phase, if any.
    detail: Option<String>,
}

/// State of the container updater sidecar, as reported by the sidecar itself.
/// The sidecar may add further fields; the ones listed here are what the `WebUI` reads.
#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct UpdateStatus {
    /// Updater state, for example `idle`, `checking`, `updating`.
    state: String,
    /// Version of the running image, if known.
    current_version: Option<String>,
    /// Newest release found by the last check, if any.
    available_version: Option<String>,
    /// Whether `availableVersion` is newer than `currentVersion`.
    update_available: bool,
    /// Error of the last check or update, if it failed.
    last_error: Option<String>,
    /// `manual` or `automatic`.
    update_mode: String,
    /// Local time of day for automatic updates, `HH:MM`.
    maintenance_time: String,
    /// IANA time zone for `maintenanceTime`.
    timezone: String,
    /// Progress of a running update, `null` when none runs.
    progress: Option<UpdateProgress>,
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

/// Read the state of the container updater sidecar.
///
/// Needs `SNAPDOG_UPDATER_URL` and `UPDATER_TOKEN` (see `deploy/docker-compose.yml`);
/// without them, and outside the Docker deployment, the answer is 503.
#[utoipa::path(
    get,
    path = "/api/v1/system/updates",
    tag = "system",
    responses(
        (status = 200, description = "Updater state", body = UpdateStatus),
        (status = 502, description = "The updater sidecar did not answer or refused", body = ErrorBody),
        (status = 503, description = "No updater sidecar is configured", body = ErrorBody)
    )
)]
async fn update_status() -> Result<Json<Value>, ApiError> {
    updater_request("/v1/status", None).await
}

/// Ask the sidecar to look for a newer release now.
///
/// Answers with the updater state once the check has been started. A check or update
/// that is already running makes the sidecar refuse, which shows up as 502.
#[utoipa::path(
    post,
    path = "/api/v1/system/updates/check",
    tag = "system",
    responses(
        (status = 200, description = "Updater state after starting the check", body = UpdateStatus),
        (status = 502, description = "The updater sidecar did not answer or refused (for example an operation is running)", body = ErrorBody),
        (status = 503, description = "No updater sidecar is configured", body = ErrorBody)
    )
)]
async fn check_updates() -> Result<Json<Value>, ApiError> {
    updater_request("/v1/check", None).await
}

/// Apply the available update now.
///
/// The sidecar pulls the new image, verifies it and restarts the server, so the
/// connection drops while this runs. Does nothing if no update is available.
#[utoipa::path(
    post,
    path = "/api/v1/system/updates/apply",
    tag = "system",
    responses(
        (status = 200, description = "Updater state after starting the update", body = UpdateStatus),
        (status = 502, description = "The updater sidecar did not answer or refused (for example an operation is running)", body = ErrorBody),
        (status = 503, description = "No updater sidecar is configured", body = ErrorBody)
    )
)]
async fn apply_update() -> Result<Json<Value>, ApiError> {
    updater_request("/v1/apply", None).await
}

/// Change the update mode and maintenance window of the sidecar.
#[utoipa::path(
    post,
    path = "/api/v1/system/updates/config",
    tag = "system",
    request_body = UpdateConfig,
    responses(
        (status = 200, description = "Updater state with the new schedule", body = UpdateStatus),
        (status = 422, description = "Malformed body or unknown field (plain-text rejection)"),
        (status = 502, description = "The updater sidecar did not answer or rejected the schedule", body = ErrorBody),
        (status = 503, description = "No updater sidecar is configured", body = ErrorBody)
    )
)]
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
    tag = "system",
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
    tag = "system",
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
