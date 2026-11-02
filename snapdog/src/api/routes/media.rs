// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! Media endpoints: /api/v1/media

use axum::extract::{Path, State};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use serde::Serialize;

use crate::api::CACHE_CONTROL_1DAY;
use crate::api::SharedState;
use crate::api::error::{ApiError, ErrorBody};
use crate::config::{RADIO_PLAYLIST_ID, ResolvedPlaylist, subsonic_playlist_id};
use crate::subsonic::{PlaylistEntry, SubsonicClient};

const PLAYLIST_CACHE_TTL: std::time::Duration = std::time::Duration::from_mins(1);
/// Wait before asking a failing Subsonic server again.
const PLAYLIST_RETRY: std::time::Duration = std::time::Duration::from_secs(10);

#[derive(Serialize, utoipa::ToSchema)]
pub struct PlaylistInfo {
    /// Unified playlist index.
    ///
    /// Deprecated. Use `index`. Removed with SnapDog 1.0, which breaks compatibility of `/api/v1` for clients that still read it.
    #[schema(deprecated)]
    #[allow(clippy::doc_markdown)]
    id: usize,
    /// Unified playlist index (0-based). Moves when the list changes, see `playlist_id`.
    index: usize,
    /// Stable reference: `snapdog:radio` or `subsonic:<subsonic id>`.
    playlist_id: String,
    name: String,
    song_count: u32,
    duration: u64,
    cover_art: Option<String>,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct PlaylistDetails {
    /// Unified playlist index.
    ///
    /// Deprecated. Use `index`. Removed with SnapDog 1.0, which breaks compatibility of `/api/v1` for clients that still read it.
    #[schema(deprecated)]
    #[allow(clippy::doc_markdown)]
    id: usize,
    /// Unified playlist index (0-based). Moves when the list changes, see `playlist_id`.
    index: usize,
    /// Stable reference: `snapdog:radio` or `subsonic:<subsonic id>`.
    playlist_id: String,
    name: String,
    tracks: usize,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct MediaTrackInfo {
    id: String,
    title: String,
    artist: String,
    album: String,
    duration: u64,
    track: usize,
    cover_art: Option<String>,
}

pub fn router(state: SharedState) -> Router {
    Router::new()
        .route("/playlists", get(get_playlists))
        .route("/playlists/by-id/{playlist_id}", get(get_playlist_by_id))
        .route(
            "/playlists/by-id/{playlist_id}/tracks",
            get(get_playlist_tracks_by_id),
        )
        .route("/playlists/{playlist_index}", get(get_playlist))
        .route("/playlists/{playlist_index}/cover", get(get_playlist_cover))
        .route(
            "/playlists/{playlist_index}/tracks",
            get(get_playlist_tracks),
        )
        .route(
            "/playlists/{playlist_index}/tracks/{track_index}",
            get(get_playlist_track),
        )
        .route(
            "/playlists/{playlist_index}/tracks/{track_index}/cover",
            get(get_track_cover_art),
        )
        .with_state(state)
}

fn subsonic(state: &SharedState) -> Result<SubsonicClient, ApiError> {
    state
        .config
        .subsonic
        .as_ref()
        .map(SubsonicClient::new)
        .ok_or(ApiError::ServiceUnavailable("subsonic"))
}

/// Get Subsonic playlists with 60s cache.
///
/// A failed fetch keeps the last good list instead of replacing it with an empty one, so a
/// Subsonic outage does not make every playlist and the indexes behind them vanish. The next
/// attempt follows after [`PLAYLIST_RETRY`].
pub(super) async fn cached_playlists(state: &SharedState) -> Vec<PlaylistEntry> {
    // Check cache
    {
        let cache = state.playlist_cache.read().await;
        if let Some((ts, ref entries)) = *cache
            && ts.elapsed() < PLAYLIST_CACHE_TTL
        {
            return entries.clone();
        }
    }
    // Fetch and cache
    let fetched = match subsonic(state).ok() {
        Some(sub) => sub.get_playlists().await.map(Some),
        None => Ok(None),
    };
    let mut cache = state.playlist_cache.write().await;
    let (stamp, entries) = refreshed_cache(cache.take(), fetched, std::time::Instant::now());
    *cache = Some((stamp, entries.clone()));
    entries
}

/// Next cache content after a fetch. `Ok(None)` means Subsonic is not configured.
fn refreshed_cache(
    previous: Option<(std::time::Instant, Vec<PlaylistEntry>)>,
    fetched: anyhow::Result<Option<Vec<PlaylistEntry>>>,
    now: std::time::Instant,
) -> (std::time::Instant, Vec<PlaylistEntry>) {
    match fetched {
        Ok(entries) => (now, entries.unwrap_or_default()),
        Err(e) => {
            tracing::warn!("Subsonic playlists unavailable, keeping the last list: {e:#}");
            // Back-dated so that the entry expires after PLAYLIST_RETRY, not after the full TTL.
            let stamp = PLAYLIST_CACHE_TTL
                .checked_sub(PLAYLIST_RETRY)
                .and_then(|back| now.checked_sub(back))
                .unwrap_or(now);
            (
                stamp,
                previous.map(|(_, entries)| entries).unwrap_or_default(),
            )
        }
    }
}

const MEDIA_PATH: &str = "/api/v1/media/playlists";

/// Get all media playlists
///
/// Returns a list of all playlists available on the system, including local Radio stations and Subsonic playlists.
#[utoipa::path(
    get,
    path = "/api/v1/media/playlists",
    responses(
        (status = 200, description = "List of all media playlists", body = Vec<PlaylistInfo>)
    ),
    tag = "media"
)]
async fn get_playlists(State(state): State<SharedState>) -> impl IntoResponse {
    let mut result: Vec<PlaylistInfo> = Vec::new();
    let mut idx: usize = 0;

    // Playlist 0: Radio stations (from config)
    if !state.config.radios.is_empty() {
        result.push(PlaylistInfo {
            id: idx,
            index: idx,
            playlist_id: RADIO_PLAYLIST_ID.into(),
            name: "Radio".into(),
            song_count: state.config.radios.len() as u32,
            duration: 0,
            cover_art: Some("/assets/radio-cover.png".into()),
        });
        idx += 1;
    }

    // Playlist 1+: Subsonic playlists
    for p in cached_playlists(&state).await {
        let cover_art = p
            .cover_art
            .as_ref()
            .map(|_| format!("{MEDIA_PATH}/{idx}/cover"));
        result.push(PlaylistInfo {
            id: idx,
            index: idx,
            playlist_id: subsonic_playlist_id(&p.id),
            name: p.name,
            song_count: p.song_count,
            duration: p.duration,
            cover_art,
        });
        idx += 1;
    }

    Ok::<_, ApiError>(Json(result))
}

/// Resolve a unified playlist index using the shared config logic.
async fn resolve_playlist(state: &SharedState, index: usize) -> Result<ResolvedPlaylist, ApiError> {
    let playlists = cached_playlists(state).await;
    state
        .config
        .resolve_playlist_index(index, playlists.len())
        .ok_or(ApiError::NotFound("resource"))
}

/// Resolve index and return the Subsonic playlist ID (or `NOT_FOUND` for radio/out-of-range).
async fn resolve_subsonic_id(state: &SharedState, index: usize) -> Result<String, ApiError> {
    let playlists = cached_playlists(state).await;
    match state.config.resolve_playlist_index(index, playlists.len()) {
        Some(ResolvedPlaylist::Subsonic(sub_idx)) => playlists
            .get(sub_idx)
            .map(|p| p.id.clone())
            .ok_or(ApiError::NotFound("resource")),
        _ => Err(ApiError::NotFound("resource")),
    }
}

/// Resolve a `playlist_id` to its current unified index (404 for an unknown id).
pub(super) async fn playlist_index_by_id(
    state: &SharedState,
    playlist_id: &str,
) -> Result<usize, ApiError> {
    let playlists = cached_playlists(state).await;
    let ids: Vec<&str> = playlists.iter().map(|p| p.id.as_str()).collect();
    state
        .config
        .playlist_index_by_id(playlist_id, &ids)
        .ok_or(ApiError::NotFound("resource"))
}

/// Get playlist details by `playlist_id`
///
/// Same body as the index path. This path takes the `playlist_id` (`snapdog:radio`,
/// `subsonic:<id>`, percent-encoded); the path without `by-id` takes the unified index.
#[utoipa::path(
    get,
    path = "/api/v1/media/playlists/by-id/{playlist_id}",
    params(
        ("playlist_id" = String, Path, description = "Stable playlist reference, percent-encoded")
    ),
    responses(
        (status = 200, description = "Playlist details", body = PlaylistDetails),
        (status = 404, description = "Playlist not found", body = ErrorBody),
        (status = 502, description = "Upstream request failed", body = ErrorBody)
    ),
    tag = "media"
)]
async fn get_playlist_by_id(
    State(state): State<SharedState>,
    Path(playlist_id): Path<String>,
) -> Result<Json<PlaylistDetails>, ApiError> {
    let index = playlist_index_by_id(&state, &playlist_id).await?;
    get_playlist(State(state), Path(index)).await
}

/// Get playlist tracks by `playlist_id`
///
/// Same body as the index path. This path takes the `playlist_id`, the path without `by-id`
/// takes the unified index.
#[utoipa::path(
    get,
    path = "/api/v1/media/playlists/by-id/{playlist_id}/tracks",
    params(
        ("playlist_id" = String, Path, description = "Stable playlist reference, percent-encoded")
    ),
    responses(
        (status = 200, description = "List of tracks in the playlist", body = Vec<MediaTrackInfo>),
        (status = 404, description = "Playlist not found", body = ErrorBody),
        (status = 502, description = "Upstream request failed", body = ErrorBody)
    ),
    tag = "media"
)]
async fn get_playlist_tracks_by_id(
    State(state): State<SharedState>,
    Path(playlist_id): Path<String>,
) -> Result<Json<Vec<MediaTrackInfo>>, ApiError> {
    let index = playlist_index_by_id(&state, &playlist_id).await?;
    get_playlist_tracks(State(state), Path(index)).await
}

/// Get playlist details
///
/// Returns details (name, track count) of a unified playlist by its index.
#[utoipa::path(
    get,
    path = "/api/v1/media/playlists/{playlist_index}",
    params(
        ("playlist_index" = usize, Path, description = "0-based unified playlist index")
    ),
    responses(
        (status = 200, description = "Playlist details", body = PlaylistDetails),
        (status = 404, description = "Playlist not found", body = ErrorBody),
        (status = 502, description = "Upstream request failed", body = ErrorBody)
    ),
    tag = "media"
)]
async fn get_playlist(
    State(state): State<SharedState>,
    Path(index): Path<usize>,
) -> Result<Json<PlaylistDetails>, ApiError> {
    match resolve_playlist(&state, index).await? {
        ResolvedPlaylist::Radio => Ok(Json(PlaylistDetails {
            id: index,
            index,
            playlist_id: RADIO_PLAYLIST_ID.into(),
            name: "Radio".into(),
            tracks: state.config.radios.len(),
        })),
        ResolvedPlaylist::Subsonic(_) => {
            let id = resolve_subsonic_id(&state, index).await?;
            let sub = subsonic(&state)?;
            match sub.get_playlist(&id).await {
                Ok(playlist) => Ok(Json(PlaylistDetails {
                    id: index,
                    index,
                    playlist_id: subsonic_playlist_id(&playlist.id),
                    name: playlist.name,
                    tracks: playlist.entry.len(),
                })),
                Err(e) => {
                    tracing::error!(error = %e, "Failed to fetch playlist");
                    Err(ApiError::BadGateway("upstream request failed".into()))
                }
            }
        }
    }
}

/// Get playlist cover image
///
/// Returns the cover art image binary for the specified playlist.
#[utoipa::path(
    get,
    path = "/api/v1/media/playlists/{playlist_index}/cover",
    params(
        ("playlist_index" = usize, Path, description = "0-based unified playlist index")
    ),
    responses(
        (status = 200, description = "Playlist cover image", content_type = "image/*"),
        (status = 404, description = "Playlist or cover not found", body = ErrorBody)
    ),
    tag = "media"
)]
async fn get_playlist_cover(
    State(state): State<SharedState>,
    Path(index): Path<usize>,
) -> impl IntoResponse {
    match resolve_playlist(&state, index).await? {
        ResolvedPlaylist::Radio => Ok((
            [
                (axum::http::header::CONTENT_TYPE, "image/png".to_string()),
                (
                    axum::http::header::CACHE_CONTROL,
                    CACHE_CONTROL_1DAY.to_string(),
                ),
            ],
            include_bytes!("../../../../assets/radio-cover.png").to_vec(),
        )),
        ResolvedPlaylist::Subsonic(_) => {
            let id = resolve_subsonic_id(&state, index).await?;
            let sub = subsonic(&state)?;
            let playlists = cached_playlists(&state).await;
            let cover_id = playlists
                .iter()
                .find(|p| p.id == id)
                .and_then(|p| p.cover_art.clone())
                .ok_or(ApiError::NotFound("resource"))?;
            sub.get_cover_art(&cover_id).await.map_or_else(
                |_| Err(ApiError::NotFound("resource")),
                |bytes| {
                    let mime = crate::state::cover::detect_mime(&bytes);
                    Ok((
                        [
                            (axum::http::header::CONTENT_TYPE, mime.to_string()),
                            (
                                axum::http::header::CACHE_CONTROL,
                                CACHE_CONTROL_1DAY.to_string(),
                            ),
                        ],
                        bytes,
                    ))
                },
            )
        }
    }
}

/// Get playlist tracks
///
/// Returns a list of all tracks in the specified playlist.
#[utoipa::path(
    get,
    path = "/api/v1/media/playlists/{playlist_index}/tracks",
    params(
        ("playlist_index" = usize, Path, description = "0-based unified playlist index")
    ),
    responses(
        (status = 200, description = "List of tracks in the playlist", body = Vec<MediaTrackInfo>),
        (status = 404, description = "Playlist not found", body = ErrorBody),
        (status = 502, description = "Upstream request failed", body = ErrorBody)
    ),
    tag = "media"
)]
async fn get_playlist_tracks(
    State(state): State<SharedState>,
    Path(index): Path<usize>,
) -> Result<Json<Vec<MediaTrackInfo>>, ApiError> {
    match resolve_playlist(&state, index).await? {
        ResolvedPlaylist::Radio => Ok(Json(
            state
                .config
                .radios
                .iter()
                .enumerate()
                .map(|(i, r)| {
                    let cover = format!("{MEDIA_PATH}/{index}/tracks/{i}/cover");
                    MediaTrackInfo {
                        id: format!("radio_{i}"),
                        title: r.name.clone(),
                        artist: "Radio".into(),
                        album: String::new(),
                        duration: 0,
                        track: i + 1,
                        cover_art: Some(cover),
                    }
                })
                .collect::<Vec<_>>(),
        )),
        ResolvedPlaylist::Subsonic(_) => {
            let id = resolve_subsonic_id(&state, index).await?;
            let sub = subsonic(&state)?;
            match sub.get_playlist(&id).await {
                Ok(playlist) => Ok(Json(
                    playlist
                        .entry
                        .iter()
                        .enumerate()
                        .map(|(i, t)| {
                            let cover = t
                                .cover_art
                                .as_ref()
                                .map(|_| format!("{MEDIA_PATH}/{index}/tracks/{i}/cover"));
                            MediaTrackInfo {
                                id: t.id.clone(),
                                title: t.title.clone(),
                                artist: t.artist.clone().unwrap_or_default(),
                                album: t.album.clone().unwrap_or_default(),
                                duration: t.duration,
                                track: t.track.map_or(i + 1, |v| v as usize),
                                cover_art: cover,
                            }
                        })
                        .collect::<Vec<_>>(),
                )),
                Err(e) => {
                    tracing::error!(error = %e, "Failed to fetch tracks");
                    Err(ApiError::BadGateway("upstream request failed".into()))
                }
            }
        }
    }
}

/// Get track details
///
/// Returns details of a specific track within a playlist.
#[utoipa::path(
    get,
    path = "/api/v1/media/playlists/{playlist_index}/tracks/{track_index}",
    params(
        ("playlist_index" = usize, Path, description = "0-based unified playlist index"),
        ("track_index" = usize, Path, description = "0-based track index within the playlist")
    ),
    responses(
        (status = 200, description = "Track details", body = MediaTrackInfo),
        (status = 404, description = "Playlist or track not found", body = ErrorBody),
        (status = 502, description = "Upstream request failed", body = ErrorBody)
    ),
    tag = "media"
)]
async fn get_playlist_track(
    State(state): State<SharedState>,
    Path((index, track_index)): Path<(usize, usize)>,
) -> impl IntoResponse {
    match resolve_playlist(&state, index).await? {
        ResolvedPlaylist::Radio => state
            .config
            .radios
            .get(track_index)
            .map(|r| {
                let cover = format!("{MEDIA_PATH}/{index}/tracks/{track_index}/cover");
                Json(MediaTrackInfo {
                    id: format!("radio_{track_index}"),
                    title: r.name.clone(),
                    artist: "Radio".into(),
                    album: String::new(),
                    duration: 0,
                    track: track_index + 1,
                    cover_art: Some(cover),
                })
            })
            .ok_or(ApiError::NotFound("resource")),
        ResolvedPlaylist::Subsonic(_) => {
            let id = resolve_subsonic_id(&state, index).await?;
            let sub = subsonic(&state)?;
            match sub.get_playlist(&id).await {
                Ok(playlist) => playlist.entry.get(track_index).map_or(
                    Err(ApiError::NotFound("resource")),
                    |t| {
                        let cover = t
                            .cover_art
                            .as_ref()
                            .map(|_| format!("{MEDIA_PATH}/{index}/tracks/{track_index}/cover"));
                        Ok(Json(MediaTrackInfo {
                            id: t.id.clone(),
                            title: t.title.clone(),
                            artist: t.artist.clone().unwrap_or_default(),
                            album: t.album.clone().unwrap_or_default(),
                            duration: t.duration,
                            track: t.track.map_or(track_index + 1, |v| v as usize),
                            cover_art: cover,
                        }))
                    },
                ),
                Err(_) => Err(ApiError::BadGateway("upstream request failed".into())),
            }
        }
    }
}

/// Get track cover image
///
/// Returns the cover art image binary for a specific track.
#[utoipa::path(
    get,
    path = "/api/v1/media/playlists/{playlist_index}/tracks/{track_index}/cover",
    params(
        ("playlist_index" = usize, Path, description = "0-based unified playlist index"),
        ("track_index" = usize, Path, description = "0-based track index within the playlist")
    ),
    responses(
        (status = 200, description = "Track cover image", content_type = "image/*"),
        (status = 404, description = "Track or cover not found", body = ErrorBody)
    ),
    tag = "media"
)]
async fn get_track_cover_art(
    State(state): State<SharedState>,
    Path((index, track_index)): Path<(usize, usize)>,
) -> Result<([(axum::http::header::HeaderName, String); 2], Vec<u8>), ApiError> {
    match resolve_playlist(&state, index).await? {
        ResolvedPlaylist::Radio => {
            let radio = state
                .config
                .radios
                .get(track_index)
                .ok_or(ApiError::NotFound("resource"))?;

            // Try cache first (keyed by station URL)
            if let Some(entry) = state.covers.read().await.get_static(&radio.url) {
                return Ok((
                    [
                        (axum::http::header::CONTENT_TYPE, entry.mime.clone()),
                        (
                            axum::http::header::CACHE_CONTROL,
                            CACHE_CONTROL_1DAY.to_string(),
                        ),
                    ],
                    entry.bytes.clone(),
                ));
            }

            // Fetch cover with favicon fallback
            let (bytes, mime) = crate::state::cover::fetch_cover_with_favicon_fallback(
                radio.cover.as_deref(),
                &radio.url,
            )
            .await
            .ok_or(ApiError::NotFound("resource"))?;

            // Store in cache
            state
                .covers
                .write()
                .await
                .set_static(&radio.url, bytes.clone(), mime.clone());

            Ok((
                [
                    (axum::http::header::CONTENT_TYPE, mime),
                    (
                        axum::http::header::CACHE_CONTROL,
                        CACHE_CONTROL_1DAY.to_string(),
                    ),
                ],
                bytes,
            ))
        }
        ResolvedPlaylist::Subsonic(_) => {
            let id = resolve_subsonic_id(&state, index).await?;
            let sub = subsonic(&state)?;
            let playlist = sub
                .get_playlist(&id)
                .await
                .map_err(|e| ApiError::BadGateway(e.to_string()))?;
            let track = playlist
                .entry
                .get(track_index)
                .ok_or(ApiError::NotFound("resource"))?;
            let cover_id = track
                .cover_art
                .as_ref()
                .ok_or(ApiError::NotFound("resource"))?;
            let bytes = sub
                .get_cover_art(cover_id)
                .await
                .map_err(|e| ApiError::BadGateway(e.to_string()))?;
            let mime = crate::state::cover::detect_mime(&bytes);
            Ok((
                [
                    (axum::http::header::CONTENT_TYPE, mime.to_string()),
                    (
                        axum::http::header::CACHE_CONTROL,
                        CACHE_CONTROL_1DAY.to_string(),
                    ),
                ],
                bytes,
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    fn entry(id: &str) -> PlaylistEntry {
        PlaylistEntry {
            id: id.into(),
            name: id.into(),
            song_count: 0,
            duration: 0,
            cover_art: None,
        }
    }

    #[test]
    fn successful_fetch_replaces_the_list() {
        let now = Instant::now();
        let previous = Some((now, vec![entry("old")]));
        let (stamp, entries) = refreshed_cache(previous, Ok(Some(vec![entry("new")])), now);
        assert_eq!(stamp, now);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].id, "new");
    }

    #[test]
    fn unconfigured_subsonic_yields_an_empty_list() {
        let now = Instant::now();
        let (_, entries) = refreshed_cache(Some((now, vec![entry("old")])), Ok(None), now);
        assert!(entries.is_empty());
    }

    #[test]
    fn failed_fetch_keeps_the_last_good_list() {
        let now = Instant::now();
        let previous = Some((now, vec![entry("a"), entry("b")]));
        let (_, entries) = refreshed_cache(previous, Err(anyhow::anyhow!("down")), now);
        assert_eq!(
            entries.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(),
            ["a", "b"]
        );
    }

    #[test]
    fn failed_first_fetch_yields_an_empty_list() {
        let (_, entries) = refreshed_cache(None, Err(anyhow::anyhow!("down")), Instant::now());
        assert!(entries.is_empty());
    }

    #[test]
    fn failed_fetch_expires_after_the_retry_delay() {
        let now = Instant::now();
        let (stamp, _) = refreshed_cache(None, Err(anyhow::anyhow!("down")), now);
        let age = now.duration_since(stamp);
        assert_eq!(age, PLAYLIST_CACHE_TTL.checked_sub(PLAYLIST_RETRY).unwrap());
    }
}
