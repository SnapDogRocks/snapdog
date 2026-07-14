// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! KNX device management routes.

use axum::extract::State;
use axum::http::header;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use utoipa::ToSchema;

use crate::api::SharedState;
use crate::api::error::{ApiError, ErrorBody};
use crate::knx::KnxDeviceControl as _;
use crate::knx::group_objects::{KNXPROD_APP_NUMBER, KNXPROD_APP_VERSION, KNXPROD_HW_VERSION};

/// The compiled ETS product database, embedded at build time. Released binaries embed a
/// **signed** archive when an ETS key is available to the build (see the release
/// workflow); otherwise the committed unsigned artifact is served.
const KNXPROD: &[u8] = include_bytes!("../../../../knx/snapdog.knxprod");

pub fn router(state: SharedState) -> Router {
    Router::new()
        .route(
            "/programming-mode",
            get(get_programming_mode).put(set_programming_mode),
        )
        .route("/knxprod", get(get_knxprod))
        .route("/product-info", get(get_product_info))
        .with_state(state)
}

/// ETS product identity of the embedded `.knxprod`, shown next to the `WebUI` download so
/// an integrator can confirm it matches this device's firmware.
#[derive(serde::Serialize, ToSchema)]
pub struct ProductInfo {
    /// ETS `ApplicationVersion`.
    app_version: u32,
    /// `ApplicationNumber` / order number, formatted `0xFF01`.
    application_number: String,
    /// Hardware revision.
    hardware_version: u8,
}

/// Product identity (version / order number / hardware revision) of the embedded
/// `.knxprod`.
#[utoipa::path(
    get,
    path = "/api/v1/knx/product-info",
    responses((status = 200, description = "Embedded ETS product identity", body = ProductInfo)),
    tag = "knx"
)]
pub async fn get_product_info() -> impl IntoResponse {
    Json(ProductInfo {
        app_version: KNXPROD_APP_VERSION,
        application_number: format!("0x{KNXPROD_APP_NUMBER:04X}"),
        hardware_version: KNXPROD_HW_VERSION,
    })
}

/// Download the KNX ETS product database
///
/// Returns the `.knxprod` for importing `SnapDog` into ETS. Served regardless of KNX
/// device mode so it can be fetched for commissioning at any time.
#[utoipa::path(
    get,
    path = "/api/v1/knx/knxprod",
    responses((status = 200, description = "The .knxprod product database (ZIP)")),
    tag = "knx"
)]
pub async fn get_knxprod() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "application/octet-stream"),
            (
                header::CONTENT_DISPOSITION,
                "attachment; filename=\"snapdog.knxprod\"",
            ),
        ],
        KNXPROD,
    )
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
