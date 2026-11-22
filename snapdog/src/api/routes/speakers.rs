// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! Speaker correction endpoints: /api/v1/speakers and /api/v1/clients/{id}/speaker

use axum::extract::{Path, State};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};

use crate::api::SharedState;
use crate::api::error::{ApiError, ErrorBody};
use crate::audio::eq::EqConfig;

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

/// List available speaker profiles
///
/// Returns a list of all speaker profiles available in the spinorama database.
#[utoipa::path(
    get,
    path = "/api/v1/speakers",
    responses(
        (status = 200, description = "List of speaker profiles", body = Vec<String>),
        (status = 503, description = "Speaker database unavailable", body = ErrorBody)
    ),
    tag = "speakers"
)]
async fn list(State(state): State<SharedState>) -> Result<Json<Vec<String>>, ApiError> {
    state
        .speaker_db
        .list_speakers()
        .await
        .map(Json)
        .map_err(|e| {
            tracing::warn!(error = %format!("{e:#}"), "Failed to fetch speaker list");
            ApiError::ServiceUnavailable("speaker database")
        })
}

/// 404 for a profile the database does not have, 502 when the database could not be read
/// (network outage, timeout, an unexpected answer).
fn profile_error(name: &str, e: &anyhow::Error) -> ApiError {
    if e.downcast_ref::<crate::spinorama::ProfileNotFound>()
        .is_some()
    {
        ApiError::NotFound("speaker")
    } else {
        tracing::warn!(speaker = %name, error = %format!("{e:#}"), "Speaker database request failed");
        ApiError::BadGateway("the speaker database could not be read".into())
    }
}

/// Get speaker correction profile
///
/// Returns the parametric Equalizer configuration (PEQ bands) for the specified speaker profile name.
#[utoipa::path(
    get,
    path = "/api/v1/speakers/{name}/profile",
    params(
        ("name" = String, Path, description = "Name of the speaker profile")
    ),
    responses(
        (status = 200, description = "Speaker PEQ profile bands", body = EqConfig),
        (status = 404, description = "Speaker profile not found", body = ErrorBody),
        (status = 502, description = "The speaker database could not be read", body = ErrorBody)
    ),
    tag = "speakers"
)]
async fn get_profile(
    State(state): State<SharedState>,
    Path(name): Path<String>,
) -> Result<Json<EqConfig>, ApiError> {
    state
        .speaker_db
        .get_profile(&name)
        .await
        .map(Json)
        .map_err(|e| profile_error(&name, &e))
}

/// Apply speaker correction profile
///
/// Configures speaker correction for a client using either a named spinorama profile or custom EQ bands.
#[utoipa::path(
    put,
    path = "/api/v1/clients/{client_index}/speaker",
    params(
        ("client_index" = usize, Path, description = "1-based index of the target client")
    ),
    request_body = ApplySpeakerRequest,
    responses(
        (status = 200, description = "Applied speaker correction configuration", body = EqConfig),
        (status = 400, description = "Invalid request payload (e.g., mutually exclusive options, validation failed)", body = ErrorBody),
        (status = 404, description = "Client or speaker profile not found", body = ErrorBody),
        (status = 422, description = "Client does not support speaker correction (not a SnapDog client)", body = ErrorBody),
        (status = 500, description = "The command could not be handed to the server task", body = ErrorBody),
        (status = 502, description = "The speaker database could not be read", body = ErrorBody)
    ),
    tag = "speakers"
)]
async fn apply_speaker(
    State(state): State<SharedState>,
    Path(idx): Path<usize>,
    Json(body): Json<ApplySpeakerRequest>,
) -> impl IntoResponse {
    require_snapdog(&state, idx).await?;
    if body.speaker.is_some() && body.custom.is_some() {
        return Err(ApiError::BadRequest(
            "speaker and custom are mutually exclusive".into(),
        ));
    }

    let config = if let Some(ref name) = body.speaker {
        state
            .speaker_db
            .get_profile(name)
            .await
            .map_err(|e| profile_error(name, &e))?
    } else {
        body.custom.unwrap_or_default()
    };
    // Profiles from the speaker database are taken as they come, a custom correction is checked.
    if body.speaker.is_none() {
        config.validate().map_err(ApiError::BadRequest)?;
    } else {
        config.validate_band_count().map_err(ApiError::BadRequest)?;
    }

    state
        .eq_store
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .set_speaker_correction(idx, config.clone());

    send_speaker_eq(&state, idx, &config).await?;
    Ok::<_, ApiError>(Json(config))
}

/// Get client speaker correction
///
/// Returns the current active speaker correction configuration (PEQ bands) applied to the specified client.
#[utoipa::path(
    get,
    path = "/api/v1/clients/{client_index}/speaker",
    params(
        ("client_index" = usize, Path, description = "1-based index of the target client")
    ),
    responses(
        (status = 200, description = "Active speaker correction configuration", body = EqConfig),
        (status = 404, description = "Client not found", body = ErrorBody),
        (status = 422, description = "Client does not support speaker correction (not a SnapDog client)", body = ErrorBody)
    ),
    tag = "speakers"
)]
async fn get_speaker(
    State(state): State<SharedState>,
    Path(idx): Path<usize>,
) -> impl IntoResponse {
    require_snapdog(&state, idx).await?;
    let config = state
        .eq_store
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get_speaker_correction(idx);
    Ok::<_, ApiError>(Json(config))
}

/// Request body for applying a speaker correction.
#[derive(serde::Deserialize, utoipa::ToSchema)]
pub struct ApplySpeakerRequest {
    /// Speaker name (from spinorama). Mutually exclusive with `custom`.
    pub speaker: Option<String>,
    /// Custom EQ config. Mutually exclusive with `speaker`.
    pub custom: Option<EqConfig>,
}

async fn require_snapdog(state: &SharedState, idx: usize) -> Result<(), ApiError> {
    super::clients::require_snapdog(state, idx, "speaker correction").await
}

/// Announce the stored correction and send it to the client if it is connected.
async fn send_speaker_eq(
    state: &SharedState,
    idx: usize,
    config: &EqConfig,
) -> Result<(), ApiError> {
    crate::api::ws::broadcast_notification(
        &state.notifications,
        &crate::api::ws::Notification::ClientSpeakerChanged {
            client: idx,
            config: config.clone(),
        },
    );
    super::clients::send_setting_if_connected(
        state,
        idx,
        snapdog_common::MSG_TYPE_SPEAKER_EQ,
        config,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::StatusCode;

    /// A server that accepts connections and never answers.
    #[allow(
        clippy::disallowed_methods,
        reason = "test-only loopback server, not a service listener"
    )]
    async fn silent_server() -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            // Keep every connection open without answering.
            let mut held = Vec::new();
            while let Ok((sock, _)) = listener.accept().await {
                held.push(sock);
            }
            drop(held);
        });
        format!("http://{addr}")
    }

    #[tokio::test]
    async fn a_stalled_speaker_database_is_a_bad_gateway_after_the_timeout() {
        let base = silent_server().await;
        let db = crate::spinorama::SpeakerDb::with_endpoints(
            &base,
            &format!("{base}/index"),
            std::time::Duration::from_millis(200),
        );
        let started = std::time::Instant::now();
        let error = db.get_profile("Some Speaker").await.unwrap_err();
        assert!(started.elapsed() < std::time::Duration::from_secs(5));
        let response = profile_error("Some Speaker", &error).into_response();
        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    }

    #[tokio::test]
    async fn a_profile_the_database_does_not_have_is_not_found() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .respond_with(wiremock::ResponseTemplate::new(404))
            .mount(&server)
            .await;
        let db = crate::spinorama::SpeakerDb::with_endpoints(
            &server.uri(),
            &format!("{}/index", server.uri()),
            std::time::Duration::from_secs(5),
        );
        let error = db.get_profile("Nope").await.unwrap_err();
        let response = profile_error("Nope", &error).into_response();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
}
