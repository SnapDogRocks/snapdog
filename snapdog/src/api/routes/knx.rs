// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! KNX device management routes.

use axum::extract::State;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};

use crate::api::SharedState;
use crate::api::error::{ApiError, ErrorBody};
use crate::knx::KnxDeviceControl as _;

pub fn router(state: SharedState) -> Router {
    Router::new()
        .route(
            "/programming-mode",
            get(get_programming_mode).put(set_programming_mode),
        )
        .with_state(state)
}

/// Get KNX programming mode status
///
/// Returns whether the KNX device programming mode is currently active.
/// This endpoint is only available when the server is running in KNX device mode.
#[utoipa::path(
    get,
    path = "/api/v1/knx/programming-mode",
    responses(
        (status = 200, description = "Programming mode status", body = bool),
        (status = 409, description = "KNX device mode not active", body = ErrorBody)
    ),
    tag = "knx"
)]
async fn get_programming_mode(State(state): State<SharedState>) -> impl IntoResponse {
    let Some(ref ctl) = state.knx_device_control else {
        return Err(ApiError::Conflict("KNX device mode not active"));
    };
    Ok(Json(ctl.get_prog_mode().await))
}

/// Set KNX programming mode status
///
/// Enable or disable the KNX device programming mode to allow ETS physical addressing.
/// This endpoint is only available when the server is running in KNX device mode.
#[utoipa::path(
    put,
    path = "/api/v1/knx/programming-mode",
    request_body = bool,
    responses(
        (status = 200, description = "Updated programming mode status", body = bool),
        (status = 409, description = "KNX device mode not active", body = ErrorBody)
    ),
    tag = "knx"
)]
async fn set_programming_mode(
    State(state): State<SharedState>,
    Json(enabled): Json<bool>,
) -> impl IntoResponse {
    let Some(ref ctl) = state.knx_device_control else {
        return Err(ApiError::Conflict("KNX device mode not active"));
    };
    ctl.set_prog_mode(enabled).await;
    Ok(Json(enabled))
}
