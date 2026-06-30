// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! `ZonePlayer` helper functions for track navigation and completion.

use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use super::commands::ActiveSource;
use super::context::{NotifySender, stop_decode, update_and_notify};
use super::nav;
use crate::audio;
use crate::config::AppConfig;
use crate::state::{self, PlaybackState, SourceType, TrackInfo, cover::SharedCoverCache};
use crate::subsonic::SubsonicClient;

/// Mutable decode state passed to navigation helpers.
pub struct DecodeState<'a> {
    pub current_decode: &'a mut Option<JoinHandle<()>>,
    pub current_cover: &'a mut Option<JoinHandle<()>>,
    pub decode_rx: &'a mut Option<mpsc::Receiver<audio::PcmMessage>>,
    pub source: &'a mut ActiveSource,
}

/// Read-only playback context shared by all navigation helpers.
pub struct PlaybackCtx<'a> {
    pub config: &'a AppConfig,
    pub subsonic: &'a Option<SubsonicClient>,
    pub store: &'a state::SharedState,
    pub zone_index: usize,
    pub notify: &'a NotifySender,
    pub covers: &'a SharedCoverCache,
    pub track_cache: &'a Option<crate::audio::cache::TrackCache>,
}

/// Fetch cover art in the background and update zone state.
///
/// For radio: fetches from config URL. For subsonic: fetches from getCoverArt.
/// Stores bytes in `SharedCoverCache` and sets `cover_url` on the zone.
pub fn spawn_cover_fetch(
    handle: &mut Option<JoinHandle<()>>,
    covers: &SharedCoverCache,
    store: &state::SharedState,
    zone_index: usize,
    notify: &NotifySender,
    url: String,
) {
    if let Some(h) = handle.take() {
        h.abort();
    }
    let covers = covers.clone();
    let store = store.clone();
    let notify = notify.clone();
    *handle = Some(tokio::spawn(async move {
        if let Some((bytes, mime)) = state::cover::fetch_cover(&url).await {
            let mut cache = covers.write().await;
            cache.set(zone_index, bytes, mime);
            let hash = cache.get(zone_index).map(|e| e.hash.clone());
            drop(cache);
            if let Some(h) = hash {
                let cover_url = format!("/api/v1/zones/{zone_index}/cover?h={h}");
                update_and_notify(&store, zone_index, &notify, |z| {
                    z.cover_url = Some(cover_url.clone());
                })
                .await;
            }
        }
    }));
}
pub async fn start_subsonic_track_decode(
    sub: &SubsonicClient,
    track: &crate::subsonic::Track,
    ds: &mut DecodeState<'_>,
    ctx: &PlaybackCtx<'_>,
) {
    let (tx, rx) = audio::pcm_channel(super::runner::PCM_DECODE_CHANNEL_SIZE);
    *ds.decode_rx = Some(rx);
    if let Some(ref cover_id) = track.cover_art {
        let cover_url = sub.cover_art_fetch_url(cover_id);
        spawn_cover_fetch(
            ds.current_cover,
            ctx.covers,
            ctx.store,
            ctx.zone_index,
            ctx.notify,
            cover_url,
        );
    }

    // Check cache for this track
    if let Some(cache) = ctx.track_cache {
        if let audio::cache::CacheEntry::Complete { path, content_type } = cache.get(&track.id) {
            // Cache hit — decode directly from file (instant, seekable)
            tracing::info!(track = %track.title, "Playing from cache");
            let inv_cache = cache.clone();
            let inv_tid = track.id.clone();
            *ds.current_decode = Some(tokio::spawn(async move {
                if let Err(e) = tokio::task::spawn_blocking(move || {
                    audio::decode_cached_file(&path, &content_type, None, &tx)
                })
                .await
                .unwrap_or_else(|e| Err(e.into()))
                {
                    tracing::error!(error = %e, "Cached decode failed");
                    inv_cache.invalidate(&inv_tid);
                }
            }));
            return;
        }
    }

    // Cache miss — stream and cache simultaneously
    let url = sub.stream_url(&track.id);

    if let Some(cache) = ctx.track_cache {
        let cache = cache.clone();
        let tid = track.id.clone();
        *ds.current_decode = Some(tokio::spawn(async move {
            if let Err(e) = audio::decode_http_stream_cached(url, tx, &cache, &tid).await {
                tracing::error!(error = %e, "Subsonic cached decode failed");
            }
        }));
        return;
    }

    // Fallback — no cache, stream directly
    let ac = ctx.config.audio.clone();
    *ds.current_decode = Some(tokio::spawn(async move {
        if let Err(e) = audio::decode_http_stream(url, tx, ac, None).await {
            tracing::error!(error = %e, "Subsonic decode failed");
        }
    }));
}

pub fn subsonic_track_info(track: &crate::subsonic::Track) -> TrackInfo {
    TrackInfo {
        title: track.title.clone(),
        artist: track.artist.clone().unwrap_or_default(),
        album: track.album.clone().unwrap_or_default(),
        album_artist: None,
        genre: None,
        year: None,
        track_number: track.track,
        disc_number: None,
        duration_ms: {
            #[allow(clippy::cast_possible_wrap)]
            let v = (track.duration * 1000) as i64;
            v
        },
        position_ms: 0,
        seekable: true,
        source: SourceType::SubsonicPlaylist,
        bitrate_kbps: None,
        content_type: None,
        sample_rate: None,
    }
}

pub fn radio_track_info(name: &str) -> TrackInfo {
    TrackInfo {
        title: name.to_string(),
        artist: "Radio".into(),
        album: String::new(),
        album_artist: None,
        genre: None,
        year: None,
        track_number: None,
        disc_number: None,
        duration_ms: 0,
        position_ms: 0,
        seekable: false,
        source: SourceType::Radio,
        bitrate_kbps: None,
        content_type: None,
        sample_rate: None,
    }
}

pub async fn start_radio_decode(
    radio: &crate::config::RadioConfig,
    ds: &mut DecodeState<'_>,
    ctx: &PlaybackCtx<'_>,
) {
    let (tx, rx) = audio::pcm_channel(super::runner::PCM_DECODE_CHANNEL_SIZE);
    *ds.decode_rx = Some(rx);
    let (icy_tx, mut icy_rx) = mpsc::channel::<audio::icy::IcyMetadata>(4);
    let icy_store = ctx.store.clone();
    let icy_notify = ctx.notify.clone();
    let icy_covers = ctx.covers.clone();
    let zone_index = ctx.zone_index;
    let fallback_cover = radio.cover.clone();
    tokio::spawn(async move {
        let mut meta_cover_handle: Option<JoinHandle<()>> = None;
        while let Some(meta) = icy_rx.recv().await {
            if let Some(raw_title) = meta.title {
                tracing::debug!(zone = zone_index, title = %raw_title, "ICY metadata");
                let (artist, title) = parse_icy_title(&raw_title);
                update_and_notify(&icy_store, zone_index, &icy_notify, |z| {
                    if let Some(ref mut track) = z.track {
                        track.artist.clone_from(&artist);
                        track.title.clone_from(&title);
                    }
                })
                .await;
            }
            // Use StreamUrl as cover if it's an image, else fall back to config cover
            let cover_url = match meta.url {
                Some(ref url) if is_image_url(url).await => Some(url.clone()),
                _ => fallback_cover.clone(),
            };
            if let Some(url) = cover_url {
                spawn_cover_fetch(
                    &mut meta_cover_handle,
                    &icy_covers,
                    &icy_store,
                    zone_index,
                    &icy_notify,
                    url,
                );
            }
        }
    });
    if let Some(h) = ds.current_cover.take() {
        h.abort();
    }
    {
        let covers = ctx.covers.clone();
        let store = ctx.store.clone();
        let notify = ctx.notify.clone();
        let zone_index = ctx.zone_index;
        let cover_url = radio.cover.clone();
        let stream_url = radio.url.clone();
        *ds.current_cover = Some(tokio::spawn(async move {
            if let Some((bytes, mime)) =
                state::cover::fetch_cover_with_favicon_fallback(cover_url.as_deref(), &stream_url)
                    .await
            {
                let mut cache = covers.write().await;
                cache.set(zone_index, bytes, mime);
                let hash = cache.get(zone_index).map(|e| e.hash.clone());
                drop(cache);
                if let Some(h) = hash {
                    let url = format!("/api/v1/zones/{zone_index}/cover?h={h}");
                    update_and_notify(&store, zone_index, &notify, |z| {
                        z.cover_url = Some(url.clone());
                    })
                    .await;
                }
            }
        }));
    }
    let url = radio.url.clone();
    let ac = ctx.config.audio.clone();
    *ds.current_decode = Some(tokio::spawn(async move {
        if let Err(e) = audio::decode_http_stream(url, tx, ac, Some(icy_tx)).await {
            tracing::error!(error = %e, "Radio decode failed");
        }
    }));
}

pub async fn handle_next(ds: &mut DecodeState<'_>, ctx: &PlaybackCtx<'_>) {
    let was_playing = ctx
        .store
        .read()
        .await
        .zones
        .get(&ctx.zone_index)
        .is_some_and(|z| z.playback == crate::state::PlaybackState::Playing);

    match ds.source.clone() {
        ActiveSource::Radio { index } => {
            let next = nav::radio_next_index(index, ctx.config.radios.len());
            stop_decode(ds.current_decode, ds.decode_rx).await;
            if let Some(radio) = ctx.config.radios.get(next) {
                if was_playing {
                    start_radio_decode(radio, ds, ctx).await;
                }
                *ds.source = ActiveSource::Radio { index: next };
                update_and_notify(ctx.store, ctx.zone_index, ctx.notify, |z| {
                    if was_playing {
                        z.playback = crate::state::PlaybackState::Playing;
                    }
                    z.playlist_index = Some(0);
                    z.playlist_track_index = Some(next);
                    z.track = Some(radio_track_info(&radio.name));
                })
                .await;
                tracing::info!(zone = %ctx.config.zones[ctx.zone_index - 1].name, radio = %radio.name, "Next station");
            }
        }
        ActiveSource::SubsonicPlaylist {
            playlist_id,
            track_index,
            track_count,
        } => {
            let repeat = ctx
                .store
                .read()
                .await
                .zones
                .get(&ctx.zone_index)
                .map_or(snapdog_common::RepeatMode::Off, |z| z.repeat);
            if let Some(i) = nav::next_index(track_index, track_count, repeat) {
                advance_playlist_track(ds, &playlist_id, i, track_count, ctx).await;
            } else {
                stop_decode(ds.current_decode, ds.decode_rx).await;
                *ds.source = ActiveSource::Idle;
                update_and_notify(ctx.store, ctx.zone_index, ctx.notify, |z| {
                    z.playback = PlaybackState::Stopped;
                    z.source = SourceType::Idle;
                })
                .await;
            }
        }
        _ => {}
    }
}

pub async fn handle_previous(ds: &mut DecodeState<'_>, ctx: &PlaybackCtx<'_>) {
    // CD-player behavior: if position > 3s, restart current track.
    // Otherwise, go to previous track.
    let (position_ms, was_playing) = {
        let z = ctx.store.read().await.zones.get(&ctx.zone_index).cloned();
        (
            z.as_ref()
                .and_then(|z| z.track.as_ref().map(|t| t.position_ms))
                .unwrap_or(0),
            z.as_ref()
                .is_some_and(|z| z.playback == crate::state::PlaybackState::Playing),
        )
    };

    match ds.source.clone() {
        ActiveSource::Radio { index } => {
            let prev = nav::radio_prev_index(index, ctx.config.radios.len());
            stop_decode(ds.current_decode, ds.decode_rx).await;
            if let Some(radio) = ctx.config.radios.get(prev) {
                if was_playing {
                    start_radio_decode(radio, ds, ctx).await;
                }
                *ds.source = ActiveSource::Radio { index: prev };
                update_and_notify(ctx.store, ctx.zone_index, ctx.notify, |z| {
                    if was_playing {
                        z.playback = crate::state::PlaybackState::Playing;
                    }
                    z.playlist_index = Some(0);
                    z.playlist_track_index = Some(prev);
                    z.track = Some(radio_track_info(&radio.name));
                })
                .await;
                tracing::info!(zone = %ctx.config.zones[ctx.zone_index - 1].name, radio = %radio.name, "Previous station");
            }
        }
        ActiveSource::SubsonicPlaylist {
            playlist_id,
            track_index,
            track_count,
        } => {
            if let Some(i) = nav::prev_index(track_index, position_ms) {
                advance_playlist_track(ds, &playlist_id, i, track_count, ctx).await;
            }
        }
        _ => {}
    }
}

pub async fn handle_track_complete(ds: &mut DecodeState<'_>, ctx: &PlaybackCtx<'_>) {
    let (repeat, shuffle) = {
        let z = ctx.store.read().await.zones.get(&ctx.zone_index).cloned();
        (
            z.as_ref()
                .map_or(snapdog_common::RepeatMode::Off, |z| z.repeat),
            z.as_ref().is_some_and(|z| z.shuffle),
        )
    };

    match ds.source.clone() {
        ActiveSource::SubsonicPlaylist {
            playlist_id,
            track_index,
            track_count,
        } => {
            let draw = if shuffle {
                fastrand::usize(..track_count)
            } else {
                0
            };
            if let Some(i) = nav::complete_index(track_index, repeat, shuffle, draw) {
                advance_playlist_track(ds, &playlist_id, i, track_count, ctx).await;
            } else {
                handle_next(ds, ctx).await;
            }
        }
        ActiveSource::Radio { index } => {
            tracing::warn!(zone = ctx.zone_index, "Radio stream ended, restarting");
            if let Some(radio) = ctx.config.radios.get(index) {
                start_radio_decode(radio, ds, ctx).await;
            }
        }
        ActiveSource::AirPlay => {
            *ds.source = ActiveSource::Idle;
            update_and_notify(ctx.store, ctx.zone_index, ctx.notify, |z| {
                z.playback = PlaybackState::Stopped;
                z.source = SourceType::Idle;
                z.track = None;
            })
            .await;
            tracing::info!(zone = %ctx.config.zones[ctx.zone_index - 1].name, "AirPlay ended");
        }
        _ => {
            *ds.source = ActiveSource::Idle;
            update_and_notify(ctx.store, ctx.zone_index, ctx.notify, |z| {
                z.playback = PlaybackState::Stopped;
                z.source = SourceType::Idle;
            })
            .await;
        }
    }
}

async fn advance_playlist_track(
    ds: &mut DecodeState<'_>,
    playlist_id: &str,
    track_index: usize,
    track_count: usize,
    ctx: &PlaybackCtx<'_>,
) {
    let was_playing = ctx
        .store
        .read()
        .await
        .zones
        .get(&ctx.zone_index)
        .is_some_and(|z| z.playback == crate::state::PlaybackState::Playing);

    stop_decode(ds.current_decode, ds.decode_rx).await;
    if let Some(sub) = &ctx.subsonic {
        if let Ok(playlist) = sub.get_playlist(playlist_id).await {
            if let Some(track) = playlist.entry.get(track_index) {
                // Only start decode if we were playing; otherwise just load metadata
                if was_playing {
                    start_subsonic_track_decode(sub, track, ds, ctx).await;
                }
                *ds.source = ActiveSource::SubsonicPlaylist {
                    playlist_id: playlist_id.to_string(),
                    track_index,
                    track_count,
                };
                update_and_notify(ctx.store, ctx.zone_index, ctx.notify, |z| {
                    if was_playing {
                        z.playback = crate::state::PlaybackState::Playing;
                    }
                    z.playlist_track_index = Some(track_index);
                    z.track = Some(subsonic_track_info(track));
                    z.track.as_mut().unwrap().position_ms = 0;
                })
                .await;
                // Prefetch: always cache the current + next tracks
                if let Some(cache) = ctx.track_cache {
                    let lookahead = ctx
                        .config
                        .subsonic
                        .as_ref()
                        .map_or(0, |_| crate::config::CACHE_LOOKAHEAD);
                    prefetch_next_tracks(sub, &playlist.entry, track_index, cache, lookahead);
                }
                tracing::info!(
                    zone = ctx.zone_index,
                    track = track_index,
                    "Advanced to track"
                );
            }
        }
    }
}

use std::sync::LazyLock;

static HTTP_CLIENT: LazyLock<reqwest::Client> = LazyLock::new(reqwest::Client::new);

/// Check if a URL points to an image by sending a HEAD request and checking Content-Type.
async fn is_image_url(url: &str) -> bool {
    let Ok(resp) = HTTP_CLIENT
        .head(url)
        .timeout(std::time::Duration::from_secs(3))
        .send()
        .await
    else {
        tracing::debug!(url, "HEAD request failed for cover URL check");
        return false;
    };
    resp.headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|ct| ct.starts_with("image/"))
}

/// Parse ICY `StreamTitle` into (artist, title).
///
/// Strips station prefixes (before `|` or `:`) and splits on ` - `.
/// Examples:
///   "SWR3: Coldplay - Yellow"       → ("Coldplay", "Yellow")
///   "ANTENNE | Ed Sheeran - Shape"  → ("Ed Sheeran", "Shape")
///   "Coldplay - Yellow"             → ("Coldplay", "Yellow")
///   "Nachrichten"                   → ("", "Nachrichten")
fn parse_icy_title(raw: &str) -> (String, String) {
    // Strip station prefix: take content after last `|` or after first `:` followed by space
    let stripped = raw
        .rsplit_once('|')
        .map(|(_, rest)| rest.trim())
        .or_else(|| raw.split_once(": ").map(|(_, rest)| rest))
        .unwrap_or(raw)
        .trim();

    // Split artist - title on ` - ` (most common separator)
    if let Some((artist, title)) = stripped.split_once(" - ") {
        (artist.trim().to_string(), title.trim().to_string())
    } else if let Some((artist, title)) = stripped.split_once(" – ") {
        // En-dash variant
        (artist.trim().to_string(), title.trim().to_string())
    } else {
        (String::new(), stripped.to_string())
    }
}

/// Pre-fetch upcoming playlist tracks into the disk cache in the background.
///
/// Spawns low-priority download tasks for the next `lookahead` tracks that aren't
/// already cached. Does nothing if cache is disabled or lookahead is 0.
pub fn prefetch_next_tracks(
    sub: &SubsonicClient,
    playlist_tracks: &[crate::subsonic::Track],
    current_index: usize,
    cache: &crate::audio::cache::TrackCache,
    lookahead: usize,
) {
    if lookahead == 0 {
        return;
    }

    let upcoming: Vec<_> = playlist_tracks
        .iter()
        .skip(current_index + 1)
        .take(lookahead)
        .filter(|t| !cache.is_complete(&t.id))
        .map(|t| (t.id.clone(), sub.stream_url(&t.id)))
        .collect();

    if upcoming.is_empty() {
        return;
    }

    let cache = cache.clone();
    tokio::spawn(async move {
        for (track_id, url) in upcoming {
            tracing::debug!(track_id = %track_id, "Prefetching track");
            if let Err(e) = prefetch_one(&cache, &track_id, &url).await {
                tracing::debug!(track_id = %track_id, error = %e, "Prefetch failed");
            }
            // Yield between downloads to avoid starving playback
            tokio::task::yield_now().await;
        }
    });
}

async fn prefetch_one(
    cache: &crate::audio::cache::TrackCache,
    track_id: &str,
    url: &str,
) -> anyhow::Result<()> {
    use futures_util::StreamExt;

    anyhow::ensure!(
        url::Url::parse(url)
            .is_ok_and(|u| matches!(u.scheme(), "http" | "https") && u.host().is_some()),
        "Only http and https URLs can be prefetched"
    );
    let client = reqwest::Client::new();
    let response = client.get(url).send().await?.error_for_status()?;

    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("audio/mpeg")
        .to_string();
    let content_length = response.content_length();

    let mut writer = cache.start_download(track_id, &content_type, content_length)?;

    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let bytes = chunk?;
        writer.write(&bytes)?;
    }
    writer.complete()?;
    tracing::debug!(track_id = %track_id, "Prefetch complete");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_icy_title_basic() {
        assert_eq!(
            parse_icy_title("Coldplay - Yellow"),
            ("Coldplay".into(), "Yellow".into())
        );
    }

    #[test]
    fn parse_icy_title_with_station_pipe() {
        assert_eq!(
            parse_icy_title("ANTENNE BAYERN | Ed Sheeran - Shape Of You"),
            ("Ed Sheeran".into(), "Shape Of You".into())
        );
    }

    #[test]
    fn parse_icy_title_with_station_colon() {
        assert_eq!(
            parse_icy_title("SWR3: Coldplay - Yellow"),
            ("Coldplay".into(), "Yellow".into())
        );
    }

    #[test]
    fn parse_icy_title_no_separator() {
        assert_eq!(
            parse_icy_title("Nachrichten"),
            (String::new(), "Nachrichten".into())
        );
    }

    #[test]
    fn parse_icy_title_en_dash() {
        assert_eq!(
            parse_icy_title("Adele \u{2013} Hello"),
            ("Adele".into(), "Hello".into())
        );
    }
}
