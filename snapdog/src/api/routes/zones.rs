// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! Zone endpoints: /api/v1/zones

use axum::extract::{Path, State};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use crate::api::SharedState;
use crate::api::error::ApiError;
use crate::player::ZoneCommand;
use crate::state;

/// Embedded `SnapDog` icon PNG (1024×1024) used as placeholder when no cover art is available.
const PLACEHOLDER_COVER: &[u8] = include_bytes!("../../../../assets/snapdog-icon-placeholder.png");

/// Volume value: absolute (e.g. `75`) or relative (e.g. `"+5"`, `"-3"`).
#[derive(Debug, Deserialize)]
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

#[derive(Serialize)]
struct ZoneInfo {
    index: usize,
    name: String,
    icon: String,
    volume: i32,
    muted: bool,
    playback: String,
    source: String,
    shuffle: bool,
    repeat: bool,
    track_repeat: bool,
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
        .route(
            "/{zone_index}/track/repeat",
            get(get_track_repeat).put(set_track_repeat),
        )
        .route(
            "/{zone_index}/track/repeat/toggle",
            post(toggle_track_repeat),
        )
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

async fn get_count(State(state): State<SharedState>) -> Json<usize> {
    Json(state.config.zones.len())
}

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
                    repeat: zs.is_some_and(|s| s.repeat),
                    track_repeat: zs.is_some_and(|s| s.track_repeat),
                    presence: zs.is_some_and(|s| s.presence),
                    presence_enabled: zs.is_none_or(|s| s.presence_enabled),
                    presence_timer_active: zs.is_some_and(|s| s.auto_off_active),
                }
            })
            .collect(),
    )
}

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
        repeat: zs.as_ref().is_some_and(|s| s.repeat),
        track_repeat: zs.as_ref().is_some_and(|s| s.track_repeat),
        presence: zs.as_ref().is_some_and(|s| s.presence),
        presence_enabled: zs.as_ref().is_none_or(|s| s.presence_enabled),
        presence_timer_active: zs.as_ref().is_some_and(|s| s.auto_off_active),
    }))
}

// ── Volume ────────────────────────────────────────────────────

async fn get_volume(State(state): State<SharedState>, Path(idx): Path<usize>) -> impl IntoResponse {
    read_zone(&state, idx)
        .await
        .map(|z| Json(z.volume))
        .ok_or(zone_not_found())
}

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

async fn get_mute(State(state): State<SharedState>, Path(idx): Path<usize>) -> impl IntoResponse {
    read_zone(&state, idx)
        .await
        .map(|z| Json(z.muted))
        .ok_or(zone_not_found())
}

async fn set_mute(
    State(state): State<SharedState>,
    Path(idx): Path<usize>,
    Json(v): Json<bool>,
) -> impl IntoResponse {
    send_cmd(&state, idx, ZoneCommand::SetMute(v)).await
}

async fn toggle_mute(
    State(state): State<SharedState>,
    Path(idx): Path<usize>,
) -> impl IntoResponse {
    send_cmd(&state, idx, ZoneCommand::ToggleMute).await
}

// ── Playback control ──────────────────────────────────────────

async fn play(State(state): State<SharedState>, Path(idx): Path<usize>) -> impl IntoResponse {
    send_cmd(&state, idx, ZoneCommand::Play).await
}

async fn pause(State(state): State<SharedState>, Path(idx): Path<usize>) -> impl IntoResponse {
    send_cmd(&state, idx, ZoneCommand::Pause).await
}

async fn stop(State(state): State<SharedState>, Path(idx): Path<usize>) -> impl IntoResponse {
    send_cmd(&state, idx, ZoneCommand::Stop).await
}

async fn next_track(State(state): State<SharedState>, Path(idx): Path<usize>) -> impl IntoResponse {
    send_cmd(&state, idx, ZoneCommand::Next).await
}

async fn previous_track(
    State(state): State<SharedState>,
    Path(idx): Path<usize>,
) -> impl IntoResponse {
    send_cmd(&state, idx, ZoneCommand::Previous).await
}

// ── Playlist ──────────────────────────────────────────────────

async fn get_playlist(
    State(state): State<SharedState>,
    Path(idx): Path<usize>,
) -> impl IntoResponse {
    read_zone(&state, idx)
        .await
        .map(|z| Json(z.playlist_index.unwrap_or(0)))
        .ok_or(zone_not_found())
}

async fn set_playlist(
    State(state): State<SharedState>,
    Path(idx): Path<usize>,
    Json(v): Json<usize>,
) -> impl IntoResponse {
    send_cmd(&state, idx, ZoneCommand::SetPlaylist(v, 0)).await
}

async fn next_playlist(
    State(state): State<SharedState>,
    Path(idx): Path<usize>,
) -> impl IntoResponse {
    send_cmd(&state, idx, ZoneCommand::NextPlaylist).await
}
async fn previous_playlist(
    State(state): State<SharedState>,
    Path(idx): Path<usize>,
) -> impl IntoResponse {
    send_cmd(&state, idx, ZoneCommand::PreviousPlaylist).await
}

async fn get_shuffle(
    State(state): State<SharedState>,
    Path(idx): Path<usize>,
) -> impl IntoResponse {
    read_zone(&state, idx)
        .await
        .map(|z| Json(z.shuffle))
        .ok_or(zone_not_found())
}
async fn set_shuffle(
    State(state): State<SharedState>,
    Path(idx): Path<usize>,
    Json(v): Json<bool>,
) -> impl IntoResponse {
    send_cmd(&state, idx, ZoneCommand::SetShuffle(v)).await
}
async fn toggle_shuffle(
    State(state): State<SharedState>,
    Path(idx): Path<usize>,
) -> impl IntoResponse {
    send_cmd(&state, idx, ZoneCommand::ToggleShuffle).await
}

async fn get_repeat(State(state): State<SharedState>, Path(idx): Path<usize>) -> impl IntoResponse {
    read_zone(&state, idx)
        .await
        .map(|z| Json(z.repeat))
        .ok_or(zone_not_found())
}
async fn set_repeat(
    State(state): State<SharedState>,
    Path(idx): Path<usize>,
    Json(v): Json<bool>,
) -> impl IntoResponse {
    send_cmd(&state, idx, ZoneCommand::SetRepeat(v)).await
}
async fn toggle_repeat(
    State(state): State<SharedState>,
    Path(idx): Path<usize>,
) -> impl IntoResponse {
    send_cmd(&state, idx, ZoneCommand::ToggleRepeat).await
}

// ── Track info ────────────────────────────────────────────────

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
async fn get_track_metadata(
    State(state): State<SharedState>,
    Path(idx): Path<usize>,
) -> impl IntoResponse {
    let zone = read_zone(&state, idx).await.ok_or(zone_not_found())?;
    Ok::<_, ApiError>(Json(serde_json::json!({
        "title": zone.track.as_ref().map_or("", |t| &t.title),
        "artist": zone.track.as_ref().map_or("", |t| &t.artist),
        "album": zone.track.as_ref().map_or("", |t| &t.album),
        "album_artist": zone.track.as_ref().and_then(|t| t.album_artist.as_deref()),
        "genre": zone.track.as_ref().and_then(|t| t.genre.as_deref()),
        "year": zone.track.as_ref().and_then(|t| t.year),
        "track_number": zone.track.as_ref().and_then(|t| t.track_number),
        "disc_number": zone.track.as_ref().and_then(|t| t.disc_number),
        "duration_ms": zone.track.as_ref().map_or(0, |t| t.duration_ms),
        "position_ms": zone.track.as_ref().map_or(0, |t| t.position_ms),
        "seekable": zone.track.as_ref().is_some_and(|t| t.seekable),
        "bitrate_kbps": zone.track.as_ref().and_then(|t| t.bitrate_kbps),
        "content_type": zone.track.as_ref().and_then(|t| t.content_type.as_deref()),
        "sample_rate": zone.track.as_ref().and_then(|t| t.sample_rate),
        "source": zone.source.to_string(),
        "cover_url": zone.cover_url,
        "playlist_index": zone.playlist_index,
        "playlist_track_index": zone.playlist_track_index,
        "playlist_track_count": zone.playlist_track_count,
    })))
}
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
async fn get_track_title(
    State(state): State<SharedState>,
    Path(idx): Path<usize>,
) -> impl IntoResponse {
    read_zone(&state, idx)
        .await
        .map(|z| Json(z.track.map_or(String::new(), |t| t.title)))
        .ok_or(zone_not_found())
}
async fn get_track_artist(
    State(state): State<SharedState>,
    Path(idx): Path<usize>,
) -> impl IntoResponse {
    read_zone(&state, idx)
        .await
        .map(|z| Json(z.track.map_or(String::new(), |t| t.artist)))
        .ok_or(zone_not_found())
}
async fn get_track_album(
    State(state): State<SharedState>,
    Path(idx): Path<usize>,
) -> impl IntoResponse {
    read_zone(&state, idx)
        .await
        .map(|z| Json(z.track.map_or(String::new(), |t| t.album)))
        .ok_or(zone_not_found())
}
async fn get_track_duration(
    State(state): State<SharedState>,
    Path(idx): Path<usize>,
) -> impl IntoResponse {
    read_zone(&state, idx)
        .await
        .map(|z| Json(z.track.map_or(0i64, |t| t.duration_ms)))
        .ok_or(zone_not_found())
}
async fn get_track_position(
    State(state): State<SharedState>,
    Path(idx): Path<usize>,
) -> impl IntoResponse {
    read_zone(&state, idx)
        .await
        .map(|z| Json(z.track.map_or(0i64, |t| t.position_ms)))
        .ok_or(zone_not_found())
}
async fn seek_position(
    State(state): State<SharedState>,
    Path(idx): Path<usize>,
    Json(body): Json<SeekRequest>,
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

#[derive(serde::Deserialize)]
struct SeekRequest {
    position_ms: Option<i64>,
    offset_ms: Option<i64>,
}
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
async fn seek_progress(
    State(state): State<SharedState>,
    Path(idx): Path<usize>,
    Json(v): Json<f64>,
) -> impl IntoResponse {
    send_cmd(&state, idx, ZoneCommand::SeekProgress(v)).await
}
async fn get_track_playing(
    State(state): State<SharedState>,
    Path(idx): Path<usize>,
) -> impl IntoResponse {
    read_zone(&state, idx)
        .await
        .map(|z| Json(z.playback == state::PlaybackState::Playing))
        .ok_or(zone_not_found())
}
async fn get_track_repeat(
    State(state): State<SharedState>,
    Path(idx): Path<usize>,
) -> impl IntoResponse {
    read_zone(&state, idx)
        .await
        .map(|z| Json(z.track_repeat))
        .ok_or(zone_not_found())
}
async fn set_track_repeat(
    State(state): State<SharedState>,
    Path(idx): Path<usize>,
    Json(v): Json<bool>,
) -> impl IntoResponse {
    send_cmd(&state, idx, ZoneCommand::SetTrackRepeat(v)).await
}
async fn toggle_track_repeat(
    State(state): State<SharedState>,
    Path(idx): Path<usize>,
) -> impl IntoResponse {
    send_cmd(&state, idx, ZoneCommand::ToggleTrackRepeat).await
}

// ── Play specific content ─────────────────────────────────────

async fn play_track(
    State(state): State<SharedState>,
    Path(idx): Path<usize>,
    Json(v): Json<i32>,
) -> impl IntoResponse {
    send_cmd(&state, idx, ZoneCommand::SetTrack(v as usize)).await
}
async fn play_url(
    State(state): State<SharedState>,
    Path(idx): Path<usize>,
    Json(v): Json<String>,
) -> impl IntoResponse {
    require_http_url(&v)?;
    send_cmd(&state, idx, ZoneCommand::PlayUrl(v)).await
}

#[derive(Deserialize)]
struct PlayPlaylistRequest {
    id: usize,
    #[serde(default)]
    track: usize,
}

async fn play_subsonic_playlist(
    State(state): State<SharedState>,
    Path(idx): Path<usize>,
    Json(v): Json<PlayPlaylistRequest>,
) -> impl IntoResponse {
    // Single command: SetPlaylist with playlist index and starting track
    let _ = send_cmd(&state, idx, ZoneCommand::SetPlaylist(v.id, v.track)).await;
    Ok::<_, ApiError>(())
}
async fn play_playlist_track(
    State(state): State<SharedState>,
    Path((zone, _playlist)): Path<(usize, usize)>,
    Json(v): Json<i32>,
) -> impl IntoResponse {
    send_cmd(&state, zone, ZoneCommand::SetTrack(v as usize)).await
}

async fn play_subsonic_track(
    State(state): State<SharedState>,
    Path((idx, track_id)): Path<(usize, String)>,
) -> impl IntoResponse {
    send_cmd(&state, idx, ZoneCommand::PlaySubsonicTrack(track_id)).await
}

// ── Zone info ─────────────────────────────────────────────────

async fn get_name(State(state): State<SharedState>, Path(idx): Path<usize>) -> impl IntoResponse {
    read_zone(&state, idx)
        .await
        .map(|z| Json(z.name))
        .ok_or(zone_not_found())
}
async fn get_icon(State(state): State<SharedState>, Path(idx): Path<usize>) -> impl IntoResponse {
    read_zone(&state, idx)
        .await
        .map(|z| Json(z.icon))
        .ok_or(zone_not_found())
}
async fn get_playback(
    State(state): State<SharedState>,
    Path(idx): Path<usize>,
) -> impl IntoResponse {
    read_zone(&state, idx)
        .await
        .map(|z| Json(z.playback.to_string()))
        .ok_or(zone_not_found())
}
async fn get_playlist_name(
    State(state): State<SharedState>,
    Path(idx): Path<usize>,
) -> impl IntoResponse {
    read_zone(&state, idx)
        .await
        .map(|z| Json(z.playlist_name.unwrap_or_default()))
        .ok_or(zone_not_found())
}
async fn get_playlist_info(
    State(state): State<SharedState>,
    Path(idx): Path<usize>,
) -> impl IntoResponse {
    let zone = read_zone(&state, idx).await.ok_or(zone_not_found())?;
    Ok::<_, ApiError>(Json(serde_json::json!({
        "index": zone.playlist_index,
        "name": zone.playlist_name,
        "track_index": zone.playlist_track_index,
        "track_count": zone.playlist_track_count,
    })))
}
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

async fn get_presence(
    State(state): State<SharedState>,
    Path(idx): Path<usize>,
) -> impl IntoResponse {
    read_zone(&state, idx)
        .await
        .map(|z| Json(z.presence))
        .ok_or(zone_not_found())
}

async fn set_presence(
    State(state): State<SharedState>,
    Path(idx): Path<usize>,
    Json(v): Json<bool>,
) -> impl IntoResponse {
    send_cmd(&state, idx, ZoneCommand::SetPresence(v)).await
}

async fn get_presence_enabled(
    State(state): State<SharedState>,
    Path(idx): Path<usize>,
) -> impl IntoResponse {
    read_zone(&state, idx)
        .await
        .map(|z| Json(z.presence_enabled))
        .ok_or(zone_not_found())
}

async fn set_presence_enabled(
    State(state): State<SharedState>,
    Path(idx): Path<usize>,
    Json(v): Json<bool>,
) -> impl IntoResponse {
    send_cmd(&state, idx, ZoneCommand::SetPresenceEnabled(v)).await
}

async fn get_presence_timeout(
    State(state): State<SharedState>,
    Path(idx): Path<usize>,
) -> impl IntoResponse {
    read_zone(&state, idx)
        .await
        .map(|z| Json(z.auto_off_delay))
        .ok_or(zone_not_found())
}

async fn set_presence_timeout(
    State(state): State<SharedState>,
    Path(idx): Path<usize>,
    Json(v): Json<u16>,
) -> impl IntoResponse {
    send_cmd(&state, idx, ZoneCommand::SetAutoOffDelay(v)).await
}

async fn get_presence_timer(
    State(state): State<SharedState>,
    Path(idx): Path<usize>,
) -> impl IntoResponse {
    read_zone(&state, idx)
        .await
        .map(|z| Json(z.auto_off_active))
        .ok_or(zone_not_found())
}
