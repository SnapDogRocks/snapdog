// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! Client endpoints: /api/v1/clients

use axum::extract::{Path, State};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Serialize;

use crate::api::SharedState;
use crate::api::error::{ApiError, ErrorBody};
use crate::api::routes::zones::VolumeValue;
use crate::player::{ClientAction, SnapcastCmd};
use crate::state;

#[derive(Serialize, utoipa::ToSchema)]
pub struct ClientInfo {
    index: usize,
    name: String,
    mac: String,
    zone_index: usize,
    icon: String,
    volume: i32,
    max_volume: i32,
    muted: bool,
    connected: bool,
    is_snapdog: bool,
}

pub fn router(state: SharedState) -> Router {
    Router::new()
        .route("/count", get(get_count))
        .route("/", get(get_all))
        .route("/{client_index}", get(get_client))
        .route("/{client_index}/volume", get(get_volume).put(set_volume))
        .route("/{client_index}/mute", get(get_mute).put(set_mute))
        .route("/{client_index}/mute/toggle", post(toggle_mute))
        .route("/{client_index}/latency", get(get_latency).put(set_latency))
        .route("/{client_index}/zone", get(get_zone).put(set_zone))
        .route("/{client_index}/name", get(get_name).put(set_name))
        .route("/{client_index}/icon", get(get_icon))
        .route("/{client_index}/connected", get(get_connected))
        .with_state(state)
}

async fn read_client(state: &SharedState, idx: usize) -> Option<state::ClientState> {
    state.store.read().await.clients.get(&idx).cloned()
}

const fn not_found() -> ApiError {
    ApiError::NotFound("client")
}

/// Get total number of clients
///
/// Returns the number of configured clients in the system.
#[utoipa::path(
    get,
    path = "/api/v1/clients/count",
    responses(
        (status = 200, description = "Total client count", body = usize)
    ),
    tag = "clients"
)]
async fn get_count(State(state): State<SharedState>) -> Json<usize> {
    Json(state.config.clients.len())
}

/// Get all clients details
///
/// Returns a list of all clients, including their volume, mute, connection status, and assigned zone.
#[utoipa::path(
    get,
    path = "/api/v1/clients",
    responses(
        (status = 200, description = "List of all client details", body = Vec<ClientInfo>)
    ),
    tag = "clients"
)]
async fn get_all(State(state): State<SharedState>) -> Json<Vec<ClientInfo>> {
    let store = state.store.read().await;
    Json(
        state
            .config
            .clients
            .iter()
            .map(|c| {
                let cs = store.clients.get(&c.index);
                ClientInfo {
                    index: c.index,
                    name: c.name.clone(),
                    mac: c.mac.clone(),
                    zone_index: cs.map_or(c.zone_index, |s| s.zone_index),
                    icon: c.icon.clone(),
                    volume: cs.map_or(crate::state::DEFAULT_VOLUME, |s| s.base_volume),
                    max_volume: cs.map_or(c.max_volume, |s| s.max_volume),
                    muted: cs.is_some_and(|s| s.muted),
                    connected: cs.is_some_and(|s| s.connected),
                    is_snapdog: cs.is_some_and(|s| s.is_snapdog),
                }
            })
            .collect(),
    )
}

/// Get client details by index
///
/// Returns the details of a single client specified by its 1-based index.
#[utoipa::path(
    get,
    path = "/api/v1/clients/{client_index}",
    params(
        ("client_index" = usize, Path, description = "1-based index of the target client")
    ),
    responses(
        (status = 200, description = "Client details", body = ClientInfo),
        (status = 404, description = "Client not found", body = ErrorBody)
    ),
    tag = "clients"
)]
async fn get_client(State(state): State<SharedState>, Path(idx): Path<usize>) -> impl IntoResponse {
    let cfg = idx
        .checked_sub(1)
        .and_then(|i| state.config.clients.get(i))
        .ok_or(not_found())?;
    let cs = state.store.read().await.clients.get(&idx).cloned();
    Ok::<_, ApiError>(Json(ClientInfo {
        index: cfg.index,
        name: cfg.name.clone(),
        mac: cfg.mac.clone(),
        zone_index: cs.as_ref().map_or(cfg.zone_index, |s| s.zone_index),
        icon: cfg.icon.clone(),
        volume: cs
            .as_ref()
            .map_or(crate::state::DEFAULT_VOLUME, |s| s.base_volume),
        max_volume: cs.as_ref().map_or(cfg.max_volume, |s| s.max_volume),
        muted: cs.as_ref().is_some_and(|s| s.muted),
        connected: cs.as_ref().is_some_and(|s| s.connected),
        is_snapdog: cs.as_ref().is_some_and(|s| s.is_snapdog),
    }))
}

/// Get client volume level
///
/// Returns the current volume level (0-100) of the specified client.
#[utoipa::path(
    get,
    path = "/api/v1/clients/{client_index}/volume",
    params(
        ("client_index" = usize, Path, description = "1-based index of the target client")
    ),
    responses(
        (status = 200, description = "Current client volume", body = i32),
        (status = 404, description = "Client not found", body = ErrorBody)
    ),
    tag = "clients"
)]
async fn get_volume(State(state): State<SharedState>, Path(idx): Path<usize>) -> impl IntoResponse {
    read_client(&state, idx)
        .await
        .map(|c| Json(c.base_volume))
        .ok_or(not_found())
}

/// Set client volume level
///
/// Updates the volume level of the specified client. Supports absolute or relative values (e.g. "+5", "-5", or "45").
#[utoipa::path(
    put,
    path = "/api/v1/clients/{client_index}/volume",
    params(
        ("client_index" = usize, Path, description = "1-based index of the target client")
    ),
    request_body = VolumeValue,
    responses(
        (status = 200, description = "Updated client volume", body = i32),
        (status = 400, description = "Invalid volume value", body = ErrorBody),
        (status = 404, description = "Client not found", body = ErrorBody)
    ),
    tag = "clients"
)]
async fn set_volume(
    State(state): State<SharedState>,
    Path(idx): Path<usize>,
    Json(value): Json<VolumeValue>,
) -> impl IntoResponse {
    let store = state.store.read().await;
    let client = store.clients.get(&idx).ok_or(not_found())?;
    let volume = value
        .resolve(client.base_volume)
        .map_err(|e| ApiError::BadRequest(e.to_string()))?;
    let snap_id = client.snapcast_id.clone().ok_or(not_found())?;
    drop(store);

    let _ = state
        .snap_tx
        .send(SnapcastCmd::Client {
            client_id: snap_id,
            action: ClientAction::Volume(volume),
        })
        .await;
    // State update comes from Snapcast Client.OnVolumeChanged notification
    tracing::debug!(client = idx, volume, "Volume set");
    Ok::<_, ApiError>(Json(volume))
}

/// Get client mute status
///
/// Returns whether the specified client is currently muted.
#[utoipa::path(
    get,
    path = "/api/v1/clients/{client_index}/mute",
    params(
        ("client_index" = usize, Path, description = "1-based index of the target client")
    ),
    responses(
        (status = 200, description = "Client mute status", body = bool),
        (status = 404, description = "Client not found", body = ErrorBody)
    ),
    tag = "clients"
)]
async fn get_mute(State(state): State<SharedState>, Path(idx): Path<usize>) -> impl IntoResponse {
    read_client(&state, idx)
        .await
        .map(|c| Json(c.muted))
        .ok_or(not_found())
}

/// Set client mute status
///
/// Mutes or unmutes the specified client.
#[utoipa::path(
    put,
    path = "/api/v1/clients/{client_index}/mute",
    params(
        ("client_index" = usize, Path, description = "1-based index of the target client")
    ),
    request_body = bool,
    responses(
        (status = 200, description = "Updated client mute status", body = bool),
        (status = 404, description = "Client not found", body = ErrorBody)
    ),
    tag = "clients"
)]
async fn set_mute(
    State(state): State<SharedState>,
    Path(idx): Path<usize>,
    Json(v): Json<bool>,
) -> impl IntoResponse {
    let snap_id = read_client(&state, idx)
        .await
        .and_then(|c| c.snapcast_id)
        .ok_or(not_found())?;
    let _ = state
        .snap_tx
        .send(SnapcastCmd::Client {
            client_id: snap_id,
            action: ClientAction::Mute(v),
        })
        .await;
    tracing::debug!(client = idx, muted = v, "Mute set");
    Ok::<_, ApiError>(Json(v))
}

/// Toggle client mute status
///
/// Toggles the mute status of the specified client and returns the new status.
#[utoipa::path(
    post,
    path = "/api/v1/clients/{client_index}/mute/toggle",
    params(
        ("client_index" = usize, Path, description = "1-based index of the target client")
    ),
    responses(
        (status = 200, description = "New client mute status after toggling", body = bool),
        (status = 404, description = "Client not found", body = ErrorBody)
    ),
    tag = "clients"
)]
async fn toggle_mute(
    State(state): State<SharedState>,
    Path(idx): Path<usize>,
) -> impl IntoResponse {
    let client = read_client(&state, idx).await.ok_or(not_found())?;
    let muted = !client.muted;
    let snap_id = client.snapcast_id.clone().ok_or(not_found())?;
    let _ = state
        .snap_tx
        .send(SnapcastCmd::Client {
            client_id: snap_id,
            action: ClientAction::Mute(muted),
        })
        .await;
    // State update comes from Snapcast Client.OnVolumeChanged notification
    tracing::debug!(client = %client.name, muted, "Mute toggled");
    Ok::<_, ApiError>(Json(muted))
}

/// Get client latency setting
///
/// Returns the manual latency correction in milliseconds for the specified client.
#[utoipa::path(
    get,
    path = "/api/v1/clients/{client_index}/latency",
    params(
        ("client_index" = usize, Path, description = "1-based index of the target client")
    ),
    responses(
        (status = 200, description = "Current client latency in milliseconds", body = i32),
        (status = 404, description = "Client not found", body = ErrorBody)
    ),
    tag = "clients"
)]
async fn get_latency(
    State(state): State<SharedState>,
    Path(idx): Path<usize>,
) -> impl IntoResponse {
    read_client(&state, idx)
        .await
        .map(|c| Json(c.latency_ms))
        .ok_or(not_found())
}

/// Set client latency setting
///
/// Updates the manual latency correction (in milliseconds) for the specified client.
#[utoipa::path(
    put,
    path = "/api/v1/clients/{client_index}/latency",
    params(
        ("client_index" = usize, Path, description = "1-based index of the target client")
    ),
    request_body = i32,
    responses(
        (status = 200, description = "Updated client latency in milliseconds", body = i32),
        (status = 404, description = "Client not found", body = ErrorBody)
    ),
    tag = "clients"
)]
async fn set_latency(
    State(state): State<SharedState>,
    Path(idx): Path<usize>,
    Json(v): Json<i32>,
) -> impl IntoResponse {
    let snap_id = read_client(&state, idx)
        .await
        .and_then(|c| c.snapcast_id)
        .ok_or(not_found())?;
    let _ = state
        .snap_tx
        .send(SnapcastCmd::Client {
            client_id: snap_id,
            action: ClientAction::Latency(v),
        })
        .await;
    tracing::debug!(client = idx, latency = v, "Latency set");
    Ok::<_, ApiError>(Json(v))
}

/// Get client zone assignment
///
/// Returns the index of the zone that the specified client is currently playing from.
#[utoipa::path(
    get,
    path = "/api/v1/clients/{client_index}/zone",
    params(
        ("client_index" = usize, Path, description = "1-based index of the target client")
    ),
    responses(
        (status = 200, description = "Assigned zone index", body = usize),
        (status = 404, description = "Client not found", body = ErrorBody)
    ),
    tag = "clients"
)]
async fn get_zone(State(state): State<SharedState>, Path(idx): Path<usize>) -> impl IntoResponse {
    read_client(&state, idx)
        .await
        .map(|c| Json(c.zone_index))
        .ok_or(not_found())
}

/// Set client zone assignment
///
/// Assigns the specified client to a target zone.
#[utoipa::path(
    put,
    path = "/api/v1/clients/{client_index}/zone",
    params(
        ("client_index" = usize, Path, description = "1-based index of the target client")
    ),
    request_body = usize,
    responses(
        (status = 200, description = "Assigned zone index", body = usize),
        (status = 404, description = "Client or Zone not found", body = ErrorBody)
    ),
    tag = "clients"
)]
async fn set_zone(
    State(state): State<SharedState>,
    Path(idx): Path<usize>,
    Json(target_zone): Json<usize>,
) -> impl IntoResponse {
    if !state.config.zones.iter().any(|z| z.index == target_zone) {
        return Err(ApiError::NotFound("zone"));
    }
    if !state.store.read().await.clients.contains_key(&idx) {
        return Err(not_found());
    }

    // Send fade-out to client before switching zones (only SnapDog clients support it)
    let fade_ms = state.config.audio.zone_switch_fade_ms;
    let is_snapdog = if fade_ms > 0 {
        let (is_sd, snap_id) = {
            let s = state.store.read().await;
            s.clients
                .get(&idx)
                .map_or((false, None), |c| (c.is_snapdog, c.snapcast_id.clone()))
        };
        if is_sd {
            if let Some(snap_id) = snap_id {
                let _ = state
                    .snap_tx
                    .send(SnapcastCmd::Client {
                        client_id: snap_id,
                        action: ClientAction::SendCustom {
                            type_id: snapdog_common::MSG_TYPE_FADE_OUT,
                            payload: fade_ms.to_le_bytes().to_vec(),
                        },
                    })
                    .await;
            }
        }
        is_sd
    } else {
        false
    };

    // Wait for client fade-out to complete. No ack mechanism exists;
    // the sleep duration matches the fade the client is performing.
    if is_snapdog {
        tokio::time::sleep(std::time::Duration::from_millis(u64::from(fade_ms))).await;
    }

    // Update state (zone assignment is SnapDog-owned)
    crate::state::update_client_and_notify(&state.store, idx, &state.notifications, |c| {
        c.zone_index = target_zone;
    })
    .await;

    // Tell main loop to reconcile Snapcast groups
    let _ = state.snap_tx.send(SnapcastCmd::ReconcileZones).await;

    tracing::info!(client = idx, zone = target_zone, "Client zone changed");
    Ok::<_, ApiError>(Json(target_zone))
}

/// Get client friendly name
///
/// Returns the display name of the specified client.
#[utoipa::path(
    get,
    path = "/api/v1/clients/{client_index}/name",
    params(
        ("client_index" = usize, Path, description = "1-based index of the target client")
    ),
    responses(
        (status = 200, description = "Client friendly name", body = String),
        (status = 404, description = "Client not found", body = ErrorBody)
    ),
    tag = "clients"
)]
async fn get_name(State(state): State<SharedState>, Path(idx): Path<usize>) -> impl IntoResponse {
    read_client(&state, idx)
        .await
        .map(|c| Json(c.name))
        .ok_or(not_found())
}

/// Set client friendly name
///
/// Updates the friendly name of the specified client.
#[utoipa::path(
    put,
    path = "/api/v1/clients/{client_index}/name",
    params(
        ("client_index" = usize, Path, description = "1-based index of the target client")
    ),
    request_body = String,
    responses(
        (status = 200, description = "Updated friendly name", body = String),
        (status = 404, description = "Client not found", body = ErrorBody)
    ),
    tag = "clients"
)]
async fn set_name(
    State(state): State<SharedState>,
    Path(idx): Path<usize>,
    Json(v): Json<String>,
) -> impl IntoResponse {
    let name = v.clone();
    crate::state::update_client_and_notify(&state.store, idx, &state.notifications, |c| c.name = v)
        .await;
    tracing::debug!(client = idx, name = %name, "Name set");
    Ok::<_, ApiError>(Json(name))
}

/// Get client icon name
///
/// Returns the icon identifier string configured for the specified client.
#[utoipa::path(
    get,
    path = "/api/v1/clients/{client_index}/icon",
    params(
        ("client_index" = usize, Path, description = "1-based index of the target client")
    ),
    responses(
        (status = 200, description = "Client icon name", body = String),
        (status = 404, description = "Client not found", body = ErrorBody)
    ),
    tag = "clients"
)]
async fn get_icon(State(state): State<SharedState>, Path(idx): Path<usize>) -> impl IntoResponse {
    read_client(&state, idx)
        .await
        .map(|c| Json(c.icon))
        .ok_or(not_found())
}

/// Get client connection status
///
/// Returns whether the specified client is currently online and connected to the Snapcast server.
#[utoipa::path(
    get,
    path = "/api/v1/clients/{client_index}/connected",
    params(
        ("client_index" = usize, Path, description = "1-based index of the target client")
    ),
    responses(
        (status = 200, description = "Connection status", body = bool),
        (status = 404, description = "Client not found", body = ErrorBody)
    ),
    tag = "clients"
)]
async fn get_connected(
    State(state): State<SharedState>,
    Path(idx): Path<usize>,
) -> impl IntoResponse {
    read_client(&state, idx)
        .await
        .map(|c| Json(c.connected))
        .ok_or(not_found())
}
