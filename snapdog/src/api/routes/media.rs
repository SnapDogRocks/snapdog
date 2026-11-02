// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! Media endpoints: /api/v1/media

use axum::extract::{Path, Query, State};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use crate::api::CACHE_CONTROL_1DAY;
use crate::api::SharedState;
use crate::api::error::{ApiError, ErrorBody};
use crate::config::{RADIO_PLAYLIST_ID, ResolvedPlaylist, subsonic_playlist_id};
use crate::subsonic::{PlaylistEntry, SubsonicClient};

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

/// Optional paging of a track list.
#[derive(Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub struct Paging {
    /// Maximum number of tracks. Without `limit` and `offset` every track is returned and no
    /// `X-Total-Count` header is sent.
    limit: Option<usize>,
    /// Number of tracks to skip (default 0).
    offset: Option<usize>,
}

/// The whole list, or with paging the same array cut, plus `X-Total-Count` (the full size).
fn paged(tracks: Vec<MediaTrackInfo>, paging: &Paging) -> axum::response::Response {
    if paging.limit.is_none() && paging.offset.is_none() {
        return Json(tracks).into_response();
    }
    let total = tracks.len();
    let offset = paging.offset.unwrap_or(0).min(total);
    let end = paging
        .limit
        .map_or(total, |l| offset.saturating_add(l).min(total));
    let page: Vec<MediaTrackInfo> = tracks.into_iter().take(end).skip(offset).collect();
    ([("X-Total-Count", total.to_string())], Json(page)).into_response()
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
        .route("/search", get(search))
        .route("/cover/{cover_id}", get(get_cover))
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

/// Get the Subsonic playlists from the cache shared with the zone players.
///
/// A failed fetch keeps the last good list instead of replacing it with an empty one, see
/// [`crate::subsonic::PlaylistCache`].
pub(super) async fn cached_playlists(state: &SharedState) -> Vec<PlaylistEntry> {
    let entries = state
        .playlist_cache
        .get(subsonic(state).ok().as_ref())
        .await;
    announce_changes(state, &entries);
    entries
}

/// Read the list from Subsonic now and store it, whatever the age of the cache.
async fn fetch_playlists(state: &SharedState) -> Vec<PlaylistEntry> {
    let entries = state
        .playlist_cache
        .fetch(subsonic(state).ok().as_ref())
        .await;
    announce_changes(state, &entries);
    entries
}

/// The request path and the refresh task share this, so the API and the event agree.
fn announce_changes(state: &SharedState, entries: &[PlaylistEntry]) {
    if state.playlist_cache.announce(entries) {
        notify_playlists_changed(state, entries.len());
    }
}

fn notify_playlists_changed(state: &SharedState, subsonic_count: usize) {
    crate::api::ws::broadcast_notification(
        &state.notifications,
        &crate::api::ws::Notification::PlaylistsChanged {
            total: state.config.unified_playlist_count(subsonic_count),
        },
    );
}

/// Read the playlist list every `[subsonic].refresh_secs` and send `playlists_changed` when it
/// differs from the last announced list. Runs until the task is aborted.
pub async fn run_playlist_refresh(state: SharedState) {
    let Some(secs) = state
        .config
        .subsonic
        .as_ref()
        .map(|c| c.refresh_secs)
        .filter(|s| *s > 0)
    else {
        return;
    };
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(secs));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    // The first tick fires at once and fills the cache, which needs no event.
    loop {
        interval.tick().await;
        fetch_playlists(&state).await;
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

/// Query of `GET /media/search`.
#[derive(Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub struct SearchQuery {
    /// Text to search for, at least two characters.
    q: String,
    /// Comma-separated kinds: `track`, `album`, `playlist`, `radio` (default all four).
    kind: Option<String>,
    /// Results per kind (default 20, at most 100).
    limit: Option<usize>,
    /// Results to skip per kind (default 0).
    offset: Option<usize>,
}

/// One search result. The identifier has a field per kind, there is no generic one.
#[derive(Serialize, utoipa::ToSchema)]
pub struct SearchItem {
    /// `track`, `album`, `playlist` or `radio`.
    kind: &'static str,
    title: String,
    /// Artist, or album for a track. Absent for a playlist.
    #[serde(skip_serializing_if = "Option::is_none")]
    subtitle: Option<String>,
    /// Seconds, 0 if unknown.
    duration: u64,
    /// Path of a cover image, see `GET /media/cover/{cover_id}`.
    cover_art: Option<String>,
    /// Subsonic track id (kind `track`). Plays with `POST /zones/{zone}/play/subsonic/{track_id}`.
    #[serde(skip_serializing_if = "Option::is_none")]
    track_id: Option<String>,
    /// Subsonic album id (kind `album`). Plays with `POST /zones/{zone}/play/subsonic/album/{album_id}`.
    #[serde(skip_serializing_if = "Option::is_none")]
    album_id: Option<String>,
    /// Stable reference (kinds `playlist` and `radio`, where `radio` is the list `snapdog:radio`).
    #[serde(skip_serializing_if = "Option::is_none")]
    playlist_id: Option<String>,
    /// Current unified index of the playlist (kinds `playlist` and `radio`).
    #[serde(skip_serializing_if = "Option::is_none")]
    index: Option<usize>,
    /// Position of the station in the radio list (kind `radio`), the `track` to play it with.
    #[serde(skip_serializing_if = "Option::is_none")]
    track: Option<usize>,
}

impl SearchItem {
    const fn new(kind: &'static str, title: String) -> Self {
        Self {
            kind,
            title,
            subtitle: None,
            duration: 0,
            cover_art: None,
            track_id: None,
            album_id: None,
            playlist_id: None,
            index: None,
            track: None,
        }
    }
}

/// Body of `GET /media/search`.
#[derive(Serialize, utoipa::ToSchema)]
pub struct SearchResponse {
    items: Vec<SearchItem>,
    /// Offset of the next page, `null` when no kind filled its page.
    next_offset: Option<usize>,
}

const SEARCH_KINDS: [&str; 4] = ["track", "album", "playlist", "radio"];
const SEARCH_DEFAULT_LIMIT: usize = 20;
const SEARCH_MAX_LIMIT: usize = 100;

/// Path of the cover proxy for a Subsonic cover id, percent-encoded.
fn cover_path(cover_id: &str) -> String {
    use std::fmt::Write as _;
    let mut path = String::from("/api/v1/media/cover/");
    for byte in cover_id.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            path.push(char::from(byte));
        } else {
            let _ = write!(path, "%{byte:02X}");
        }
    }
    path
}

/// Search the music library, the playlists and the radio stations
///
/// Tracks and albums come from the `search3` call of the Subsonic server, so what matches
/// follows the rules of that server (some match word prefixes, some any substring). Playlists
/// and radio stations are matched by the server on a case-insensitive substring of the name.
/// `limit` and `offset` apply to each kind. `search3` reports no total, so `next_offset` is set
/// when any kind filled its page.
#[utoipa::path(
    get,
    path = "/api/v1/media/search",
    params(SearchQuery),
    responses(
        (status = 200, description = "Search results", body = SearchResponse),
        (status = 400, description = "q shorter than two characters, or an unknown kind", body = ErrorBody),
        (status = 502, description = "Upstream request failed", body = ErrorBody)
    ),
    tag = "media"
)]
async fn search(
    State(state): State<SharedState>,
    Query(query): Query<SearchQuery>,
) -> Result<Json<SearchResponse>, ApiError> {
    let q = query.q.trim();
    if q.chars().count() < 2 {
        return Err(ApiError::BadRequest(
            "q needs at least two characters".into(),
        ));
    }
    let kinds: Vec<&str> = match query.kind.as_deref() {
        None => SEARCH_KINDS.to_vec(),
        Some(list) => {
            let mut kinds = Vec::new();
            for kind in list.split(',').map(str::trim) {
                let Some(known) = SEARCH_KINDS.iter().find(|k| **k == kind) else {
                    return Err(ApiError::BadRequest(format!("unknown kind `{kind}`")));
                };
                kinds.push(*known);
            }
            kinds
        }
    };
    let want = |kind: &str| kinds.contains(&kind);
    let limit = query
        .limit
        .unwrap_or(SEARCH_DEFAULT_LIMIT)
        .clamp(1, SEARCH_MAX_LIMIT);
    let offset = query.offset.unwrap_or(0);
    let needle = q.to_lowercase();

    let mut items = Vec::new();
    let mut page_full = false;
    let mut take = |found: Vec<SearchItem>| {
        page_full |= found.len() >= limit;
        items.extend(found);
    };
    if want("radio") {
        take(radio_items(&state, &needle, limit, offset));
    }
    if want("playlist") {
        take(playlist_items(&state, &needle, limit, offset).await);
    }
    if (want("track") || want("album")) && state.config.subsonic.is_some() {
        let ask = |wanted: bool| crate::subsonic::Page {
            count: if wanted { limit } else { 0 },
            offset,
        };
        let result = subsonic(&state)?
            .search(q, ask(want("track")), ask(want("album")))
            .await
            .map_err(|e| {
                tracing::error!(error = %e, "Subsonic search failed");
                ApiError::BadGateway("upstream request failed".into())
            })?;
        if want("album") {
            take(result.album.into_iter().map(album_item).collect());
        }
        if want("track") {
            take(result.song.into_iter().map(track_item).collect());
        }
    }
    Ok(Json(SearchResponse {
        items,
        next_offset: page_full.then_some(offset + limit),
    }))
}

fn radio_items(state: &SharedState, needle: &str, limit: usize, offset: usize) -> Vec<SearchItem> {
    state
        .config
        .radios
        .iter()
        .enumerate()
        .filter(|(_, r)| r.name.to_lowercase().contains(needle))
        .skip(offset)
        .take(limit)
        .map(|(i, r)| {
            let mut item = SearchItem::new("radio", r.name.clone());
            item.cover_art = Some(format!("{MEDIA_PATH}/0/tracks/{i}/cover"));
            item.playlist_id = Some(RADIO_PLAYLIST_ID.into());
            item.index = Some(0);
            item.track = Some(i);
            item
        })
        .collect()
}

async fn playlist_items(
    state: &SharedState,
    needle: &str,
    limit: usize,
    offset: usize,
) -> Vec<SearchItem> {
    let base = usize::from(state.config.has_radio_playlist());
    cached_playlists(state)
        .await
        .into_iter()
        .enumerate()
        .filter(|(_, p)| p.name.to_lowercase().contains(needle))
        .skip(offset)
        .take(limit)
        .map(|(sub_idx, p)| {
            let index = base + sub_idx;
            let mut item = SearchItem::new("playlist", p.name);
            item.duration = p.duration;
            item.cover_art = p
                .cover_art
                .as_ref()
                .map(|_| format!("{MEDIA_PATH}/{index}/cover"));
            item.playlist_id = Some(subsonic_playlist_id(&p.id));
            item.index = Some(index);
            item
        })
        .collect()
}

fn album_item(a: crate::subsonic::AlbumEntry) -> SearchItem {
    let mut item = SearchItem::new("album", a.name);
    item.subtitle = a.artist;
    item.duration = a.duration;
    item.cover_art = a.cover_art.as_deref().map(cover_path);
    item.album_id = Some(a.id);
    item
}

fn track_item(t: crate::subsonic::Track) -> SearchItem {
    let mut item = SearchItem::new("track", t.title);
    item.subtitle = t.album.or(t.artist);
    item.duration = t.duration;
    item.cover_art = t.cover_art.as_deref().map(cover_path);
    item.track_id = Some(t.id);
    item
}

/// Get a cover image by its Subsonic cover id
///
/// Proxies the cover of a search result, with the caching headers of the other cover routes.
/// A search result has no playlist index to build the playlist cover path from.
#[utoipa::path(
    get,
    path = "/api/v1/media/cover/{cover_id}",
    params(
        ("cover_id" = String, Path, description = "Subsonic cover art id, percent-encoded")
    ),
    responses(
        (status = 200, description = "Cover image", content_type = "image/*"),
        (status = 404, description = "Cover not found", body = ErrorBody),
        (status = 503, description = "Subsonic is not configured", body = ErrorBody)
    ),
    tag = "media"
)]
async fn get_cover(
    State(state): State<SharedState>,
    Path(cover_id): Path<String>,
) -> Result<([(axum::http::header::HeaderName, String); 2], Vec<u8>), ApiError> {
    let sub = subsonic(&state)?;
    let bytes = sub
        .get_cover_art(&cover_id)
        .await
        .map_err(|_| ApiError::NotFound("resource"))?;
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
        ("playlist_id" = String, Path, description = "Stable playlist reference, percent-encoded"),
        Paging
    ),
    responses(
        (status = 200, description = "List of tracks in the playlist", body = Vec<MediaTrackInfo>,
            headers(("X-Total-Count" = usize, description = "Full number of tracks, only when `limit` or `offset` is given"))),
        (status = 404, description = "Playlist not found", body = ErrorBody),
        (status = 502, description = "Upstream request failed", body = ErrorBody)
    ),
    tag = "media"
)]
async fn get_playlist_tracks_by_id(
    State(state): State<SharedState>,
    Path(playlist_id): Path<String>,
    Query(paging): Query<Paging>,
) -> Result<axum::response::Response, ApiError> {
    let index = playlist_index_by_id(&state, &playlist_id).await?;
    get_playlist_tracks(State(state), Path(index), Query(paging)).await
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
/// Returns the tracks of the specified playlist. With `limit` and/or `offset` the same array is
/// cut and the header `X-Total-Count` carries the full number of tracks. Without them the
/// response is the whole list and has no such header.
#[utoipa::path(
    get,
    path = "/api/v1/media/playlists/{playlist_index}/tracks",
    params(
        ("playlist_index" = usize, Path, description = "0-based unified playlist index"),
        Paging
    ),
    responses(
        (status = 200, description = "List of tracks in the playlist", body = Vec<MediaTrackInfo>,
            headers(("X-Total-Count" = usize, description = "Full number of tracks, only when `limit` or `offset` is given"))),
        (status = 404, description = "Playlist not found", body = ErrorBody),
        (status = 502, description = "Upstream request failed", body = ErrorBody)
    ),
    tag = "media"
)]
async fn get_playlist_tracks(
    State(state): State<SharedState>,
    Path(index): Path<usize>,
    Query(paging): Query<Paging>,
) -> Result<axum::response::Response, ApiError> {
    let Json(tracks) = tracks_json(State(state), Path(index)).await?;
    Ok(paged(tracks, &paging))
}

async fn tracks_json(
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

    #[test]
    fn cover_path_percent_encodes_everything_but_unreserved_characters() {
        assert_eq!(cover_path("al-1_x.y~z"), "/api/v1/media/cover/al-1_x.y~z");
        assert_eq!(
            cover_path("a/b c?é"),
            "/api/v1/media/cover/a%2Fb%20c%3F%C3%A9"
        );
    }
}
