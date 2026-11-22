// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! Client EQ endpoints: /`api/v1/clients/{client_index}/eq`

use axum::extract::{Path, State};
use axum::response::IntoResponse;
use axum::routing::{get, post, put};
use axum::{Json, Router};

use crate::api::SharedState;
use crate::api::error::{ApiError, ErrorBody};
use crate::audio::eq::{EqBand, EqConfig, TYPE_EQ_CONFIG};

use super::eq::resolve_preset;

pub fn router(state: SharedState) -> Router {
    Router::new()
        .route("/{client_index}/eq", get(get_eq).put(set_eq))
        .route("/{client_index}/eq/{band_index}", put(set_band))
        .route("/{client_index}/eq/preset", post(apply_preset))
        .with_state(state)
}

async fn require_snapdog(state: &SharedState, idx: usize) -> Result<(), ApiError> {
    super::clients::require_snapdog(state, idx, "EQ").await
}

/// Announce the stored EQ and send it to the client if it is connected.
async fn send_eq(state: &SharedState, idx: usize, config: &EqConfig) -> Result<(), ApiError> {
    crate::api::ws::broadcast_notification(
        &state.notifications,
        &crate::api::ws::Notification::ClientEqChanged {
            client: idx,
            config: config.clone(),
        },
    );
    super::clients::send_setting_if_connected(state, idx, TYPE_EQ_CONFIG, config).await
}

/// Get client Equalizer configuration
///
/// Returns the current active 10-band parametric Equalizer configuration for the specified client.
#[utoipa::path(
    get,
    path = "/api/v1/clients/{client_index}/eq",
    params(
        ("client_index" = usize, Path, description = "1-based index of the target client")
    ),
    responses(
        (status = 200, description = "Current equalizer configuration", body = EqConfig),
        (status = 404, description = "Client not found", body = ErrorBody),
        (status = 422, description = "Client does not support EQ (not a SnapDog client)", body = ErrorBody)
    ),
    tag = "equalizer"
)]
async fn get_eq(State(state): State<SharedState>, Path(idx): Path<usize>) -> impl IntoResponse {
    require_snapdog(&state, idx).await?;
    let config = state
        .eq_store
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get_client(idx);
    Ok::<_, ApiError>(Json(config))
}

/// Set client Equalizer configuration
///
/// Updates the full 10-band parametric Equalizer configuration for the specified client.
#[utoipa::path(
    put,
    path = "/api/v1/clients/{client_index}/eq",
    params(
        ("client_index" = usize, Path, description = "1-based index of the target client")
    ),
    request_body = EqConfig,
    responses(
        (status = 200, description = "Updated equalizer configuration", body = EqConfig),
        (status = 400, description = "Too many bands or validation error", body = ErrorBody),
        (status = 404, description = "Client not found", body = ErrorBody),
        (status = 422, description = "Client does not support EQ (not a SnapDog client)", body = ErrorBody),
        (status = 500, description = "The command could not be handed to the server task", body = ErrorBody)
    ),
    tag = "equalizer"
)]
async fn set_eq(
    State(state): State<SharedState>,
    Path(idx): Path<usize>,
    Json(config): Json<EqConfig>,
) -> impl IntoResponse {
    require_snapdog(&state, idx).await?;
    config.validate().map_err(ApiError::BadRequest)?;
    state
        .eq_store
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .set_client(idx, config.clone());
    send_eq(&state, idx, &config).await?;
    Ok::<_, ApiError>(Json(config))
}

/// Update specific client Equalizer band
///
/// Modifies a single filter band at the specified index within a client's Equalizer configuration.
#[utoipa::path(
    put,
    path = "/api/v1/clients/{client_index}/eq/{band_index}",
    params(
        ("client_index" = usize, Path, description = "1-based index of the target client"),
        ("band_index" = usize, Path, description = "0-based index of the filter band to edit")
    ),
    request_body = EqBand,
    responses(
        (status = 200, description = "Updated full equalizer configuration", body = EqConfig),
        (status = 400, description = "Validation error", body = ErrorBody),
        (status = 404, description = "Client or band not found", body = ErrorBody),
        (status = 422, description = "Client does not support EQ (not a SnapDog client)", body = ErrorBody),
        (status = 500, description = "The command could not be handed to the server task", body = ErrorBody)
    ),
    tag = "equalizer"
)]
async fn set_band(
    State(state): State<SharedState>,
    Path((idx, band_idx)): Path<(usize, usize)>,
    Json(band): Json<EqBand>,
) -> impl IntoResponse {
    band.validate().map_err(ApiError::BadRequest)?;
    require_snapdog(&state, idx).await?;
    // Read, change and write under one lock, so that two quick band changes both survive.
    let config = {
        let mut store = state
            .eq_store
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut config = store.get_client(idx);
        if band_idx >= config.bands.len() {
            return Err(ApiError::NotFound("band"));
        }
        config.bands[band_idx] = band;
        config.preset = None;
        store.set_client(idx, config.clone());
        config
    };
    send_eq(&state, idx, &config).await?;
    Ok::<_, ApiError>(Json(config))
}

/// Apply client Equalizer preset
///
/// Overwrites the client's Equalizer settings with a predefined preset curve.
/// The body is a preset name as listed by `GET /api/v1/eq/presets`.
#[utoipa::path(
    post,
    path = "/api/v1/clients/{client_index}/eq/preset",
    params(
        ("client_index" = usize, Path, description = "1-based index of the target client")
    ),
    request_body = String,
    responses(
        (status = 200, description = "Updated full equalizer configuration from preset", body = EqConfig),
        (status = 400, description = "Unknown preset name", body = ErrorBody),
        (status = 404, description = "Client not found", body = ErrorBody),
        (status = 422, description = "Client does not support EQ (not a SnapDog client)", body = ErrorBody),
        (status = 500, description = "The command could not be handed to the server task", body = ErrorBody)
    ),
    tag = "equalizer"
)]
async fn apply_preset(
    State(state): State<SharedState>,
    Path(idx): Path<usize>,
    Json(name): Json<String>,
) -> impl IntoResponse {
    require_snapdog(&state, idx).await?;
    let config = resolve_preset(&name)?;
    state
        .eq_store
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .set_client(idx, config.clone());
    send_eq(&state, idx, &config).await?;
    Ok::<_, ApiError>(Json(config))
}
