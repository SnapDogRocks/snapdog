// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! Client EQ endpoints: /`api/v1/clients/{client_index}/eq`

use axum::extract::{Path, State};
use axum::response::IntoResponse;
use axum::routing::{get, post, put};
use axum::{Json, Router};

use crate::api::SharedState;
use crate::api::error::ApiError;
use crate::audio::eq::{EqBand, EqConfig, TYPE_EQ_CONFIG};
use crate::player::{ClientAction, SnapcastCmd};

use super::eq::resolve_preset;

pub fn router(state: SharedState) -> Router {
    Router::new()
        .route("/{client_index}/eq", get(get_eq).put(set_eq))
        .route("/{client_index}/eq/{band_index}", put(set_band))
        .route("/{client_index}/eq/preset", post(apply_preset))
        .with_state(state)
}

/// Returns 400 if the client is not a `SnapDog` client.
async fn require_snapdog(state: &SharedState, idx: usize) -> Result<(), ApiError> {
    if idx == 0 || idx > state.config.clients.len() {
        return Err(ApiError::NotFound("client"));
    }
    let store = state.store.read().await;
    match store.clients.get(&idx) {
        Some(c) if c.is_snapdog => Ok(()),
        Some(_) => Err(ApiError::Unprocessable(
            "Client does not support EQ (not a SnapDog client)".into(),
        )),
        None => Err(ApiError::NotFound("client")),
    }
}

async fn snap_id(state: &SharedState, idx: usize) -> Result<String, ApiError> {
    state
        .store
        .read()
        .await
        .clients
        .get(&idx)
        .and_then(|c| c.snapcast_id.clone())
        .ok_or(ApiError::NotFound("client"))
}

async fn send_eq(state: &SharedState, idx: usize, config: &EqConfig) -> Result<(), ApiError> {
    let client_id = snap_id(state, idx).await?;
    let payload = serde_json::to_vec(config).map_err(|e| ApiError::Internal(e.to_string()))?;
    let _ = state
        .snap_tx
        .send(SnapcastCmd::Client {
            client_id,
            action: ClientAction::SendCustom {
                type_id: TYPE_EQ_CONFIG,
                payload,
            },
        })
        .await;
    Ok(())
}

async fn get_eq(State(state): State<SharedState>, Path(idx): Path<usize>) -> impl IntoResponse {
    require_snapdog(&state, idx).await?;
    let config = state
        .eq_store
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get_client(idx);
    Ok::<_, ApiError>(Json(config))
}

async fn set_eq(
    State(state): State<SharedState>,
    Path(idx): Path<usize>,
    Json(config): Json<EqConfig>,
) -> impl IntoResponse {
    require_snapdog(&state, idx).await?;
    if config.bands.len() > snapdog_common::MAX_EQ_BANDS {
        return Err(ApiError::BadRequest("Maximum 10 EQ bands".into()));
    }
    state
        .eq_store
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .set_client(idx, config.clone());
    send_eq(&state, idx, &config).await?;
    Ok::<_, ApiError>(Json(config))
}

async fn set_band(
    State(state): State<SharedState>,
    Path((idx, band_idx)): Path<(usize, usize)>,
    Json(band): Json<EqBand>,
) -> impl IntoResponse {
    require_snapdog(&state, idx).await?;
    let mut config = state
        .eq_store
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get_client(idx);
    if band_idx >= config.bands.len() {
        return Err(ApiError::NotFound("band"));
    }
    config.bands[band_idx] = band;
    config.preset = None;
    state
        .eq_store
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .set_client(idx, config.clone());
    send_eq(&state, idx, &config).await?;
    Ok::<_, ApiError>(Json(config))
}

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
