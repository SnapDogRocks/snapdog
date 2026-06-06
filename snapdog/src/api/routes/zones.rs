// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! Zone endpoints: /api/v1/zones

use axum::extract::{Path, State};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use crate::api::SharedState;
use crate::api::error::{ApiError, ErrorBody};
use crate::player::ZoneCommand;
use crate::state;
use snapdog_common::RepeatMode;

/// Embedded `SnapDog` icon PNG (1024×1024) used as placeholder when no cover art is available.
const PLACEHOLDER_COVER: &[u8] = include_bytes!("../../../../assets/snapdog-icon-placeholder.png");

/// Volume value: absolute (e.g. `75`) or relative (e.g. `"+5"`, `"-3"`).
#[derive(Debug, Deserialize, utoipa::ToSchema)]
#[serde(untagged)]
pub enum VolumeValue {
    Absolute(i32),
    Relative(String),
}

impl VolumeValue {
    pub fn resolve(&self, current: i32) -> Result<i32, &'static str> {
        let v = match self {
            Self::Absolute(v) => *v,
            Self::Relative(s) => {
                let delta: i32 = s
                    .parse()
                    .map_err(|_| "Invalid relative volume (use e.g. \"+5\" or \"-3\")")?;
                current + delta
            }
        };
        Ok(v.clamp(0, 100))
    }
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct ZoneInfo {
    index: usize,
    name: String,
    icon: String,
    volume: i32,
    muted: bool,
    playback: String,
    source: String,
    shuffle: bool,
    repeat: snapdog_common::RepeatMode,
    presence: bool,
    presence_enabled: bool,
    presence_timer_active: bool,
}

pub fn router(state: SharedState) -> Router {
    Router::new()
        .route("/count", get(get_count))
        .route("/", get(get_all))
        .route("/{zone_index}", get(get_zone))
        .route("/{zone_index}/volume", get(get_volume).put(set_volume))
        .route("/{zone_index}/mute", get(get_mute).put(set_mute))
        .route("/{zone_index}/mute/toggle", post(toggle_mute))
        .route("/{zone_index}/play", post(play))
        .route("/{zone_index}/pause", post(pause))
        .route("/{zone_index}/stop", post(stop))
        .route("/{zone_index}/next", post(next_track))
        .route("/{zone_index}/previous", post(previous_track))
        .route(
            "/{zone_index}/playlist",
            get(get_playlist).put(set_playlist),
        )
        .route("/{zone_index}/next/playlist", post(next_playlist))
        .route("/{zone_index}/previous/playlist", post(previous_playlist))
        .route("/{zone_index}/shuffle", get(get_shuffle).put(set_shuffle))
        .route("/{zone_index}/shuffle/toggle", post(toggle_shuffle))
        .route("/{zone_index}/repeat", get(get_repeat).put(set_repeat))
        .route("/{zone_index}/repeat/toggle", post(toggle_repeat))
        .route("/{zone_index}/track", get(get_track))
        .route("/{zone_index}/track/metadata", get(get_track_metadata))
        .route("/{zone_index}/cover", get(get_zone_cover))
        .route("/{zone_index}/track/title", get(get_track_title))
        .route("/{zone_index}/track/artist", get(get_track_artist))
        .route("/{zone_index}/track/album", get(get_track_album))
        .route("/{zone_index}/track/duration", get(get_track_duration))
        .route(
            "/{zone_index}/track/position",
            get(get_track_position).put(seek_position),
        )
        .route(
            "/{zone_index}/track/progress",
            get(get_track_progress).put(seek_progress),
        )
        .route("/{zone_index}/track/playing", get(get_track_playing))
        .route("/{zone_index}/play/track", post(play_track))
        .route("/{zone_index}/play/url", post(play_url))
        .route("/{zone_index}/play/playlist", post(play_subsonic_playlist))
        .route(
            "/{zone_index}/play/playlist/{playlist_index}/track",
            post(play_playlist_track),
        )
        .route(
            "/{zone_index}/play/subsonic/{track_id}",
            post(play_subsonic_track),
        )
        .route("/{zone_index}/name", get(get_name))
        .route("/{zone_index}/icon", get(get_icon))
        .route("/{zone_index}/playback", get(get_playback))
        .route("/{zone_index}/playlist/name", get(get_playlist_name))
        .route("/{zone_index}/playlist/info", get(get_playlist_info))
        .route("/{zone_index}/playlist/count", get(get_playlist_count))
        .route("/{zone_index}/clients", get(get_clients))
        .route(
            "/{zone_index}/presence",
            get(get_presence).put(set_presence),
        )
        .route(
            "/{zone_index}/presence/enable",
            get(get_presence_enabled).put(set_presence_enabled),
        )
        .route(
            "/{zone_index}/presence/timeout",
            get(get_presence_timeout).put(set_presence_timeout),
        )
        .route("/{zone_index}/presence/timer", get(get_presence_timer))
        .with_state(state)
}

// ── Helpers ───────────────────────────────────────────────────

async fn read_zone(state: &SharedState, idx: usize) -> Option<state::ZoneState> {
    state.store.read().await.zones.get(&idx).cloned()
}

const fn zone_not_found() -> ApiError {
    ApiError::NotFound("zone")
}

async fn send_cmd(state: &SharedState, idx: usize, cmd: ZoneCommand) -> Result<(), ApiError> {
    state
        .zone_commands
        .get(&idx)
        .ok_or(ApiError::NotFound("zone"))?
        .send(cmd)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))
}

fn require_http_url(url: &str) -> Result<(), ApiError> {
    if url::Url::parse(url)
        .is_ok_and(|u| matches!(u.scheme(), "http" | "https") && u.host().is_some())
    {
        Ok(())
    } else {
        Err(ApiError::BadRequest(
            "Only absolute http and https URLs are supported".into(),
        ))
    }
}

// ── Zone listing ──────────────────────────────────────────────

/// Get total number of zones
///
/// Returns the number of configured zones in the system.
#[utoipa::path(
    get,
    path = "/api/v1/zones/count",
    responses(
        (status = 200, description = "Total zone count", body = usize)
    ),
    tag = "zones"
)]
async fn get_count(State(state): State<SharedState>) -> Json<usize> {
    Json(state.config.zones.len())
}

/// Get all zones details
///
/// Returns a list of all zones, including volume, mute status, playback state, and presence settings.
#[utoipa::path(
    get,
    path = "/api/v1/zones",
    responses(
        (status = 200, description = "List of all zone details", body = Vec<ZoneInfo>)
    ),
    tag = "zones"
)]
async fn get_all(State(state): State<SharedState>) -> Json<Vec<ZoneInfo>> {
    let store = state.store.read().await;
    Json(
        state
            .config
            .zones
            .iter()
            .map(|z| {
                let zs = store.zones.get(&z.index);
                ZoneInfo {
                    index: z.index,
                    name: z.name.clone(),
                    icon: z.icon.clone(),
                    volume: zs.map_or(crate::state::DEFAULT_VOLUME, |s| s.volume),
                    muted: zs.is_some_and(|s| s.muted),
                    playback: zs.map_or_else(|| "stopped".into(), |s| s.playback.to_string()),
                    source: zs.map_or_else(|| "idle".into(), |s| s.source.to_string()),
                    shuffle: zs.is_some_and(|s| s.shuffle),
                    repeat: zs.map_or(snapdog_common::RepeatMode::Off, |s| s.repeat),
                    presence: zs.is_some_and(|s| s.presence),
                    presence_enabled: zs.is_none_or(|s| s.presence_enabled),
                    presence_timer_active: zs.is_some_and(|s| s.auto_off_active),
                }
            })
            .collect(),
    )
}

/// Get zone details by index
///
/// Returns the details of a single zone specified by its 1-based index.
#[utoipa::path(
    get,
    path = "/api/v1/zones/{zone_index}",
    params(
        ("zone_index" = usize, Path, description = "1-based index of the target zone")
    ),
    responses(
        (status = 200, description = "Zone details", body = ZoneInfo),
        (status = 404, description = "Zone not found", body = ErrorBody)
    ),
    tag = "zones"
)]
async fn get_zone(State(state): State<SharedState>, Path(idx): Path<usize>) -> impl IntoResponse {
    let cfg = idx
        .checked_sub(1)
        .and_then(|i| state.config.zones.get(i))
        .ok_or(zone_not_found())?;
    let zs = state.store.read().await.zones.get(&idx).cloned();
    Ok::<_, ApiError>(Json(ZoneInfo {
        index: cfg.index,
        name: cfg.name.clone(),
        icon: cfg.icon.clone(),
        volume: zs
            .as_ref()
            .map_or(crate::state::DEFAULT_VOLUME, |s| s.volume),
        muted: zs.as_ref().is_some_and(|s| s.muted),
        playback: zs
            .as_ref()
            .map_or_else(|| "stopped".into(), |s| s.playback.to_string()),
        source: zs
            .as_ref()
            .map_or_else(|| "idle".into(), |s| s.source.to_string()),
        shuffle: zs.as_ref().is_some_and(|s| s.shuffle),
        repeat: zs
            .as_ref()
            .map_or(snapdog_common::RepeatMode::Off, |s| s.repeat),
        presence: zs.as_ref().is_some_and(|s| s.presence),
        presence_enabled: zs.as_ref().is_none_or(|s| s.presence_enabled),
        presence_timer_active: zs.as_ref().is_some_and(|s| s.auto_off_active),
    }))
}

// ── Volume ────────────────────────────────────────────────────

/// Get zone volume level
///
/// Returns the current group volume level (0-100) of the specified zone.
#[utoipa::path(
    get,
    path = "/api/v1/zones/{zone_index}/volume",
    params(
        ("zone_index" = usize, Path, description = "1-based index of the target zone")
    ),
    responses(
        (status = 200, description = "Current zone volume", body = i32),
        (status = 404, description = "Zone not found", body = ErrorBody)
    ),
    tag = "zones"
)]
async fn get_volume(State(state): State<SharedState>, Path(idx): Path<usize>) -> impl IntoResponse {
    read_zone(&state, idx)
        .await
        .map(|z| Json(z.volume))
        .ok_or(zone_not_found())
}

/// Set zone volume level
///
/// Updates the volume level of the specified zone. Supports absolute or relative values (e.g. "+5", "-5", or "45").
#[utoipa::path(
    put,
    path = "/api/v1/zones/{zone_index}/volume",
    params(
        ("zone_index" = usize, Path, description = "1-based index of the target zone")
    ),
    request_body = VolumeValue,
    responses(
        (status = 200, description = "Updated zone volume", body = i32),
        (status = 400, description = "Invalid volume value", body = ErrorBody),
        (status = 404, description = "Zone not found", body = ErrorBody)
    ),
    tag = "zones"
)]
async fn set_volume(
    State(state): State<SharedState>,
    Path(idx): Path<usize>,
    Json(value): Json<VolumeValue>,
) -> impl IntoResponse {
    let current = read_zone(&state, idx)
        .await
        .map_or(crate::state::DEFAULT_VOLUME, |z| z.volume);
    let volume = value
        .resolve(current)
        .map_err(|e| ApiError::BadRequest(e.to_string()))?;
    send_cmd(&state, idx, ZoneCommand::SetVolume(volume)).await?;
    Ok::<_, ApiError>(Json(volume))
}

/// Get zone mute status
///
/// Returns whether the specified zone is currently muted.
#[utoipa::path(
    get,
    path = "/api/v1/zones/{zone_index}/mute",
    params(
        ("zone_index" = usize, Path, description = "1-based index of the target zone")
    ),
    responses(
        (status = 200, description = "Zone mute status", body = bool),
        (status = 404, description = "Zone not found", body = ErrorBody)
    ),
    tag = "zones"
)]
async fn get_mute(State(state): State<SharedState>, Path(idx): Path<usize>) -> impl IntoResponse {
    read_zone(&state, idx)
        .await
        .map(|z| Json(z.muted))
        .ok_or(zone_not_found())
}

/// Set zone mute status
///
/// Mutes or unmutes all outputs assigned to the specified zone.
#[utoipa::path(
    put,
    path = "/api/v1/zones/{zone_index}/mute",
    params(
        ("zone_index" = usize, Path, description = "1-based index of the target zone")
    ),
    request_body = bool,
    responses(
        (status = 200, description = "Updated mute status"),
        (status = 404, description = "Zone not found", body = ErrorBody)
    ),
    tag = "zones"
)]
async fn set_mute(
    State(state): State<SharedState>,
    Path(idx): Path<usize>,
    Json(v): Json<bool>,
) -> impl IntoResponse {
    send_cmd(&state, idx, ZoneCommand::SetMute(v)).await
}

/// Toggle zone mute status
///
/// Toggles the mute status of the specified zone.
#[utoipa::path(
    post,
    path = "/api/v1/zones/{zone_index}/mute/toggle",
    params(
        ("zone_index" = usize, Path, description = "1-based index of the target zone")
    ),
    responses(
        (status = 200, description = "Mute toggled successfully"),
        (status = 404, description = "Zone not found", body = ErrorBody)
    ),
    tag = "zones"
)]
async fn toggle_mute(
    State(state): State<SharedState>,
    Path(idx): Path<usize>,
) -> impl IntoResponse {
    send_cmd(&state, idx, ZoneCommand::ToggleMute).await
}

// ── Playback control ──────────────────────────────────────────

/// Resume zone playback
///
/// Resumes playback in the specified zone.
#[utoipa::path(
    post,
    path = "/api/v1/zones/{zone_index}/play",
    params(
        ("zone_index" = usize, Path, description = "1-based index of the target zone")
    ),
    responses(
        (status = 200, description = "Playback resumed"),
        (status = 404, description = "Zone not found", body = ErrorBody)
    ),
    tag = "zones"
)]
async fn play(State(state): State<SharedState>, Path(idx): Path<usize>) -> impl IntoResponse {
    send_cmd(&state, idx, ZoneCommand::Play).await
}

/// Pause zone playback
///
/// Pauses playback in the specified zone.
#[utoipa::path(
    post,
    path = "/api/v1/zones/{zone_index}/pause",
    params(
        ("zone_index" = usize, Path, description = "1-based index of the target zone")
    ),
    responses(
        (status = 200, description = "Playback paused"),
        (status = 404, description = "Zone not found", body = ErrorBody)
    ),
    tag = "zones"
)]
async fn pause(State(state): State<SharedState>, Path(idx): Path<usize>) -> impl IntoResponse {
    send_cmd(&state, idx, ZoneCommand::Pause).await
}

/// Stop zone playback
///
/// Stops playback and clears the active stream in the specified zone.
#[utoipa::path(
    post,
    path = "/api/v1/zones/{zone_index}/stop",
    params(
        ("zone_index" = usize, Path, description = "1-based index of the target zone")
    ),
    responses(
        (status = 200, description = "Playback stopped"),
        (status = 404, description = "Zone not found", body = ErrorBody)
    ),
    tag = "zones"
)]
async fn stop(State(state): State<SharedState>, Path(idx): Path<usize>) -> impl IntoResponse {
    send_cmd(&state, idx, ZoneCommand::Stop).await
}

/// Skip to next track
///
/// Skips to the next track in the playlist for the specified zone.
#[utoipa::path(
    post,
    path = "/api/v1/zones/{zone_index}/next",
    params(
        ("zone_index" = usize, Path, description = "1-based index of the target zone")
    ),
    responses(
        (status = 200, description = "Skipped to next track"),
        (status = 404, description = "Zone not found", body = ErrorBody)
    ),
    tag = "zones"
)]
async fn next_track(State(state): State<SharedState>, Path(idx): Path<usize>) -> impl IntoResponse {
    send_cmd(&state, idx, ZoneCommand::Next).await
}

/// Skip to previous track
///
/// Skips to the previous track in the playlist for the specified zone.
#[utoipa::path(
    post,
    path = "/api/v1/zones/{zone_index}/previous",
    params(
        ("zone_index" = usize, Path, description = "1-based index of the target zone")
    ),
    responses(
        (status = 200, description = "Skipped to previous track"),
        (status = 404, description = "Zone not found", body = ErrorBody)
    ),
    tag = "zones"
)]
async fn previous_track(
    State(state): State<SharedState>,
    Path(idx): Path<usize>,
) -> impl IntoResponse {
    send_cmd(&state, idx, ZoneCommand::Previous).await
}

// ── Playlist ──────────────────────────────────────────────────

/// Get zone playlist index
///
/// Returns the index of the currently active unified playlist in the specified zone.
#[utoipa::path(
    get,
    path = "/api/v1/zones/{zone_index}/playlist",
    params(
        ("zone_index" = usize, Path, description = "1-based index of the target zone")
    ),
    responses(
        (status = 200, description = "Active playlist index", body = usize),
        (status = 404, description = "Zone not found", body = ErrorBody)
    ),
    tag = "zones"
)]
async fn get_playlist(
    State(state): State<SharedState>,
    Path(idx): Path<usize>,
) -> impl IntoResponse {
    read_zone(&state, idx)
        .await
        .map(|z| Json(z.playlist_index.unwrap_or(0)))
        .ok_or(zone_not_found())
}

/// Set zone playlist
///
/// Selects the specified unified playlist index. If the zone is already playing, playback switches
/// to the selected playlist; otherwise the playlist is loaded and can be started with transport play.
#[utoipa::path(
    put,
    path = "/api/v1/zones/{zone_index}/playlist",
    params(
        ("zone_index" = usize, Path, description = "1-based index of the target zone")
    ),
    request_body = usize,
    responses(
        (status = 200, description = "Playlist changed successfully"),
        (status = 404, description = "Zone not found", body = ErrorBody)
    ),
    tag = "zones"
)]
async fn set_playlist(
    State(state): State<SharedState>,
    Path(idx): Path<usize>,
    Json(v): Json<usize>,
) -> impl IntoResponse {
    send_cmd(&state, idx, ZoneCommand::SetPlaylist(v, 0)).await
}

/// Switch to next playlist
///
/// Switches the zone to the next available unified playlist.
#[utoipa::path(
    post,
    path = "/api/v1/zones/{zone_index}/next/playlist",
    params(
        ("zone_index" = usize, Path, description = "1-based index of the target zone")
    ),
    responses(
        (status = 200, description = "Switched to next playlist"),
        (status = 404, description = "Zone not found", body = ErrorBody)
    ),
    tag = "zones"
)]
async fn next_playlist(
    State(state): State<SharedState>,
    Path(idx): Path<usize>,
) -> impl IntoResponse {
    send_cmd(&state, idx, ZoneCommand::NextPlaylist).await
}

/// Switch to previous playlist
///
/// Switches the zone to the previous available unified playlist.
#[utoipa::path(
    post,
    path = "/api/v1/zones/{zone_index}/previous/playlist",
    params(
        ("zone_index" = usize, Path, description = "1-based index of the target zone")
    ),
    responses(
        (status = 200, description = "Switched to previous playlist"),
        (status = 404, description = "Zone not found", body = ErrorBody)
    ),
    tag = "zones"
)]
async fn previous_playlist(
    State(state): State<SharedState>,
    Path(idx): Path<usize>,
) -> impl IntoResponse {
    send_cmd(&state, idx, ZoneCommand::PreviousPlaylist).await
}

/// Get zone shuffle status
///
/// Returns whether playlist shuffle mode is enabled in the specified zone.
#[utoipa::path(
    get,
    path = "/api/v1/zones/{zone_index}/shuffle",
    params(
        ("zone_index" = usize, Path, description = "1-based index of the target zone")
    ),
    responses(
        (status = 200, description = "Shuffle status", body = bool),
        (status = 404, description = "Zone not found", body = ErrorBody)
    ),
    tag = "zones"
)]
async fn get_shuffle(
    State(state): State<SharedState>,
    Path(idx): Path<usize>,
) -> impl IntoResponse {
    read_zone(&state, idx)
        .await
        .map(|z| Json(z.shuffle))
        .ok_or(zone_not_found())
}

/// Set zone shuffle status
///
/// Enables or disables playlist shuffle mode in the specified zone.
#[utoipa::path(
    put,
    path = "/api/v1/zones/{zone_index}/shuffle",
    params(
        ("zone_index" = usize, Path, description = "1-based index of the target zone")
    ),
    request_body = bool,
    responses(
        (status = 200, description = "Shuffle status updated"),
        (status = 404, description = "Zone not found", body = ErrorBody)
    ),
    tag = "zones"
)]
async fn set_shuffle(
    State(state): State<SharedState>,
    Path(idx): Path<usize>,
    Json(v): Json<bool>,
) -> impl IntoResponse {
    send_cmd(&state, idx, ZoneCommand::SetShuffle(v)).await
}

/// Toggle zone shuffle status
///
/// Toggles the playlist shuffle mode in the specified zone.
#[utoipa::path(
    post,
    path = "/api/v1/zones/{zone_index}/shuffle/toggle",
    params(
        ("zone_index" = usize, Path, description = "1-based index of the target zone")
    ),
    responses(
        (status = 200, description = "Shuffle status toggled"),
        (status = 404, description = "Zone not found", body = ErrorBody)
    ),
    tag = "zones"
)]
async fn toggle_shuffle(
    State(state): State<SharedState>,
    Path(idx): Path<usize>,
) -> impl IntoResponse {
    send_cmd(&state, idx, ZoneCommand::ToggleShuffle).await
}

/// Get zone repeat mode
///
/// Returns the current playlist repeat mode for the specified zone.
#[utoipa::path(
    get,
    path = "/api/v1/zones/{zone_index}/repeat",
    params(
        ("zone_index" = usize, Path, description = "1-based index of the target zone")
    ),
    responses(
        (status = 200, description = "Current repeat mode", body = RepeatMode),
        (status = 404, description = "Zone not found", body = ErrorBody)
    ),
    tag = "zones"
)]
async fn get_repeat(State(state): State<SharedState>, Path(idx): Path<usize>) -> impl IntoResponse {
    read_zone(&state, idx)
        .await
        .map(|z| Json(z.repeat))
        .ok_or(zone_not_found())
}

/// Set zone repeat mode
///
/// Updates the playlist repeat mode ("off", "all", "one") for the specified zone.
#[utoipa::path(
    put,
    path = "/api/v1/zones/{zone_index}/repeat",
    params(
        ("zone_index" = usize, Path, description = "1-based index of the target zone")
    ),
    request_body = RepeatMode,
    responses(
        (status = 200, description = "Repeat mode updated"),
        (status = 404, description = "Zone not found", body = ErrorBody)
    ),
    tag = "zones"
)]
async fn set_repeat(
    State(state): State<SharedState>,
    Path(idx): Path<usize>,
    Json(v): Json<snapdog_common::RepeatMode>,
) -> impl IntoResponse {
    send_cmd(&state, idx, ZoneCommand::SetRepeat(v)).await
}

/// Toggle zone repeat mode
///
/// Cycles the repeat mode to the next option (Off -> All -> One -> Off) for the specified zone.
#[utoipa::path(
    post,
    path = "/api/v1/zones/{zone_index}/repeat/toggle",
    params(
        ("zone_index" = usize, Path, description = "1-based index of the target zone")
    ),
    responses(
        (status = 200, description = "Repeat mode toggled"),
        (status = 404, description = "Zone not found", body = ErrorBody)
    ),
    tag = "zones"
)]
async fn toggle_repeat(
    State(state): State<SharedState>,
    Path(idx): Path<usize>,
) -> impl IntoResponse {
    send_cmd(&state, idx, ZoneCommand::CycleRepeat).await
}

// ── Track info ────────────────────────────────────────────────

/// Get active track index
///
/// Returns the index of the currently active track in the playlist for the specified zone.
#[utoipa::path(
    get,
    path = "/api/v1/zones/{zone_index}/track",
    params(
        ("zone_index" = usize, Path, description = "1-based index of the target zone")
    ),
    responses(
        (status = 200, description = "Current track index (0-based)", body = i32),
        (status = 404, description = "Zone not found", body = ErrorBody)
    ),
    tag = "zones"
)]
async fn get_track(State(state): State<SharedState>, Path(idx): Path<usize>) -> impl IntoResponse {
    read_zone(&state, idx)
        .await
        .map(|z| {
            #[allow(clippy::cast_possible_wrap)]
            let idx = z.playlist_track_index.unwrap_or(0) as i32;
            Json(idx)
        })
        .ok_or(zone_not_found())
}

/// Get track metadata
///
/// Returns detailed metadata (title, artist, album, duration, etc.) for the currently playing track in the zone.
#[utoipa::path(
    get,
    path = "/api/v1/zones/{zone_index}/track/metadata",
    params(
        ("zone_index" = usize, Path, description = "1-based index of the target zone")
    ),
    responses(
        (status = 200, description = "Current track metadata", body = ZoneTrackMetadata),
        (status = 404, description = "Zone not found", body = ErrorBody)
    ),
    tag = "zones"
)]
async fn get_track_metadata(
    State(state): State<SharedState>,
    Path(idx): Path<usize>,
) -> impl IntoResponse {
    let zone = read_zone(&state, idx).await.ok_or(zone_not_found())?;
    Ok::<_, ApiError>(Json(ZoneTrackMetadata {
        title: zone
            .track
            .as_ref()
            .map_or(String::new(), |t| t.title.clone()),
        artist: zone
            .track
            .as_ref()
            .map_or(String::new(), |t| t.artist.clone()),
        album: zone
            .track
            .as_ref()
            .map_or(String::new(), |t| t.album.clone()),
        album_artist: zone.track.as_ref().and_then(|t| t.album_artist.clone()),
        genre: zone.track.as_ref().and_then(|t| t.genre.clone()),
        year: zone.track.as_ref().and_then(|t| t.year),
        track_number: zone.track.as_ref().and_then(|t| t.track_number),
        disc_number: zone.track.as_ref().and_then(|t| t.disc_number),
        duration_ms: zone.track.as_ref().map_or(0, |t| t.duration_ms),
        position_ms: zone.track.as_ref().map_or(0, |t| t.position_ms),
        seekable: zone.track.as_ref().is_some_and(|t| t.seekable),
        bitrate_kbps: zone.track.as_ref().and_then(|t| t.bitrate_kbps),
        content_type: zone.track.as_ref().and_then(|t| t.content_type.clone()),
        sample_rate: zone.track.as_ref().and_then(|t| t.sample_rate),
        source: zone.source.to_string(),
        cover_url: zone.cover_url.clone(),
        playlist_index: zone.playlist_index,
        playlist_name: zone.playlist_name.clone(),
        playlist_total: zone.playlist_total,
        playlist_track_index: zone.playlist_track_index,
        playlist_track_count: zone.playlist_track_count,
        can_next: zone
            .playlist_track_count
            .is_some_and(|c| zone.playlist_track_index.is_some_and(|i| i + 1 < c)),
        can_prev: zone.playlist_track_index.is_some_and(|i| i > 0),
    }))
}

/// Get zone cover art image
///
/// Returns the cover art image binary (JPEG/PNG) of the currently playing track in the zone.
/// Defaults to a placeholder icon when no cover art is loaded.
#[utoipa::path(
    get,
    path = "/api/v1/zones/{zone_index}/cover",
    params(
        ("zone_index" = usize, Path, description = "1-based index of the target zone")
    ),
    responses(
        (status = 200, description = "Zone cover art image binary", content_type = "image/*")
    ),
    tag = "zones"
)]
async fn get_zone_cover(
    State(state): State<SharedState>,
    Path(idx): Path<usize>,
) -> impl IntoResponse {
    let cache = state.covers.read().await;
    cache.get(idx).map_or_else(
        || {
            (
                [
                    ("content-type", "image/png".to_string()),
                    ("cache-control", "public, max-age=604800".to_string()),
                    ("etag", "\"snapdog-placeholder\"".to_string()),
                ],
                PLACEHOLDER_COVER.to_vec(),
            )
        },
        |entry| {
            (
                [
                    ("content-type", entry.mime.clone()),
                    ("cache-control", "public, max-age=86400, immutable".into()),
                    ("etag", format!("\"{}\"", entry.hash)),
                ],
                entry.bytes.clone(),
            )
        },
    )
}

/// Get active track title
///
/// Returns the title of the track currently playing in the specified zone.
#[utoipa::path(
    get,
    path = "/api/v1/zones/{zone_index}/track/title",
    params(
        ("zone_index" = usize, Path, description = "1-based index of the target zone")
    ),
    responses(
        (status = 200, description = "Track title", body = String),
        (status = 404, description = "Zone not found", body = ErrorBody)
    ),
    tag = "zones"
)]
async fn get_track_title(
    State(state): State<SharedState>,
    Path(idx): Path<usize>,
) -> impl IntoResponse {
    read_zone(&state, idx)
        .await
        .map(|z| Json(z.track.map_or(String::new(), |t| t.title)))
        .ok_or(zone_not_found())
}

/// Get active track artist
///
/// Returns the artist name of the track currently playing in the specified zone.
#[utoipa::path(
    get,
    path = "/api/v1/zones/{zone_index}/track/artist",
    params(
        ("zone_index" = usize, Path, description = "1-based index of the target zone")
    ),
    responses(
        (status = 200, description = "Track artist name", body = String),
        (status = 404, description = "Zone not found", body = ErrorBody)
    ),
    tag = "zones"
)]
async fn get_track_artist(
    State(state): State<SharedState>,
    Path(idx): Path<usize>,
) -> impl IntoResponse {
    read_zone(&state, idx)
        .await
        .map(|z| Json(z.track.map_or(String::new(), |t| t.artist)))
        .ok_or(zone_not_found())
}

/// Get active track album
///
/// Returns the album title of the track currently playing in the specified zone.
#[utoipa::path(
    get,
    path = "/api/v1/zones/{zone_index}/track/album",
    params(
        ("zone_index" = usize, Path, description = "1-based index of the target zone")
    ),
    responses(
        (status = 200, description = "Track album title", body = String),
        (status = 404, description = "Zone not found", body = ErrorBody)
    ),
    tag = "zones"
)]
async fn get_track_album(
    State(state): State<SharedState>,
    Path(idx): Path<usize>,
) -> impl IntoResponse {
    read_zone(&state, idx)
        .await
        .map(|z| Json(z.track.map_or(String::new(), |t| t.album)))
        .ok_or(zone_not_found())
}

/// Get active track duration
///
/// Returns the total duration of the currently active track in milliseconds.
#[utoipa::path(
    get,
    path = "/api/v1/zones/{zone_index}/track/duration",
    params(
        ("zone_index" = usize, Path, description = "1-based index of the target zone")
    ),
    responses(
        (status = 200, description = "Track duration in milliseconds", body = i64),
        (status = 404, description = "Zone not found", body = ErrorBody)
    ),
    tag = "zones"
)]
async fn get_track_duration(
    State(state): State<SharedState>,
    Path(idx): Path<usize>,
) -> impl IntoResponse {
    read_zone(&state, idx)
        .await
        .map(|z| Json(z.track.map_or(0i64, |t| t.duration_ms)))
        .ok_or(zone_not_found())
}

/// Get active track playback position
///
/// Returns the current playback position of the active track in milliseconds.
#[utoipa::path(
    get,
    path = "/api/v1/zones/{zone_index}/track/position",
    params(
        ("zone_index" = usize, Path, description = "1-based index of the target zone")
    ),
    responses(
        (status = 200, description = "Current playback position in milliseconds", body = i64),
        (status = 404, description = "Zone not found", body = ErrorBody)
    ),
    tag = "zones"
)]
async fn get_track_position(
    State(state): State<SharedState>,
    Path(idx): Path<usize>,
) -> impl IntoResponse {
    read_zone(&state, idx)
        .await
        .map(|z| Json(z.track.map_or(0i64, |t| t.position_ms)))
        .ok_or(zone_not_found())
}

/// Seek track position
///
/// Seeks to a specific timestamp or offset in milliseconds. Exactly one field must be provided.
#[utoipa::path(
    put,
    path = "/api/v1/zones/{zone_index}/track/position",
    params(
        ("zone_index" = usize, Path, description = "1-based index of the target zone")
    ),
    request_body = SeekPayload,
    responses(
        (status = 200, description = "Seek command processed"),
        (status = 400, description = "Invalid payload (both or neither field provided)", body = ErrorBody),
        (status = 404, description = "Zone not found", body = ErrorBody)
    ),
    tag = "zones"
)]
async fn seek_position(
    State(state): State<SharedState>,
    Path(idx): Path<usize>,
    Json(body): Json<SeekPayload>,
) -> impl IntoResponse {
    let cmd = match (body.position_ms, body.offset_ms) {
        (Some(pos), None) => ZoneCommand::Seek(pos),
        (None, Some(offset)) => ZoneCommand::SeekRelative(offset),
        _ => {
            return Err(ApiError::BadRequest(
                "provide exactly one of position_ms or offset_ms".into(),
            ));
        }
    };
    send_cmd(&state, idx, cmd).await
}

#[derive(serde::Deserialize, utoipa::ToSchema)]
pub struct SeekPayload {
    position_ms: Option<i64>,
    offset_ms: Option<i64>,
}

/// Get track progress percentage
///
/// Returns the current playback progress as a fractional value between 0.0 and 1.0.
#[utoipa::path(
    get,
    path = "/api/v1/zones/{zone_index}/track/progress",
    params(
        ("zone_index" = usize, Path, description = "1-based index of the target zone")
    ),
    responses(
        (status = 200, description = "Track progress fraction (0.0 to 1.0)", body = f64),
        (status = 404, description = "Zone not found", body = ErrorBody)
    ),
    tag = "zones"
)]
async fn get_track_progress(
    State(state): State<SharedState>,
    Path(idx): Path<usize>,
) -> impl IntoResponse {
    let zone = read_zone(&state, idx).await.ok_or(zone_not_found())?;
    let progress = zone.track.map_or(0.0, |t| {
        if t.duration_ms > 0 {
            t.position_ms as f64 / t.duration_ms as f64
        } else {
            0.0
        }
    });
    Ok::<_, ApiError>(Json(progress))
}

/// Seek track progress percentage
///
/// Seeks to a specific progress fraction (0.0 to 1.0) of the track.
#[utoipa::path(
    put,
    path = "/api/v1/zones/{zone_index}/track/progress",
    params(
        ("zone_index" = usize, Path, description = "1-based index of the target zone")
    ),
    request_body = f64,
    responses(
        (status = 200, description = "Seek command processed"),
        (status = 404, description = "Zone not found", body = ErrorBody)
    ),
    tag = "zones"
)]
async fn seek_progress(
    State(state): State<SharedState>,
    Path(idx): Path<usize>,
    Json(v): Json<f64>,
) -> impl IntoResponse {
    send_cmd(&state, idx, ZoneCommand::SeekProgress(v)).await
}

/// Get track playing state
///
/// Returns whether the specified zone is currently playing audio (true) or paused/stopped (false).
#[utoipa::path(
    get,
    path = "/api/v1/zones/{zone_index}/track/playing",
    params(
        ("zone_index" = usize, Path, description = "1-based index of the target zone")
    ),
    responses(
        (status = 200, description = "True if active track is playing", body = bool),
        (status = 404, description = "Zone not found", body = ErrorBody)
    ),
    tag = "zones"
)]
async fn get_track_playing(
    State(state): State<SharedState>,
    Path(idx): Path<usize>,
) -> impl IntoResponse {
    read_zone(&state, idx)
        .await
        .map(|z| Json(z.playback == state::PlaybackState::Playing))
        .ok_or(zone_not_found())
}

// ── Play specific content ─────────────────────────────────────

/// Play specific track index
///
/// Starts playback of a specific track index within the active playlist.
#[utoipa::path(
    post,
    path = "/api/v1/zones/{zone_index}/play/track",
    params(
        ("zone_index" = usize, Path, description = "1-based index of the target zone")
    ),
    request_body = i32,
    responses(
        (status = 200, description = "Track play command sent"),
        (status = 404, description = "Zone not found", body = ErrorBody)
    ),
    tag = "zones"
)]
async fn play_track(
    State(state): State<SharedState>,
    Path(idx): Path<usize>,
    Json(v): Json<i32>,
) -> impl IntoResponse {
    send_cmd(&state, idx, ZoneCommand::SetTrack(v as usize)).await
}

/// Play arbitrary audio stream URL
///
/// Forces the zone to play a direct HTTP/HTTPS audio stream URL.
#[utoipa::path(
    post,
    path = "/api/v1/zones/{zone_index}/play/url",
    params(
        ("zone_index" = usize, Path, description = "1-based index of the target zone")
    ),
    request_body = String,
    responses(
        (status = 200, description = "URL playback started"),
        (status = 400, description = "Invalid URL schema (must be http or https)", body = ErrorBody),
        (status = 404, description = "Zone not found", body = ErrorBody)
    ),
    tag = "zones"
)]
async fn play_url(
    State(state): State<SharedState>,
    Path(idx): Path<usize>,
    Json(v): Json<String>,
) -> impl IntoResponse {
    require_http_url(&v)?;
    send_cmd(&state, idx, ZoneCommand::PlayUrl(v)).await
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct PlaylistPayload {
    id: usize,
    #[serde(default)]
    track: usize,
}

/// Play unified playlist
///
/// Switches the zone to a unified playlist index and starts playing at the specified track index.
#[utoipa::path(
    post,
    path = "/api/v1/zones/{zone_index}/play/playlist",
    params(
        ("zone_index" = usize, Path, description = "1-based index of the target zone")
    ),
    request_body = PlaylistPayload,
    responses(
        (status = 200, description = "Playlist playback triggered"),
        (status = 404, description = "Zone not found", body = ErrorBody)
    ),
    tag = "zones"
)]
async fn play_subsonic_playlist(
    State(state): State<SharedState>,
    Path(idx): Path<usize>,
    Json(v): Json<PlaylistPayload>,
) -> impl IntoResponse {
    send_cmd(&state, idx, ZoneCommand::PlayPlaylist(v.id, v.track)).await
}

/// Play track in a playlist
///
/// Starts playback of the specified track index in the specified unified playlist.
#[utoipa::path(
    post,
    path = "/api/v1/zones/{zone_index}/play/playlist/{playlist_index}/track",
    params(
        ("zone_index" = usize, Path, description = "1-based index of the target zone"),
        ("playlist_index" = usize, Path, description = "Index of the playlist")
    ),
    request_body = i32,
    responses(
        (status = 200, description = "Playlist track playback started"),
        (status = 404, description = "Zone or playlist not found", body = ErrorBody)
    ),
    tag = "zones"
)]
async fn play_playlist_track(
    State(state): State<SharedState>,
    Path((zone, playlist)): Path<(usize, usize)>,
    Json(v): Json<i32>,
) -> impl IntoResponse {
    let track = usize::try_from(v)
        .map_err(|_| ApiError::BadRequest("track index must be non-negative".into()))?;
    send_cmd(&state, zone, ZoneCommand::PlayPlaylist(playlist, track)).await
}

/// Play Subsonic track by ID
///
/// Plays a specific Subsonic track directly using its unique upstream alphanumeric ID.
#[utoipa::path(
    post,
    path = "/api/v1/zones/{zone_index}/play/subsonic/{track_id}",
    params(
        ("zone_index" = usize, Path, description = "1-based index of the target zone"),
        ("track_id" = String, Path, description = "Unique Subsonic track ID string")
    ),
    responses(
        (status = 200, description = "Subsonic track playback started"),
        (status = 404, description = "Zone or track not found", body = ErrorBody)
    ),
    tag = "zones"
)]
async fn play_subsonic_track(
    State(state): State<SharedState>,
    Path((idx, track_id)): Path<(usize, String)>,
) -> impl IntoResponse {
    send_cmd(&state, idx, ZoneCommand::PlaySubsonicTrack(track_id)).await
}

// ── Zone info ─────────────────────────────────────────────────

/// Get zone name
///
/// Returns the name of the specified zone.
#[utoipa::path(
    get,
    path = "/api/v1/zones/{zone_index}/name",
    params(
        ("zone_index" = usize, Path, description = "1-based index of the target zone")
    ),
    responses(
        (status = 200, description = "Zone name", body = String),
        (status = 404, description = "Zone not found", body = ErrorBody)
    ),
    tag = "zones"
)]
async fn get_name(State(state): State<SharedState>, Path(idx): Path<usize>) -> impl IntoResponse {
    read_zone(&state, idx)
        .await
        .map(|z| Json(z.name))
        .ok_or(zone_not_found())
}

/// Get zone emoji icon
///
/// Returns the emoji character configured for the specified zone.
#[utoipa::path(
    get,
    path = "/api/v1/zones/{zone_index}/icon",
    params(
        ("zone_index" = usize, Path, description = "1-based index of the target zone")
    ),
    responses(
        (status = 200, description = "Zone icon character", body = String),
        (status = 404, description = "Zone not found", body = ErrorBody)
    ),
    tag = "zones"
)]
async fn get_icon(State(state): State<SharedState>, Path(idx): Path<usize>) -> impl IntoResponse {
    read_zone(&state, idx)
        .await
        .map(|z| Json(z.icon))
        .ok_or(zone_not_found())
}

/// Get zone playback status
///
/// Returns the playback status string ("playing", "paused", "stopped").
#[utoipa::path(
    get,
    path = "/api/v1/zones/{zone_index}/playback",
    params(
        ("zone_index" = usize, Path, description = "1-based index of the target zone")
    ),
    responses(
        (status = 200, description = "Playback status string", body = String),
        (status = 404, description = "Zone not found", body = ErrorBody)
    ),
    tag = "zones"
)]
async fn get_playback(
    State(state): State<SharedState>,
    Path(idx): Path<usize>,
) -> impl IntoResponse {
    read_zone(&state, idx)
        .await
        .map(|z| Json(z.playback.to_string()))
        .ok_or(zone_not_found())
}

/// Get active playlist name
///
/// Returns the display name of the currently active playlist in the zone.
#[utoipa::path(
    get,
    path = "/api/v1/zones/{zone_index}/playlist/name",
    params(
        ("zone_index" = usize, Path, description = "1-based index of the target zone")
    ),
    responses(
        (status = 200, description = "Active playlist display name", body = String),
        (status = 404, description = "Zone not found", body = ErrorBody)
    ),
    tag = "zones"
)]
async fn get_playlist_name(
    State(state): State<SharedState>,
    Path(idx): Path<usize>,
) -> impl IntoResponse {
    read_zone(&state, idx)
        .await
        .map(|z| Json(z.playlist_name.unwrap_or_default()))
        .ok_or(zone_not_found())
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct ZonePlaylistInfo {
    index: Option<usize>,
    name: Option<String>,
    total: Option<usize>,
    track_index: Option<usize>,
    track_count: Option<usize>,
}

/// Get active playlist summary info
///
/// Returns high-level details about the active playlist (name, current track index, total tracks).
#[utoipa::path(
    get,
    path = "/api/v1/zones/{zone_index}/playlist/info",
    params(
        ("zone_index" = usize, Path, description = "1-based index of the target zone")
    ),
    responses(
        (status = 200, description = "Active playlist summary", body = ZonePlaylistInfo),
        (status = 404, description = "Zone not found", body = ErrorBody)
    ),
    tag = "zones"
)]
async fn get_playlist_info(
    State(state): State<SharedState>,
    Path(idx): Path<usize>,
) -> impl IntoResponse {
    let zone = read_zone(&state, idx).await.ok_or(zone_not_found())?;
    Ok::<_, ApiError>(Json(ZonePlaylistInfo {
        index: zone.playlist_index,
        name: zone.playlist_name.clone(),
        total: zone.playlist_total,
        track_index: zone.playlist_track_index,
        track_count: zone.playlist_track_count,
    }))
}

/// Get playlist track count
///
/// Returns the number of tracks currently in the active playlist.
#[utoipa::path(
    get,
    path = "/api/v1/zones/{zone_index}/playlist/count",
    params(
        ("zone_index" = usize, Path, description = "1-based index of the target zone")
    ),
    responses(
        (status = 200, description = "Playlist track count", body = i32),
        (status = 404, description = "Zone not found", body = ErrorBody)
    ),
    tag = "zones"
)]
async fn get_playlist_count(
    State(state): State<SharedState>,
    Path(idx): Path<usize>,
) -> impl IntoResponse {
    read_zone(&state, idx)
        .await
        .map(|z| {
            #[allow(clippy::cast_possible_wrap)]
            let count = z.playlist_track_count.unwrap_or(0) as i32;
            Json(count)
        })
        .ok_or(zone_not_found())
}

/// Get zone clients index list
///
/// Returns a list of 1-based client indexes currently assigned to the specified zone.
#[utoipa::path(
    get,
    path = "/api/v1/zones/{zone_index}/clients",
    params(
        ("zone_index" = usize, Path, description = "1-based index of the target zone")
    ),
    responses(
        (status = 200, description = "List of assigned client indexes", body = Vec<usize>)
    ),
    tag = "zones"
)]
async fn get_clients(State(state): State<SharedState>, Path(idx): Path<usize>) -> Json<Vec<usize>> {
    let store = state.store.read().await;
    Json(
        store
            .clients
            .values()
            .filter(|c| c.zone_index == idx)
            .map(|c| {
                state
                    .config
                    .clients
                    .iter()
                    .find(|cc| cc.mac == c.mac)
                    .map_or(0, |cc| cc.index)
            })
            .collect(),
    )
}

// ── Presence ──────────────────────────────────────────────────

/// Get presence occupancy status
///
/// Returns true if presence/motion is currently active in the zone.
#[utoipa::path(
    get,
    path = "/api/v1/zones/{zone_index}/presence",
    params(
        ("zone_index" = usize, Path, description = "1-based index of the target zone")
    ),
    responses(
        (status = 200, description = "Current occupancy status", body = bool),
        (status = 404, description = "Zone not found", body = ErrorBody)
    ),
    tag = "zones"
)]
async fn get_presence(
    State(state): State<SharedState>,
    Path(idx): Path<usize>,
) -> impl IntoResponse {
    read_zone(&state, idx)
        .await
        .map(|z| Json(z.presence))
        .ok_or(zone_not_found())
}

/// Set presence occupancy status
///
/// Triggers presence/occupancy manually (true = presence active, false = presence cleared).
#[utoipa::path(
    put,
    path = "/api/v1/zones/{zone_index}/presence",
    params(
        ("zone_index" = usize, Path, description = "1-based index of the target zone")
    ),
    request_body = bool,
    responses(
        (status = 200, description = "Presence status updated successfully"),
        (status = 404, description = "Zone not found", body = ErrorBody)
    ),
    tag = "zones"
)]
async fn set_presence(
    State(state): State<SharedState>,
    Path(idx): Path<usize>,
    Json(v): Json<bool>,
) -> impl IntoResponse {
    send_cmd(&state, idx, ZoneCommand::SetPresence(v)).await
}

/// Get presence detection enable status
///
/// Returns whether presence detection is enabled for the specified zone.
#[utoipa::path(
    get,
    path = "/api/v1/zones/{zone_index}/presence/enable",
    params(
        ("zone_index" = usize, Path, description = "1-based index of the target zone")
    ),
    responses(
        (status = 200, description = "Presence sensor system status", body = bool),
        (status = 404, description = "Zone not found", body = ErrorBody)
    ),
    tag = "zones"
)]
async fn get_presence_enabled(
    State(state): State<SharedState>,
    Path(idx): Path<usize>,
) -> impl IntoResponse {
    read_zone(&state, idx)
        .await
        .map(|z| Json(z.presence_enabled))
        .ok_or(zone_not_found())
}

/// Set presence detection enable status
///
/// Enables or disables the presence/motion detection system for the specified zone.
#[utoipa::path(
    put,
    path = "/api/v1/zones/{zone_index}/presence/enable",
    params(
        ("zone_index" = usize, Path, description = "1-based index of the target zone")
    ),
    request_body = bool,
    responses(
        (status = 200, description = "Presence sensor status updated"),
        (status = 404, description = "Zone not found", body = ErrorBody)
    ),
    tag = "zones"
)]
async fn set_presence_enabled(
    State(state): State<SharedState>,
    Path(idx): Path<usize>,
    Json(v): Json<bool>,
) -> impl IntoResponse {
    send_cmd(&state, idx, ZoneCommand::SetPresenceEnabled(v)).await
}

/// Get presence timeout delay
///
/// Returns the auto-off delay in seconds after presence is cleared.
#[utoipa::path(
    get,
    path = "/api/v1/zones/{zone_index}/presence/timeout",
    params(
        ("zone_index" = usize, Path, description = "1-based index of the target zone")
    ),
    responses(
        (status = 200, description = "Auto-off timeout in seconds", body = u16),
        (status = 404, description = "Zone not found", body = ErrorBody)
    ),
    tag = "zones"
)]
async fn get_presence_timeout(
    State(state): State<SharedState>,
    Path(idx): Path<usize>,
) -> impl IntoResponse {
    read_zone(&state, idx)
        .await
        .map(|z| Json(z.auto_off_delay))
        .ok_or(zone_not_found())
}

/// Set presence timeout delay
///
/// Updates the auto-off timeout delay (in seconds) after presence is cleared.
#[utoipa::path(
    put,
    path = "/api/v1/zones/{zone_index}/presence/timeout",
    params(
        ("zone_index" = usize, Path, description = "1-based index of the target zone")
    ),
    request_body = u16,
    responses(
        (status = 200, description = "Presence timeout updated"),
        (status = 404, description = "Zone not found", body = ErrorBody)
    ),
    tag = "zones"
)]
async fn set_presence_timeout(
    State(state): State<SharedState>,
    Path(idx): Path<usize>,
    Json(v): Json<u16>,
) -> impl IntoResponse {
    send_cmd(&state, idx, ZoneCommand::SetAutoOffDelay(v)).await
}

/// Get presence timer active status
///
/// Returns whether the auto-off timer is currently running (countdown to turn off playback).
#[utoipa::path(
    get,
    path = "/api/v1/zones/{zone_index}/presence/timer",
    params(
        ("zone_index" = usize, Path, description = "1-based index of the target zone")
    ),
    responses(
        (status = 200, description = "True if auto-off countdown timer is active", body = bool),
        (status = 404, description = "Zone not found", body = ErrorBody)
    ),
    tag = "zones"
)]
async fn get_presence_timer(
    State(state): State<SharedState>,
    Path(idx): Path<usize>,
) -> impl IntoResponse {
    read_zone(&state, idx)
        .await
        .map(|z| Json(z.auto_off_active))
        .ok_or(zone_not_found())
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct ZoneTrackMetadata {
    title: String,
    artist: String,
    album: String,
    album_artist: Option<String>,
    genre: Option<String>,
    year: Option<u32>,
    track_number: Option<u32>,
    disc_number: Option<u32>,
    duration_ms: i64,
    position_ms: i64,
    seekable: bool,
    bitrate_kbps: Option<u32>,
    content_type: Option<String>,
    sample_rate: Option<u32>,
    source: String,
    cover_url: Option<String>,
    playlist_index: Option<usize>,
    playlist_name: Option<String>,
    playlist_total: Option<usize>,
    playlist_track_index: Option<usize>,
    playlist_track_count: Option<usize>,
    can_next: bool,
    can_prev: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seek_request_absolute() {
        let req: SeekPayload = serde_json::from_str(r#"{"position_ms":45000}"#).unwrap();
        assert_eq!(req.position_ms, Some(45000));
        assert_eq!(req.offset_ms, None);
    }

    #[test]
    fn seek_request_relative() {
        let req: SeekPayload = serde_json::from_str(r#"{"offset_ms":5000}"#).unwrap();
        assert_eq!(req.position_ms, None);
        assert_eq!(req.offset_ms, Some(5000));
    }

    #[test]
    fn seek_request_both_fields() {
        // Both fields set — struct allows it (validation is at handler level)
        let req: SeekPayload =
            serde_json::from_str(r#"{"position_ms":1000,"offset_ms":500}"#).unwrap();
        assert_eq!(req.position_ms, Some(1000));
        assert_eq!(req.offset_ms, Some(500));
    }

    #[test]
    fn playlist_payload_defaults_to_first_track() {
        let req: PlaylistPayload = serde_json::from_str(r#"{"id":2}"#).unwrap();
        assert_eq!(req.id, 2);
        assert_eq!(req.track, 0);
    }

    #[test]
    fn playlist_payload_accepts_start_track() {
        let req: PlaylistPayload = serde_json::from_str(r#"{"id":2,"track":7}"#).unwrap();
        assert_eq!(req.id, 2);
        assert_eq!(req.track, 7);
    }
}
