// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! Speaker correction endpoints: /api/v1/speakers and /api/v1/clients/{id}/speaker

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};

use crate::api::SharedState;
use crate::api::error::ApiError;
use crate::audio::eq::EqConfig;
use crate::player::{ClientAction, SnapcastCmd};

/// Router for `/api/v1/speakers`.
pub fn speakers_router(state: SharedState) -> Router {
    Router::new()
        .route("/", get(list))
        .route("/{name}/profile", get(get_profile))
        .with_state(state)
}

/// Router for `/api/v1/clients/{id}/speaker`.
pub fn client_speaker_router(state: SharedState) -> Router {
    Router::new()
        .route(
            "/{client_index}/speaker",
            get(get_speaker).put(apply_speaker),
        )
        .with_state(state)
}

/// GET /api/v1/speakers — list available speaker profiles.
async fn list(State(state): State<SharedState>) -> Result<Json<Vec<String>>, StatusCode> {
    state
        .speaker_db
        .list_speakers()
        .await
        .map(Json)
        .map_err(|e| {
            tracing::warn!(error = %e, "Failed to fetch speaker list");
            StatusCode::SERVICE_UNAVAILABLE
        })
}

/// GET /api/v1/speakers/:name/profile — get PEQ filters for a speaker.
async fn get_profile(
    State(state): State<SharedState>,
    Path(name): Path<String>,
) -> Result<Json<EqConfig>, StatusCode> {
    state
        .speaker_db
        .get_profile(&name)
        .await
        .map(Json)
        .map_err(|e| {
            tracing::warn!(speaker = %name, error = %e, "Failed to fetch speaker profile");
            StatusCode::NOT_FOUND
        })
}

/// PUT /api/v1/clients/:id/speaker — apply a speaker correction profile.
async fn apply_speaker(
    State(state): State<SharedState>,
    Path(idx): Path<usize>,
    Json(body): Json<ApplySpeakerRequest>,
) -> impl IntoResponse {
    require_snapdog(&state, idx).await?;

    let config = if let Some(ref name) = body.speaker {
        state.speaker_db.get_profile(name).await.map_err(|e| {
            tracing::warn!(speaker = %name, error = %e, "Speaker profile not found");
            ApiError::NotFound("speaker")
        })?
    } else if let Some(custom) = body.custom {
        custom
    } else {
        EqConfig::default()
    };

    state
        .eq_store
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .set_speaker_correction(idx, config.clone());

    send_speaker_eq(&state, idx, &config).await?;
    Ok::<_, ApiError>(Json(config))
}

/// GET /api/v1/clients/:id/speaker — get current speaker correction for a client.
async fn get_speaker(
    State(state): State<SharedState>,
    Path(idx): Path<usize>,
) -> impl IntoResponse {
    require_snapdog(&state, idx).await?;
    let config = state
        .eq_store
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get_speaker_correction(idx);
    Ok::<_, ApiError>(Json(config))
}

/// Request body for applying a speaker correction.
#[derive(serde::Deserialize)]
pub struct ApplySpeakerRequest {
    /// Speaker name (from spinorama). Mutually exclusive with `custom`.
    pub speaker: Option<String>,
    /// Custom EQ config. Mutually exclusive with `speaker`.
    pub custom: Option<EqConfig>,
}

async fn require_snapdog(state: &SharedState, idx: usize) -> Result<(), ApiError> {
    if idx == 0 || idx > state.config.clients.len() {
        return Err(ApiError::NotFound("client"));
    }
    let store = state.store.read().await;
    match store.clients.get(&idx) {
        Some(c) if c.is_snapdog => Ok(()),
        Some(_) => Err(ApiError::Unprocessable(
            "Client does not support speaker correction (not a SnapDog client)".into(),
        )),
        None => Err(ApiError::NotFound("client")),
    }
}

async fn send_speaker_eq(
    state: &SharedState,
    idx: usize,
    config: &EqConfig,
) -> Result<(), ApiError> {
    let client_id = state
        .store
        .read()
        .await
        .clients
        .get(&idx)
        .and_then(|c| c.snapcast_id.clone())
        .ok_or(ApiError::NotFound("client"))?;
    let payload = serde_json::to_vec(config).map_err(|e| ApiError::Internal(e.to_string()))?;
    let _ = state
        .snap_tx
        .send(SnapcastCmd::Client {
            client_id,
            action: ClientAction::SendCustom {
                type_id: snapdog_common::MSG_TYPE_SPEAKER_EQ,
                payload,
            },
        })
        .await;
    Ok(())
}
