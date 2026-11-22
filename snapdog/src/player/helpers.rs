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
    pub playlist_cache: &'a crate::subsonic::PlaylistCache,
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
                let cover_url = state::cover::zone_cover_url(zone_index, &h);
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
    if let Some(cache) = ctx.track_cache
        && let audio::cache::CacheEntry::Complete { path, content_type } = cache.get(&track.id)
    {
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
    let icy_task = tokio::spawn(async move {
        // Dropped with the task, so a cover fetch of this station ends with it.
        let mut meta_cover = AbortOnDrop(None);
        let meta_cover_handle = &mut meta_cover.0;
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
                    meta_cover_handle,
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
                    let url = state::cover::zone_cover_url(zone_index, &h);
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
    // The metadata task lives as long as the decode: when the decode ends or is aborted on a
    // source change, titles still buffered for this station are not written to the next source.
    let icy_guard = AbortOnDrop(Some(icy_task));
    *ds.current_decode = Some(tokio::spawn(async move {
        let _icy_guard = icy_guard;
        run_radio_with_reconnect(&url, &tx, &ac, &icy_tx, &RadioReconnect::DEFAULT).await;
    }));
}

/// How a radio stream is reconnected after an error or an end of stream.
#[derive(Debug, Clone, Copy)]
pub struct RadioReconnect {
    /// Reconnects in a row before the zone is stopped with an error.
    pub max_attempts: u32,
    /// Delay before the first reconnect; doubles with every further attempt.
    pub base_delay: std::time::Duration,
    /// Upper bound for the delay.
    pub max_delay: std::time::Duration,
    /// A connection that played this long counts as healthy and resets the attempt count.
    pub healthy_after: std::time::Duration,
}

impl RadioReconnect {
    pub const DEFAULT: Self = Self {
        max_attempts: 5,
        base_delay: std::time::Duration::from_secs(1),
        max_delay: std::time::Duration::from_secs(30),
        healthy_after: std::time::Duration::from_secs(60),
    };

    /// Delay before reconnect number `attempt` (1-based), or `None` once attempts are used up.
    #[must_use]
    pub fn delay(&self, attempt: u32) -> Option<std::time::Duration> {
        (attempt <= self.max_attempts).then(|| {
            self.base_delay
                .saturating_mul(1 << attempt.saturating_sub(1).min(16))
                .min(self.max_delay)
        })
    }
}

/// Play a radio stream and reconnect with backoff when it fails or ends: a station that drops
/// the connection (a clean end of stream included) comes back on its own, and one that keeps
/// failing stops the zone with an error after `policy.max_attempts` tries instead of looping.
pub async fn run_radio_with_reconnect(
    url: &str,
    tx: &audio::PcmSender,
    audio_config: &crate::config::AudioConfig,
    icy_tx: &mpsc::Sender<audio::icy::IcyMetadata>,
    policy: &RadioReconnect,
) {
    let mut attempt = 0u32;
    loop {
        let started = std::time::Instant::now();
        let result = audio::decode_http_stream_unreported(
            url.to_owned(),
            tx.clone(),
            audio_config.clone(),
            Some(icy_tx.clone()),
        )
        .await;
        if tx.is_closed() {
            return; // The zone moved on.
        }
        if started.elapsed() >= policy.healthy_after {
            attempt = 0;
        }
        attempt += 1;
        let reason = match &result {
            Ok(()) => "stream ended".to_owned(),
            Err(e) => format!("{e:#}"),
        };
        let Some(delay) = policy.delay(attempt) else {
            tracing::error!(%url, reason, "Radio stream failed, giving up");
            let (message, details) = match result {
                Err(e) => (e.to_string(), e.source().map(ToString::to_string)),
                Ok(()) => ("Radio stream keeps ending".to_owned(), None),
            };
            let _ = tx.send(audio::PcmMessage::Error { message, details }).await;
            return;
        };
        tracing::warn!(%url, reason, attempt, ?delay, "Radio stream interrupted, reconnecting");
        tokio::time::sleep(delay).await;
    }
}

/// Aborts the task it holds when dropped.
struct AbortOnDrop(Option<JoinHandle<()>>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        if let Some(handle) = self.0.take() {
            handle.abort();
        }
    }
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
                let total = radio_playlist_total(ctx).await;
                update_and_notify(ctx.store, ctx.zone_index, ctx.notify, |z| {
                    if was_playing {
                        z.playback = crate::state::PlaybackState::Playing;
                    }
                    apply_radio_state(z, ctx.config, total, next, radio);
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
            // `repeat` only affects the end-of-playlist decision (next_index ignores
            // it mid-playlist), so skip the store read on the common advance.
            let repeat = if track_index + 1 >= track_count {
                ctx.store
                    .read()
                    .await
                    .zones
                    .get(&ctx.zone_index)
                    .map_or(snapdog_common::RepeatMode::Off, |z| z.repeat)
            } else {
                snapdog_common::RepeatMode::Off
            };
            if let Some(i) = nav::next_index(track_index, track_count, repeat) {
                advance_playlist_track(ds, &playlist_id, i, track_count, ctx).await;
            } else {
                enter_idle(ds, ctx).await;
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
                let total = radio_playlist_total(ctx).await;
                update_and_notify(ctx.store, ctx.zone_index, ctx.notify, |z| {
                    if was_playing {
                        z.playback = crate::state::PlaybackState::Playing;
                    }
                    apply_radio_state(z, ctx.config, total, prev, radio);
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
            // complete_index uses `draw` only when shuffle is on AND repeat != Track
            // (Track replays the current track and wins). Only draw then, so the RNG
            // isn't advanced on a discarded value.
            let draw = if shuffle && !matches!(repeat, snapdog_common::RepeatMode::Track) {
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
        // The radio decode task reconnects by itself and reports an error once it gives up, so
        // its channel closing here means the task ended abnormally: stop instead of restarting
        // it without any bound (falls through to the default arm).
        ActiveSource::AirPlay => {
            enter_idle(ds, ctx).await;
            tracing::info!(zone = %ctx.config.zones[ctx.zone_index - 1].name, "AirPlay ended");
        }
        _ => enter_idle(ds, ctx).await,
    }
}

/// A track could not be fetched: leave the zone idle, so it never reads `Playing` without a
/// decoder behind it.
pub(super) async fn stop_after_failed_fetch(ds: &mut DecodeState<'_>, ctx: &PlaybackCtx<'_>) {
    enter_idle(ds, ctx).await;
}

/// Leave the zone idle: the decode and the cover fetch stopped, the internal source `Idle`,
/// the cover image dropped, and in the store `Stopped` with no track, cover URL or buffered
/// duration. The one implementation behind Stop, the end of a playlist or stream, a playback
/// error, a failed fetch, presence auto-off and the end of a receiver session.
pub async fn enter_idle(ds: &mut DecodeState<'_>, ctx: &PlaybackCtx<'_>) {
    enter_idle_with(ds, ctx, |_| {}).await;
}

/// [`enter_idle`], with `also` applied in the same store update (presence auto-off clears its
/// flags there, so the zone changes in one notification).
pub async fn enter_idle_with(
    ds: &mut DecodeState<'_>,
    ctx: &PlaybackCtx<'_>,
    also: impl FnOnce(&mut state::ZoneState),
) {
    stop_decode(ds.current_decode, ds.decode_rx).await;
    if let Some(h) = ds.current_cover.take() {
        h.abort();
    }
    *ds.source = ActiveSource::Idle;
    ctx.covers.write().await.clear(ctx.zone_index);
    update_and_notify(ctx.store, ctx.zone_index, ctx.notify, |z| {
        z.playback = PlaybackState::Stopped;
        z.source = SourceType::Idle;
        z.track = None;
        z.cover_url = None;
        z.buffered_ms = None;
        also(z);
    })
    .await;
}

/// The unified playlist count as radio reports it: radio plus the Subsonic playlists known to
/// the cache. Read without a request, so switching stations never waits for the server.
pub async fn radio_playlist_total(ctx: &PlaybackCtx<'_>) -> usize {
    ctx.config
        .unified_playlist_count(ctx.playlist_cache.cached_len().await.unwrap_or(0))
}

/// The zone state of radio station `index`: the radio playlist (unified index 0) with its
/// name, total and track count, the station as track. Playback is left to the caller.
pub fn apply_radio_state(
    z: &mut state::ZoneState,
    config: &AppConfig,
    playlist_total: usize,
    index: usize,
    radio: &crate::config::RadioConfig,
) {
    z.source = SourceType::Radio;
    z.playlist_index = Some(0);
    z.playlist_id = Some(crate::config::RADIO_PLAYLIST_ID.into());
    z.playlist_name = Some("Radio".into());
    z.playlist_total = Some(playlist_total);
    z.playlist_track_index = Some(index);
    z.playlist_track_count = Some(config.radios.len());
    z.track = Some(radio_track_info(&radio.name));
}

/// Start decoding Subsonic track `track_id` at `pos_ms`, for resume and seek: from the complete
/// cached file when there is one (a cache entry that fails to decode is invalidated), else
/// streamed from the server with a time offset. The caller has stopped the previous decode.
///
/// Returns the offset to add to the decoder's positions: 0 for the cached file, whose positions
/// are absolute, `pos_ms` for a stream that starts at the offset.
pub fn start_track_at(
    sub: &SubsonicClient,
    track_id: &str,
    pos_ms: i64,
    ctx: &PlaybackCtx<'_>,
    current_decode: &mut Option<JoinHandle<()>>,
    decode_rx: &mut Option<mpsc::Receiver<audio::PcmMessage>>,
) -> i64 {
    let (tx, rx) = audio::pcm_channel(super::runner::PCM_DECODE_CHANNEL_SIZE);
    *decode_rx = Some(rx);
    let cached = ctx.track_cache.as_ref().and_then(|cache| {
        if let audio::cache::CacheEntry::Complete { path, content_type } = cache.get(track_id) {
            Some((path, content_type))
        } else {
            None
        }
    });
    if let Some((path, content_type)) = cached {
        let cache = ctx.track_cache.clone();
        let tid = track_id.to_owned();
        *current_decode = Some(tokio::spawn(async move {
            let decoded = tokio::task::spawn_blocking(move || {
                audio::decode_cached_file(&path, &content_type, Some(pos_ms), &tx)
            })
            .await
            .unwrap_or_else(|e| Err(e.into()));
            if let Err(e) = decoded {
                tracing::error!(error = %e, "Decode from the track cache failed");
                if let Some(cache) = cache {
                    cache.invalidate(&tid);
                }
            }
        }));
        0
    } else {
        let offset_secs = u64::try_from(pos_ms / 1000).unwrap_or(0);
        let url = sub.stream_url_with_offset(track_id, offset_secs);
        let ac = ctx.config.audio.clone();
        *current_decode = Some(tokio::spawn(async move {
            if let Err(e) = audio::decode_http_stream(url, tx, ac, None).await {
                tracing::error!(error = %e, "Track decode at an offset failed");
            }
        }));
        pos_ms
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

    // Fetch before touching the running decode. `get_queue` also resolves the reserved
    // album ids, which `get_playlist` does not know.
    let fetched = match &ctx.subsonic {
        Some(sub) => match sub.get_queue(playlist_id).await {
            Ok(playlist) if playlist.entry.get(track_index).is_some() => Some((sub, playlist)),
            Ok(_) => {
                tracing::warn!(
                    zone = ctx.zone_index,
                    track = track_index,
                    "Queue has no such track"
                );
                None
            }
            Err(e) => {
                tracing::warn!(zone = ctx.zone_index, error = %e, "Could not fetch the queue to advance");
                None
            }
        },
        None => None,
    };
    let Some((sub, playlist)) = fetched else {
        stop_after_failed_fetch(ds, ctx).await;
        return;
    };
    let track = &playlist.entry[track_index];
    stop_decode(ds.current_decode, ds.decode_rx).await;
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
    let client = audio::streaming_client()?;
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

    // --- radio reconnect ---

    const FAST: RadioReconnect = RadioReconnect {
        max_attempts: 2,
        base_delay: std::time::Duration::from_millis(1),
        max_delay: std::time::Duration::from_millis(5),
        healthy_after: std::time::Duration::from_secs(3600),
    };

    #[test]
    fn radio_reconnect_backs_off_and_gives_up() {
        let p = RadioReconnect::DEFAULT;
        let secs = |a| p.delay(a).map(|d| d.as_secs());
        assert_eq!(
            (1..=6).map(secs).collect::<Vec<_>>(),
            [Some(1), Some(2), Some(4), Some(8), Some(16), None]
        );
        let capped = RadioReconnect {
            max_attempts: 40,
            ..p
        };
        assert_eq!(capped.delay(40), Some(capped.max_delay));
    }

    #[tokio::test]
    async fn a_failing_station_is_retried_then_stops_the_zone_with_an_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;
        let (tx, mut rx) = audio::pcm_channel(16);
        let (icy_tx, _icy_rx) = mpsc::channel(4);
        let url = format!("{}/stream", server.uri());
        tokio::time::timeout(
            std::time::Duration::from_secs(10),
            run_radio_with_reconnect(
                &url,
                &tx,
                &crate::config::AudioConfig::default(),
                &icy_tx,
                &FAST,
            ),
        )
        .await
        .expect("gives up after the bounded attempts");

        let stream_requests = server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .filter(|r| r.url.path() == "/stream")
            .count();
        assert!(
            stream_requests >= 3,
            "first try and two reconnects, got {stream_requests}"
        );
        let mut error = None;
        while let Ok(msg) = rx.try_recv() {
            if let audio::PcmMessage::Error { message, .. } = msg {
                error = Some(message);
            }
        }
        assert!(error.is_some(), "the zone learns that the station is gone");
    }

    #[tokio::test]
    async fn the_reconnect_loop_ends_when_the_zone_moves_on() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;
        let (tx, rx) = audio::pcm_channel(16);
        drop(rx);
        let (icy_tx, _icy_rx) = mpsc::channel(4);
        let patient = RadioReconnect {
            max_attempts: 1000,
            ..FAST
        };
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            run_radio_with_reconnect(
                &format!("{}/stream", server.uri()),
                &tx,
                &crate::config::AudioConfig::default(),
                &icy_tx,
                &patient,
            ),
        )
        .await
        .expect("a closed channel ends the loop");
    }

    // --- queue advance against a mock Subsonic server ---

    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    struct Harness {
        config: AppConfig,
        subsonic: Option<SubsonicClient>,
        store: state::SharedState,
        notify: NotifySender,
        covers: SharedCoverCache,
        cache: Option<crate::audio::cache::TrackCache>,
        playlist_cache: crate::subsonic::PlaylistCache,
    }

    fn harness(server: &MockServer) -> Harness {
        let raw: crate::config::FileConfig = toml::from_str(&format!(
            r#"
            [[zone]]
            name = "Ground Floor"
            [subsonic]
            url = "{}"
            username = "u"
            password = "p"
            [[client]]
            name = "Living Room"
            mac = "00:00:00:00:00:00"
            zone = "Ground Floor"
        "#,
            server.uri()
        ))
        .unwrap();
        let config = crate::config::load_raw(raw).unwrap();
        let subsonic = config.subsonic.as_ref().map(SubsonicClient::new);
        let store = state::init(&config, None).unwrap();
        Harness {
            config,
            subsonic,
            store,
            notify: crate::api::ws::notification_channel().0,
            covers: state::cover::new_cache(),
            cache: None,
            playlist_cache: crate::subsonic::PlaylistCache::new(),
        }
    }

    /// #499: the end of a playlist leaves the zone idle like Stop does.
    #[tokio::test]
    async fn the_end_of_a_playlist_leaves_the_zone_idle() {
        let server = MockServer::start().await;
        let h = harness(&server);
        let (mut decode, mut cover, mut rx) = (None, None, None);
        let mut source = ActiveSource::SubsonicPlaylist {
            playlist_id: "p".into(),
            track_index: 1,
            track_count: 2,
        };
        update_and_notify(&h.store, 1, &h.notify, |z| {
            z.playback = PlaybackState::Playing;
            z.track = Some(radio_track_info("Last"));
            z.cover_url = Some("/api/v1/zones/1/cover?h=x".into());
            z.buffered_ms = Some(500);
        })
        .await;
        let ctx = PlaybackCtx {
            config: &h.config,
            subsonic: &h.subsonic,
            store: &h.store,
            zone_index: 1,
            notify: &h.notify,
            covers: &h.covers,
            track_cache: &h.cache,
            playlist_cache: &h.playlist_cache,
        };
        handle_track_complete(
            &mut DecodeState {
                current_decode: &mut decode,
                current_cover: &mut cover,
                decode_rx: &mut rx,
                source: &mut source,
            },
            &ctx,
        )
        .await;
        assert!(
            matches!(source, ActiveSource::Idle),
            "internal source is idle"
        );
        let z = h.store.read().await.zones[&1].clone();
        assert_eq!(z.playback, PlaybackState::Stopped);
        assert_eq!(z.source, SourceType::Idle);
        assert!(z.track.is_none() && z.cover_url.is_none() && z.buffered_ms.is_none());
    }

    async fn run_advance(
        h: &Harness,
        queue_id: &str,
        index: usize,
    ) -> (ActiveSource, PlaybackState) {
        let (mut decode, mut cover, mut rx) = (None, None, None);
        let mut source = ActiveSource::Idle;
        let mut ds = DecodeState {
            current_decode: &mut decode,
            current_cover: &mut cover,
            decode_rx: &mut rx,
            source: &mut source,
        };
        let ctx = PlaybackCtx {
            config: &h.config,
            subsonic: &h.subsonic,
            store: &h.store,
            zone_index: 1,
            notify: &h.notify,
            covers: &h.covers,
            track_cache: &h.cache,
            playlist_cache: &h.playlist_cache,
        };
        // The zone reads Playing, as it does when a track ends on its own.
        update_and_notify(&h.store, 1, &h.notify, |z| {
            z.playback = PlaybackState::Playing;
        })
        .await;
        advance_playlist_track(&mut ds, queue_id, index, 2, &ctx).await;
        let playback = h.store.read().await.zones[&1].playback.clone();
        (source, playback)
    }

    fn album_json() -> serde_json::Value {
        let song = |id: &str, title: &str| serde_json::json!({"id": id, "title": title, "artist": "A", "album": "Alb", "duration": 60});
        serde_json::json!({"subsonic-response": {"status": "ok", "version": "1.16.1",
            "album": {"id": "al1", "name": "Alb", "song": [song("t1", "One"), song("t2", "Two")]}}})
    }

    #[tokio::test]
    async fn advancing_in_an_album_queue_fetches_the_album_and_selects_the_next_track() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/rest/getAlbum"))
            .and(query_param("id", "al1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(album_json()))
            .mount(&server)
            .await;
        let h = harness(&server);
        let queue_id = crate::subsonic::album_queue_id("al1");

        let (source, playback) = run_advance(&h, &queue_id, 1).await;

        assert!(
            matches!(
                source,
                ActiveSource::SubsonicPlaylist { track_index: 1, .. }
            ),
            "the album queue advances to track 1"
        );
        assert_eq!(playback, PlaybackState::Playing);
        assert_eq!(h.store.read().await.zones[&1].playlist_track_index, Some(1));
    }

    #[tokio::test]
    async fn a_failed_fetch_leaves_the_zone_stopped_and_not_playing_without_a_decoder() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;
        let h = harness(&server);

        let (source, playback) = run_advance(&h, "pl1", 1).await;

        assert!(matches!(source, ActiveSource::Idle));
        assert_eq!(playback, PlaybackState::Stopped);
    }

    #[tokio::test]
    async fn an_index_beyond_the_queue_leaves_the_zone_stopped() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/rest/getAlbum"))
            .respond_with(ResponseTemplate::new(200).set_body_json(album_json()))
            .mount(&server)
            .await;
        let h = harness(&server);
        let queue_id = crate::subsonic::album_queue_id("al1");

        let (_, playback) = run_advance(&h, &queue_id, 5).await;

        assert_eq!(playback, PlaybackState::Stopped);
    }

    /// #492: titles still buffered for a station when its decode is aborted (a source change)
    /// are never applied, because the metadata task ends with the decode.
    #[tokio::test]
    async fn buffered_icy_titles_are_dropped_when_the_decode_is_aborted() {
        let applied = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let (icy_tx, mut icy_rx) = mpsc::channel::<String>(4);
        let (gate_tx, gate_rx) = tokio::sync::oneshot::channel::<()>();

        // The metadata task, held up until the decode is gone, as when it is busy with a cover.
        let sink = applied.clone();
        let icy_task = tokio::spawn(async move {
            let _ = gate_rx.await;
            while let Some(title) = icy_rx.recv().await {
                sink.lock().unwrap().push(title);
            }
        });

        // The decode holds the guard and the sender, as start_radio_decode arranges it.
        let guard = AbortOnDrop(Some(icy_task));
        let decode = tokio::spawn(async move {
            let _guard = guard;
            for title in ["A - one", "A - two"] {
                icy_tx.send(title.to_string()).await.unwrap();
            }
            std::future::pending::<()>().await;
        });
        tokio::task::yield_now().await;

        // Source change: the decode is aborted while both titles are buffered.
        decode.abort();
        let _ = decode.await;
        let _ = gate_tx.send(());
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        assert!(
            applied.lock().unwrap().is_empty(),
            "no title of the old station is applied"
        );
    }

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

    #[tokio::test]
    async fn prefetch_downloads_on_miss_then_serves_from_cache() {
        use crate::audio::cache::{CacheEntry, TrackCache};
        use wiremock::matchers::method;
        use wiremock::{Mock, MockServer, ResponseTemplate};

        // Mock subsonic stream endpoint returning audio bytes.
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "audio/mpeg")
                    .set_body_bytes(vec![0u8; 4096]),
            )
            .mount(&server)
            .await;

        let dir = tempfile::tempdir().unwrap();
        let cache = TrackCache::new(&crate::config::SubsonicCacheConfig {
            path: dir.path().to_string_lossy().into_owned(),
            max_size_mb: 100,
        })
        .unwrap();

        // Miss before the prefetch fetch.
        assert!(matches!(cache.get("track1"), CacheEntry::Miss));

        let url = format!("{}/rest/stream?id=track1", server.uri());
        prefetch_one(&cache, "track1", &url).await.unwrap();

        // Miss → fetch → hit: now served from disk with the response content-type.
        match cache.get("track1") {
            CacheEntry::Complete { content_type, .. } => {
                assert_eq!(content_type, "audio/mpeg");
            }
            _ => panic!("expected Complete cache entry after prefetch"),
        }
    }
}
