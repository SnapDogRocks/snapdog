// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! `ZonePlayer` runner — the per-zone tokio task.

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use super::commands::{ActiveSource, ZoneCommand};
use super::context::{
    GroupAction, SnapcastCmd, ZoneCommandSender, ZonePlayerContext, setup_zone_group, stop_decode,
    update_and_notify, update_volume_and_notify,
};

use super::helpers::{DecodeState, PlaybackCtx};
use super::helpers::{
    handle_next, handle_previous, handle_track_complete, prefetch_next_tracks, radio_track_info,
    start_radio_decode, start_subsonic_track_decode, subsonic_track_info,
};
use crate::audio;
use crate::receiver::ReceiverProvider;
use crate::state::{PlaybackState, SourceType, TrackInfo};
use crate::subsonic::SubsonicClient;
use chrono::Timelike;

/// Channel capacity for zone player commands.
const ZONE_CMD_CHANNEL_SIZE: usize = 32;
/// Channel capacity for receiver events (`AirPlay`, Spotify).
const RECEIVER_EVENT_CHANNEL_SIZE: usize = 32;
/// Channel capacity for receiver audio frames.
const RECEIVER_AUDIO_CHANNEL_SIZE: usize = 128;
/// Channel capacity for PCM decode buffers.
pub(super) const PCM_DECODE_CHANNEL_SIZE: usize = 64;

/// Delay before restarting a crashed zone player.
const ZONE_RESTART_DELAY: std::time::Duration = std::time::Duration::from_secs(5);

/// Spawn a `ZonePlayer` task for each configured zone. Returns command senders.
///
/// # Errors
///
/// Returns an error if any zone player fails to initialize.
pub async fn spawn_zone_players(
    ctx: ZonePlayerContext,
) -> Result<HashMap<usize, ZoneCommandSender>> {
    let mut senders = HashMap::new();
    let ctx = Arc::new(ctx);

    for zone in &ctx.config.zones {
        let (cmd_tx, cmd_rx) = mpsc::channel(ZONE_CMD_CHANNEL_SIZE); // zone command backlog
        senders.insert(zone.index, cmd_tx.clone());

        let ctx = ctx.clone();
        let zone_index = zone.index;

        tokio::spawn(async move {
            let mut cmd_rx = cmd_rx;
            while let Err(e) = run(zone_index, &mut cmd_rx, cmd_tx.clone(), ctx.clone()).await {
                tracing::error!(zone = zone_index, error = %e, "Zone player crashed, restarting in 5s");
                tokio::time::sleep(ZONE_RESTART_DELAY).await;
            }
        });

        tracing::info!(zone = %zone.name, "Zone started");
    }

    Ok(senders)
}

/// Stop any active decode task and reset the playback position to zero.
async fn reset_playback(
    current_decode: &mut Option<JoinHandle<()>>,
    decode_rx: &mut Option<mpsc::Receiver<audio::PcmMessage>>,
    position_offset_ms: &mut i64,
) {
    stop_decode(current_decode, decode_rx).await;
    *position_offset_ms = 0;
}

/// Fade out the current stream, stop decode, and prepare fade-in for the next stream.
/// If `fade_ms` is 0 or no stream is active, falls back to immediate reset.
#[allow(clippy::too_many_arguments)]
async fn fade_transition(
    current_decode: &mut Option<JoinHandle<()>>,
    decode_rx: &mut Option<mpsc::Receiver<audio::PcmMessage>>,
    position_offset_ms: &mut i64,
    zone_fade: &mut Option<ZoneFade>,
    fade_ms: u16,
    sample_rate: u32,
    zone_eq: &mut crate::audio::eq::ZoneEq,
    backend: &dyn crate::snapcast::backend::SnapcastBackend,
    zone_index: usize,
    channels: u16,
    resampler: &mut audio::resample::F32Resampling,
) {
    if fade_ms == 0 || decode_rx.is_none() {
        stop_decode(current_decode, decode_rx).await;
        *position_offset_ms = 0;
        return;
    }

    // Drain old stream with fade-out applied
    let mut fade = ZoneFade::new(fade_ms, sample_rate);
    if let Some(rx) = decode_rx.as_mut() {
        let deadline =
            tokio::time::Instant::now() + std::time::Duration::from_millis(u64::from(fade_ms));
        loop {
            let timeout = tokio::time::timeout_at(deadline, rx.recv());
            match timeout.await {
                Ok(Some(audio::PcmMessage::Audio(samples))) => {
                    let mut samples = resampler.process_or_passthrough(samples);
                    zone_eq.process(&mut samples);
                    fade.process(&mut samples, channels);
                    let _ = backend
                        .send_audio(zone_index, &samples, sample_rate, channels)
                        .await;
                    if fade.remaining == 0 {
                        break;
                    }
                }
                _ => break, // Timeout or channel closed
            }
        }
    }

    stop_decode(current_decode, decode_rx).await;
    *position_offset_ms = 0;

    // Prepare fade-in for the next stream
    let mut fade_in = ZoneFade::new(fade_ms, sample_rate);
    fade_in.start_fade_in();
    *zone_fade = Some(fade_in);
}

/// Check if a receiver (AirPlay/Spotify) is active and the source conflict
/// policy blocks local playback. Returns `true` if the command should proceed.
const fn may_start_local_playback(
    source: &ActiveSource,
    policy: crate::config::SourceConflict,
) -> bool {
    if !matches!(source, ActiveSource::AirPlay | ActiveSource::Spotify) {
        return true;
    }
    matches!(policy, crate::config::SourceConflict::LastWins)
}

/// Audio fade state for source transitions within a zone.
struct ZoneFade {
    /// Total fade duration in samples.
    total: u32,
    /// Remaining samples in the current fade.
    remaining: u32,
    /// true = fading out, false = fading in.
    fading_out: bool,
}

impl ZoneFade {
    fn new(duration_ms: u16, sample_rate: u32) -> Self {
        let total = (u64::from(sample_rate) * u64::from(duration_ms) / 1000) as u32;
        Self {
            total,
            remaining: total,
            fading_out: true,
        }
    }

    /// Apply gain ramp to samples. Returns true when fade is complete.
    fn process(&mut self, samples: &mut [f32], channels: u16) -> bool {
        if self.total == 0 || self.remaining == 0 {
            return true;
        }
        let ch = channels as usize;
        for frame in samples.chunks_exact_mut(ch) {
            let gain = snapdog_common::fade_gain(self.remaining, self.total, self.fading_out);
            for sample in frame.iter_mut() {
                *sample *= gain;
            }
            self.remaining = self.remaining.saturating_sub(1);
            if self.remaining == 0 {
                break;
            }
        }
        self.remaining == 0
    }

    /// Switch from fade-out to fade-in (resets remaining to total).
    const fn start_fade_in(&mut self) {
        self.fading_out = false;
        self.remaining = self.total;
    }
}

/// Main `ZonePlayer` loop.
#[allow(clippy::too_many_lines)] // Main event loop — splitting would obscure control flow
async fn run(
    zone_index: usize,
    commands: &mut mpsc::Receiver<ZoneCommand>,
    self_tx: mpsc::Sender<ZoneCommand>,
    ctx: Arc<ZonePlayerContext>,
) -> Result<()> {
    const TIMER_INACTIVE: std::time::Duration = std::time::Duration::from_secs(86400);

    let config = &ctx.config;
    let store = &ctx.store;
    let covers = &ctx.covers;
    let notify = &ctx.notify;
    let zone_config = &config.zones[zone_index - 1];
    let audio_config = config.audio.clone(); // Cloned once, moved into decode tasks

    let backend = &ctx.backend;

    // Zone grouping: assign clients to group, set stream
    let group_id = setup_zone_group(zone_index, &ctx).await;
    {
        let mut s = store.write().await;
        if let Some(zone) = s.zones.get_mut(&zone_index) {
            zone.snapcast_group_id.clone_from(&group_id);
        }
    }

    // Subsonic client (if configured)
    let subsonic = config.subsonic.as_ref().map(SubsonicClient::new);

    // Track cache (if subsonic + cache enabled)
    let track_cache = config.subsonic.as_ref().and_then(|s| {
        crate::audio::cache::TrackCache::new(&s.cache)
            .map_err(|e| tracing::warn!(error = %e, "Failed to initialize track cache"))
            .ok()
    });

    // AirPlay: F32 audio channel + event channel + receiver instance
    let (airplay_audio_tx, mut airplay_audio_rx) =
        crate::receiver::audio_channel(RECEIVER_AUDIO_CHANNEL_SIZE);
    let (airplay_event_tx, mut airplay_event_rx) =
        mpsc::channel::<crate::receiver::ReceiverEvent>(RECEIVER_EVENT_CHANNEL_SIZE);
    let mut _airplay_receiver = {
        let ap_config = crate::config::AirplayConfig {
            password: config.airplay.password.clone(),
            mode: config.airplay.mode,
            bind: config.airplay.bind.clone(),
        };
        let mut receiver = crate::receiver::airplay::AirPlayReceiver::new(
            ap_config,
            zone_index,
            zone_config.airplay_name.clone(),
            #[cfg(feature = "ap2")]
            config.system.state_dir.clone(),
        );
        match receiver.start(airplay_audio_tx, airplay_event_tx).await {
            Ok(()) => Some(receiver),
            Err(e) => {
                tracing::warn!(zone = zone_index, error = %e, "AirPlay receiver failed to start");
                None
            }
        }
    };

    // Spotify Connect: F32 audio channel + event channel + receiver instance
    #[cfg(feature = "spotify")]
    let (mut spotify_audio_rx, mut spotify_event_rx, mut _spotify_receiver) = {
        let (audio_tx, audio_rx) = crate::receiver::audio_channel(RECEIVER_AUDIO_CHANNEL_SIZE);
        let (event_tx, event_rx) =
            mpsc::channel::<crate::receiver::ReceiverEvent>(RECEIVER_EVENT_CHANNEL_SIZE);
        let receiver = if let Some(ref sp_config) = config.spotify {
            let mut sp = crate::receiver::spotify::SpotifyReceiver::new(
                crate::config::SpotifyConfig {
                    name: zone_config.name.clone(),
                    bitrate: sp_config.bitrate,
                },
                zone_index,
            );
            match sp.start(audio_tx, event_tx).await {
                Ok(()) => Some(sp),
                Err(e) => {
                    tracing::warn!(zone = zone_index, error = %e, "Spotify receiver failed to start");
                    None
                }
            }
        } else {
            None
        };
        (audio_rx, event_rx, receiver)
    };
    #[cfg(not(feature = "spotify"))]
    let (mut spotify_audio_rx, mut spotify_event_rx) = {
        let (_tx1, rx1) = crate::receiver::audio_channel(1);
        let (_tx2, rx2) = mpsc::channel::<crate::receiver::ReceiverEvent>(1);
        (rx1, rx2)
    };

    // Decode task state
    let mut current_decode: Option<JoinHandle<()>> = None;
    let mut current_cover: Option<JoinHandle<()>> = None;
    let mut decode_rx: Option<mpsc::Receiver<audio::PcmMessage>> = None;
    let mut source = ActiveSource::Idle;
    let mut remote_control: Option<std::sync::Arc<dyn crate::receiver::RemoteControl>> = None;
    let mut position_offset_ms: i64 = 0;
    let mut last_buffer_progress = std::time::Instant::now();
    let mut zone_fade: Option<ZoneFade> = None;
    let mut resampler = audio::resample::F32Resampling::new(
        config.audio.sample_rate,
        config.audio.sample_rate,
        config.audio.channels,
    );
    let mut receiver_resampler: Option<audio::resample::F32Resampling> = None;

    // Per-zone EQ
    let mut zone_eq = audio::eq::ZoneEq::new(config.audio.sample_rate, config.audio.channels);
    {
        let eq_config = ctx
            .eq_store
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(zone_index);
        zone_eq.set_config(&eq_config);
    }

    // Presence auto-off timer (inactive until armed)
    let auto_off_timer = tokio::time::sleep(TIMER_INACTIVE);
    tokio::pin!(auto_off_timer);
    let mut auto_off_armed = false;

    /// Handle audio samples from an external receiver (AirPlay/Spotify).
    macro_rules! handle_receiver_audio {
        ($samples:expr, $active:path, $source_type:expr, $label:literal) => {{
            if !matches!(source, $active) {
                reset_playback(&mut current_decode, &mut decode_rx, &mut position_offset_ms).await;
                source = $active;
                update_and_notify(store, zone_index, notify, |z| { z.playback = PlaybackState::Playing; z.source = $source_type; }).await;
            }
            let mut samples = match &mut receiver_resampler {
                Some(r) => match r.process(&$samples) { Some(resampled) => resampled, None => continue },
                None => $samples,
            };
            zone_eq.process(&mut samples);
            if let Err(e) = backend.send_audio(zone_index, &samples, config.audio.sample_rate, config.audio.channels).await {
                tracing::error!(zone = zone_index, error = %e, concat!("Audio send failed (", $label, ")"));
            }
        }};
    }

    /// Handle events from an external receiver (AirPlay/Spotify).
    macro_rules! handle_receiver_event {
        ($event:expr, $active:path, $source_type:expr) => {{
            use crate::receiver::ReceiverEvent;
            match $event {
                ReceiverEvent::SessionStarted { format } => {
                    receiver_resampler = Some(audio::resample::F32Resampling::new(
                        format.sample_rate,
                        config.audio.sample_rate,
                        format.channels,
                    ));
                }
                ReceiverEvent::Metadata {
                    title,
                    artist,
                    album,
                } => {
                    update_and_notify(store, zone_index, notify, |z| {
                        z.track = Some(TrackInfo {
                            title,
                            artist,
                            album,
                            album_artist: None,
                            genre: None,
                            year: None,
                            track_number: None,
                            disc_number: None,
                            duration_ms: 0,
                            position_ms: 0,
                            seekable: false,
                            source: $source_type,
                            bitrate_kbps: None,
                            content_type: None,
                            sample_rate: None,
                        });
                    })
                    .await;
                }
                ReceiverEvent::CoverArt { bytes } => {
                    let mut cache = covers.write().await;
                    cache.set_auto_mime(zone_index, bytes);
                    let hash = cache.get(zone_index).map(|e| e.hash.clone());
                    drop(cache);
                    if let Some(h) = hash {
                        let base = config.http.base_url.trim_end_matches('/');
                        let url = format!("{base}/api/v1/zones/{zone_index}/cover?h={h}");
                        update_and_notify(store, zone_index, notify, |z| {
                            z.cover_url = Some(url.clone());
                        })
                        .await;
                    }
                }
                ReceiverEvent::Progress {
                    position_ms,
                    duration_ms,
                } => {
                    update_and_notify(store, zone_index, notify, |z| {
                        if let Some(ref mut t) = z.track {
                            #[allow(clippy::cast_possible_wrap)]
                            {
                                t.position_ms = position_ms as i64;
                                t.duration_ms = duration_ms as i64;
                            }
                        }
                    })
                    .await;
                }
                ReceiverEvent::Volume { percent } => {
                    update_and_notify(store, zone_index, notify, |z| z.volume = percent).await;
                    let gid = store
                        .read()
                        .await
                        .zones
                        .get(&zone_index)
                        .and_then(|z| z.snapcast_group_id.clone());
                    if let Some(gid) = gid {
                        let _ = ctx
                            .snap_tx
                            .send(SnapcastCmd::Group {
                                group_id: gid,
                                action: GroupAction::Volume(percent),
                            })
                            .await;
                    }
                }
                ReceiverEvent::RemoteAvailable { remote } => {
                    remote_control = Some(remote);
                }
                ReceiverEvent::SessionEnded => {
                    if matches!(source, $active) {
                        source = ActiveSource::Idle;
                        remote_control = None;
                        covers.write().await.clear(zone_index);
                        update_and_notify(store, zone_index, notify, |z| {
                            z.playback = PlaybackState::Stopped;
                            z.source = SourceType::Idle;
                            z.track = None;
                            z.cover_url = None;
                            z.buffered_ms = None;
                        })
                        .await;
                    }
                }
            }
        }};
    }

    loop {
        tokio::select! {
            Some(cmd) = commands.recv() => {
                match cmd {
                    ZoneCommand::PlaySubsonicPlaylist(playlist_id, track_idx) => {
                        if !may_start_local_playback(&source, config.audio.source_conflict) {
                            tracing::info!(zone = zone_index, "Playback blocked: receiver has priority");
                            continue;
                        }
                        fade_transition(&mut current_decode, &mut decode_rx, &mut position_offset_ms, &mut zone_fade, config.audio.source_switch_fade_ms, config.audio.sample_rate, &mut zone_eq, ctx.backend.as_ref(), zone_index, config.audio.channels, &mut resampler).await;
                        if let Some(sub) = &subsonic {
                            let playlist_entries = sub.get_playlists().await.unwrap_or_default();
                            let playlist_index = playlist_entries
                                .iter()
                                .position(|p| p.id == playlist_id)
                                .map(|sub_idx| if config.has_radio_playlist() { sub_idx + 1 } else { sub_idx });
                            let playlist_total = config.unified_playlist_count(playlist_entries.len());
                            if let Ok(playlist) = sub.get_playlist(&playlist_id).await {
                                let track_count = playlist.entry.len();
                                let track_idx = track_idx.min(track_count.saturating_sub(1));
                                if let Some(track) = playlist.entry.get(track_idx) {
                                    start_subsonic_track_decode(sub, track, &mut DecodeState { current_decode: &mut current_decode, current_cover: &mut current_cover, decode_rx: &mut decode_rx, source: &mut source }, &PlaybackCtx { config, subsonic: &subsonic, store, zone_index, notify, covers, track_cache: &track_cache }).await;
                                    source = ActiveSource::SubsonicPlaylist { playlist_id, track_index: track_idx, track_count };
                                    update_and_notify(store, zone_index, notify, |z| {
                                        z.playback = PlaybackState::Playing;
                                        z.source = SourceType::SubsonicPlaylist;
                                        z.playlist_index = playlist_index;
                                        z.playlist_name = Some(playlist.name.clone());
                                        z.playlist_total = Some(playlist_total);
                                        z.playlist_track_index = Some(track_idx);
                                        z.playlist_track_count = Some(track_count);
                                        z.track = Some(subsonic_track_info(track));
                                    }).await;
                                    // Prefetch next tracks
                                    if let Some(ref cache) = track_cache {
                                        let lookahead = crate::config::CACHE_LOOKAHEAD;
                                        prefetch_next_tracks(sub, &playlist.entry, track_idx, cache, lookahead);
                                    }
                                }
                            }
                        }
                    }
                    ZoneCommand::PlaySubsonicTrack(track_id) => {
                        if !may_start_local_playback(&source, config.audio.source_conflict) {
                            tracing::info!(zone = zone_index, "Playback blocked: receiver has priority");
                            continue;
                        }
                        fade_transition(&mut current_decode, &mut decode_rx, &mut position_offset_ms, &mut zone_fade, config.audio.source_switch_fade_ms, config.audio.sample_rate, &mut zone_eq, ctx.backend.as_ref(), zone_index, config.audio.channels, &mut resampler).await;
                        if let Some(sub) = &subsonic {
                            let url = sub.stream_url(&track_id);
                            let (tx, rx) = audio::pcm_channel(PCM_DECODE_CHANNEL_SIZE);
                            decode_rx = Some(rx);
                            let ac = audio_config.clone();
                            current_decode = Some(tokio::spawn(async move {
                                if let Err(e) = audio::decode_http_stream(url, tx, ac, None).await {
                                    tracing::error!(error = %e, "Subsonic track decode failed");
                                }
                            }));
                            source = ActiveSource::SubsonicTrack { track_id };
                            update_and_notify(store, zone_index, notify, |z| { z.playback = PlaybackState::Playing; z.source = SourceType::SubsonicTrack; }).await;
                        }
                    }
                    ZoneCommand::PlayUrl(url) => {
                        if !may_start_local_playback(&source, config.audio.source_conflict) {
                            tracing::info!(zone = zone_index, "Playback blocked: receiver has priority");
                            continue;
                        }
                        fade_transition(&mut current_decode, &mut decode_rx, &mut position_offset_ms, &mut zone_fade, config.audio.source_switch_fade_ms, config.audio.sample_rate, &mut zone_eq, ctx.backend.as_ref(), zone_index, config.audio.channels, &mut resampler).await;
                        let (tx, rx) = audio::pcm_channel(PCM_DECODE_CHANNEL_SIZE);
                        decode_rx = Some(rx);
                        let ac = audio_config.clone();
                        let u = url.clone();
                        current_decode = Some(tokio::spawn(async move {
                            if let Err(e) = audio::decode_http_stream(u, tx, ac, None).await { tracing::error!(error = %e, "URL decode failed"); }
                        }));
                        source = ActiveSource::Url;
                        update_and_notify(store, zone_index, notify, |z| { z.playback = PlaybackState::Playing; z.source = SourceType::Url; }).await;
                    }
                    ZoneCommand::SetTrack(track_idx) => {
                        if let ActiveSource::Radio { .. } = source {
                            if track_idx < config.radios.len() {
                                if let Some(radio) = config.radios.get(track_idx) {
                                    reset_playback(&mut current_decode, &mut decode_rx, &mut position_offset_ms).await;
                                    start_radio_decode(radio, &mut DecodeState { current_decode: &mut current_decode, current_cover: &mut current_cover, decode_rx: &mut decode_rx, source: &mut source }, &PlaybackCtx { config, subsonic: &subsonic, store, zone_index, notify, covers, track_cache: &track_cache }).await;
                                    source = ActiveSource::Radio { index: track_idx };
                                    update_and_notify(store, zone_index, notify, |z| {
                                        z.playback = PlaybackState::Playing;
                                        z.source = SourceType::Radio;
                                        z.playlist_index = Some(0);
                                        z.playlist_name = Some("Radio".into());
                                        z.playlist_track_count = Some(config.radios.len());
                                        z.playlist_track_index = Some(track_idx);
                                        z.track = Some(radio_track_info(&radio.name));
                                    }).await;
                                    tracing::info!(zone = %zone_config.name, radio = %radio.name, "Radio set");
                                }
                            }
                        } else if let ActiveSource::SubsonicPlaylist { ref playlist_id, track_count, .. } = source {
                            if track_idx < track_count {
                                let pid = playlist_id.clone();
                                reset_playback(&mut current_decode, &mut decode_rx, &mut position_offset_ms).await;
                                if let Some(sub) = &subsonic {
                                    if let Ok(playlist) = sub.get_playlist(&pid).await {
                                        if let Some(track) = playlist.entry.get(track_idx) {
                                            start_subsonic_track_decode(sub, track, &mut DecodeState { current_decode: &mut current_decode, current_cover: &mut current_cover, decode_rx: &mut decode_rx, source: &mut source }, &PlaybackCtx { config, subsonic: &subsonic, store, zone_index, notify, covers, track_cache: &track_cache }).await;
                                            source = ActiveSource::SubsonicPlaylist { playlist_id: pid, track_index: track_idx, track_count };
                                            update_and_notify(store, zone_index, notify, |z| {
                                                z.playback = PlaybackState::Playing;
                                                z.source = SourceType::SubsonicPlaylist;
                                                z.playlist_track_index = Some(track_idx);
                                                z.track = Some(subsonic_track_info(track));
                                            }).await;
                                        }
                                    }
                                }
                            }
                        }
                    }
                    ZoneCommand::Play => {
                        if matches!(source, ActiveSource::AirPlay | ActiveSource::Spotify) {
                            if let Some(ref rc) = remote_control {
                                if let Err(e) = rc.send_command(crate::receiver::RemoteCommand::Play) { tracing::warn!(error = %e, "Remote play failed"); }
                            }
                        } else if matches!(source, ActiveSource::SubsonicPlaylist { .. } | ActiveSource::SubsonicTrack { .. }) {
                            // Resume Subsonic from last position
                            if let Some(sub) = &subsonic {
                                let track_id = match &source {
                                    ActiveSource::SubsonicPlaylist { playlist_id, track_index, .. } => {
                                        sub.get_playlist(playlist_id).await.ok()
                                            .and_then(|p| p.entry.get(*track_index).map(|t| t.id.clone()))
                                    }
                                    ActiveSource::SubsonicTrack { track_id } => Some(track_id.clone()),
                                    _ => None,
                                };
                                if let Some(tid) = track_id {
                                    let pos_ms = store.read().await.zones.get(&zone_index)
                                        .and_then(|z| z.track.as_ref().map(|t| t.position_ms))
                                        .unwrap_or(0);
                                    stop_decode(&mut current_decode, &mut decode_rx).await;
                                    let (tx, rx) = audio::pcm_channel(PCM_DECODE_CHANNEL_SIZE);
                                    decode_rx = Some(rx);

                                    // Try resuming from cached file
                                    let cached_path = track_cache.as_ref().and_then(|cache| {
                                        if let audio::cache::CacheEntry::Complete { path, content_type } = cache.get(&tid) {
                                            Some((path, content_type))
                                        } else { None }
                                    });

                                    let used_cache = if let Some((path, content_type)) = cached_path {
                                        let seek = Some(pos_ms);
                                        let inv_cache = track_cache.clone();
                                        let inv_tid = tid.clone();
                                        current_decode = Some(tokio::spawn(async move {
                                            if let Err(e) = tokio::task::spawn_blocking(move || {
                                                audio::decode_cached_file(&path, &content_type, seek, &tx)
                                            }).await.unwrap_or_else(|e| Err(e.into())) {
                                                tracing::error!(error = %e, "Cached resume failed");
                                                if let Some(cache) = inv_cache {
                                                    cache.invalidate(&inv_tid);
                                                }
                                            }
                                        }));
                                        true
                                    } else {
                                        let offset_secs = (pos_ms / 1000).max(0) as u64;
                                        let url = sub.stream_url_with_offset(&tid, offset_secs);
                                        let ac = audio_config.clone();
                                        current_decode = Some(tokio::spawn(async move {
                                            if let Err(e) = audio::decode_http_stream(url, tx, ac, None).await { tracing::error!(error = %e, "Resume decode failed"); }
                                        }));
                                        false
                                    };

                                    position_offset_ms = if used_cache { 0 } else { pos_ms };
                                    update_and_notify(store, zone_index, notify, |z| { z.playback = PlaybackState::Playing; }).await;
                                }
                            }
                        } else if matches!(source, ActiveSource::Radio { .. } | ActiveSource::Idle) {
                            // Resume or start radio
                            let z_state = store.read().await;
                            let radio_idx = match &source {
                                ActiveSource::Radio { index } => *index,
                                _ => z_state.zones.get(&zone_index).and_then(|z| {
                                    if z.playlist_index == Some(0) { z.playlist_track_index } else { None }
                                }).unwrap_or(0),
                            };
                            drop(z_state);
                            if let Some(radio) = config.radios.get(radio_idx) {
                                reset_playback(&mut current_decode, &mut decode_rx, &mut position_offset_ms).await;
                                start_radio_decode(radio, &mut DecodeState { current_decode: &mut current_decode, current_cover: &mut current_cover, decode_rx: &mut decode_rx, source: &mut source }, &PlaybackCtx { config, subsonic: &subsonic, store, zone_index, notify, covers, track_cache: &track_cache }).await;
                                source = ActiveSource::Radio { index: radio_idx };
                                update_and_notify(store, zone_index, notify, |z| {
                                    z.playback = PlaybackState::Playing;
                                    z.source = SourceType::Radio;
                                    z.playlist_index = Some(0);
                                    z.playlist_name = Some("Radio".into());
                                    z.playlist_track_index = Some(radio_idx);
                                    z.playlist_track_count = Some(config.radios.len());
                                    z.track = Some(radio_track_info(&radio.name));
                                }).await;
                            }
                        }
                    }
                    ZoneCommand::Pause => {
                        if matches!(source, ActiveSource::AirPlay | ActiveSource::Spotify) {
                            if let Some(ref rc) = remote_control {
                                if let Err(e) = rc.send_command(crate::receiver::RemoteCommand::Pause) { tracing::warn!(error = %e, "Remote pause failed"); }
                            }
                        } else {
                            reset_playback(&mut current_decode, &mut decode_rx, &mut position_offset_ms).await;
                            update_and_notify(store, zone_index, notify, |z| { z.playback = PlaybackState::Paused; }).await;
                        }
                    }
                    ZoneCommand::Stop => {
                        reset_playback(&mut current_decode, &mut decode_rx, &mut position_offset_ms).await;
                        source = ActiveSource::Idle;
                        update_and_notify(store, zone_index, notify, |z| { z.playback = PlaybackState::Stopped; z.source = SourceType::Idle; z.track = None; z.cover_url = None; z.buffered_ms = None; }).await;
                    }
                    ZoneCommand::Next => {
                        if matches!(source, ActiveSource::AirPlay | ActiveSource::Spotify) {
                            if let Some(ref rc) = remote_control { let _ = rc.send_command(crate::receiver::RemoteCommand::NextTrack); }
                        } else {
                            handle_next(&mut DecodeState { current_decode: &mut current_decode, current_cover: &mut current_cover, decode_rx: &mut decode_rx, source: &mut source }, &PlaybackCtx { config, subsonic: &subsonic, store, zone_index, notify, covers, track_cache: &track_cache }).await;
                        }
                    }
                    ZoneCommand::Previous => {
                        if matches!(source, ActiveSource::AirPlay | ActiveSource::Spotify) {
                            if let Some(ref rc) = remote_control { let _ = rc.send_command(crate::receiver::RemoteCommand::PreviousTrack); }
                        } else {
                            handle_previous(&mut DecodeState { current_decode: &mut current_decode, current_cover: &mut current_cover, decode_rx: &mut decode_rx, source: &mut source }, &PlaybackCtx { config, subsonic: &subsonic, store, zone_index, notify, covers, track_cache: &track_cache }).await;
                        }
                    }
                    ZoneCommand::NextPlaylist | ZoneCommand::PreviousPlaylist | ZoneCommand::SetPlaylist(..) | ZoneCommand::PlayPlaylist(..) => {
                        if !may_start_local_playback(&source, config.audio.source_conflict) {
                            tracing::info!(zone = zone_index, "Playback blocked: receiver has priority");
                            continue;
                        }

                        // Unified playlist model: index 0 = radio (from config), index 1+ = Subsonic playlists
                        let subsonic_playlists = if let Some(sub) = &subsonic {
                            sub.get_playlists().await.unwrap_or_default()
                        } else {
                            vec![]
                        };
                        let total_count = config.unified_playlist_count(subsonic_playlists.len());
                        if total_count == 0 {
                            tracing::warn!(zone = zone_index, "No playlists available");
                            continue;
                        }

                        let has_radio = config.has_radio_playlist();
                        let current_unified = match &source {
                            ActiveSource::SubsonicPlaylist { playlist_id, .. } => {
                                let sub_idx = subsonic_playlists.iter().position(|p| p.id == *playlist_id).unwrap_or(0);
                                if has_radio { sub_idx + 1 } else { sub_idx }
                            }
                            _ => 0,
                        };

                        let force_play = matches!(&cmd, ZoneCommand::PlayPlaylist(..));
                        let (target_unified, start_track) = match cmd {
                            ZoneCommand::NextPlaylist => ((current_unified + 1) % total_count, 0),
                            ZoneCommand::PreviousPlaylist => (if current_unified == 0 { total_count - 1 } else { current_unified - 1 }, 0),
                            ZoneCommand::SetPlaylist(i, t) | ZoneCommand::PlayPlaylist(i, t) => (i.min(total_count - 1), t),
                            _ => continue,
                        };

                        let should_start = force_play || store.read().await.zones.get(&zone_index)
                            .is_some_and(|z| z.playback == PlaybackState::Playing);

                        match config.resolve_playlist_index(target_unified, subsonic_playlists.len()) {
                            Some(crate::config::ResolvedPlaylist::Radio) => {
                                let radio_idx = start_track.min(config.radios.len().saturating_sub(1));
                                if let Some(radio) = config.radios.get(radio_idx) {
                                    reset_playback(&mut current_decode, &mut decode_rx, &mut position_offset_ms).await;
                                    if should_start {
                                        start_radio_decode(radio, &mut DecodeState { current_decode: &mut current_decode, current_cover: &mut current_cover, decode_rx: &mut decode_rx, source: &mut source }, &PlaybackCtx { config, subsonic: &subsonic, store, zone_index, notify, covers, track_cache: &track_cache }).await;
                                    }
                                    source = ActiveSource::Radio { index: radio_idx };
                                    update_and_notify(store, zone_index, notify, |z| {
                                        z.playback = if should_start { PlaybackState::Playing } else { PlaybackState::Stopped };
                                        z.source = SourceType::Radio;
                                        z.playlist_index = Some(target_unified);
                                        z.playlist_name = Some("Radio".into());
                                        z.playlist_total = Some(total_count);
                                        z.playlist_track_index = Some(radio_idx);
                                        z.playlist_track_count = Some(config.radios.len());
                                        z.track = Some(radio_track_info(&radio.name));
                                    }).await;
                                    if should_start {
                                        tracing::info!(zone = %zone_config.name, radio = %radio.name, "Radio playlist started");
                                    } else {
                                        tracing::info!(zone = %zone_config.name, radio = %radio.name, "Radio playlist selected (stopped)");
                                    }
                                }
                            }
                            Some(crate::config::ResolvedPlaylist::Subsonic(sub_idx)) => {
                                if let Some(sub) = &subsonic {
                                    if let Some(pl) = subsonic_playlists.get(sub_idx) {
                                        let playlist_id = pl.id.clone();
                                        let playlist_name = pl.name.clone();
                                        match sub.get_playlist(&playlist_id).await {
                                            Ok(playlist) => {
                                                let track_count = playlist.entry.len();
                                                let track_idx = if track_count == 0 {
                                                    0
                                                } else {
                                                    start_track.min(track_count - 1)
                                                };
                                                let track = playlist.entry.get(track_idx);
                                                if should_start {
                                                    reset_playback(&mut current_decode, &mut decode_rx, &mut position_offset_ms).await;
                                                    if let Some(track) = track {
                                                        start_subsonic_track_decode(sub, track, &mut DecodeState { current_decode: &mut current_decode, current_cover: &mut current_cover, decode_rx: &mut decode_rx, source: &mut source }, &PlaybackCtx { config, subsonic: &subsonic, store, zone_index, notify, covers, track_cache: &track_cache }).await;
                                                    }
                                                }
                                                source = ActiveSource::SubsonicPlaylist {
                                                    playlist_id,
                                                    track_index: track_idx,
                                                    track_count,
                                                };
                                                update_and_notify(store, zone_index, notify, |z| {
                                                    z.playback = if should_start && track.is_some() { PlaybackState::Playing } else { PlaybackState::Stopped };
                                                    z.source = SourceType::SubsonicPlaylist;
                                                    z.playlist_index = Some(target_unified);
                                                    z.playlist_name = Some(playlist.name.clone());
                                                    z.playlist_total = Some(total_count);
                                                    z.playlist_track_index = Some(track_idx);
                                                    z.playlist_track_count = Some(track_count);
                                                    z.track = track.map(subsonic_track_info);
                                                }).await;
                                                if should_start && track.is_some() {
                                                    if let Some(ref cache) = track_cache {
                                                        prefetch_next_tracks(sub, &playlist.entry, track_idx, cache, crate::config::CACHE_LOOKAHEAD);
                                                    }
                                                    tracing::info!(zone = %zone_config.name, playlist = %playlist.name, "Playlist switched (playing)");
                                                } else {
                                                    tracing::info!(zone = %zone_config.name, playlist = %playlist.name, "Playlist selected (stopped)");
                                                }
                                            }
                                            Err(e) => {
                                                tracing::warn!(zone = %zone_config.name, playlist = %playlist_name, error = %e, "Playlist fetch failed");
                                            }
                                        }
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                    ZoneCommand::Seek(pos_ms) => {
                        if let Some(sub) = &subsonic {
                            let track_id = match &source {
                                ActiveSource::SubsonicTrack { track_id } => Some(track_id.clone()),
                                ActiveSource::SubsonicPlaylist { playlist_id, track_index, .. } => {
                                    sub.get_playlist(playlist_id).await.ok()
                                        .and_then(|p| p.entry.get(*track_index).map(|t| t.id.clone()))
                                }
                                _ => None,
                            };
                            if let Some(tid) = track_id {
                                stop_decode(&mut current_decode, &mut decode_rx).await;
                                let (tx, rx) = audio::pcm_channel(PCM_DECODE_CHANNEL_SIZE);
                                decode_rx = Some(rx);

                                // Try seeking in cached file first
                                let cached_path = track_cache.as_ref().and_then(|cache| {
                                    if let audio::cache::CacheEntry::Complete { path, content_type } = cache.get(&tid) {
                                        Some((path, content_type))
                                    } else { None }
                                });

                                let used_cache = if let Some((path, content_type)) = cached_path {
                                    let seek = Some(pos_ms);
                                    let inv_cache = track_cache.clone();
                                    let inv_tid = tid.clone();
                                    current_decode = Some(tokio::spawn(async move {
                                        if let Err(e) = tokio::task::spawn_blocking(move || {
                                            audio::decode_cached_file(&path, &content_type, seek, &tx)
                                        }).await.unwrap_or_else(|e| Err(e.into())) {
                                            tracing::error!(error = %e, "Cached seek decode failed");
                                            if let Some(cache) = inv_cache {
                                                cache.invalidate(&inv_tid);
                                            }
                                        }
                                    }));
                                    true
                                } else {
                                    // Fallback: re-fetch with timeOffset
                                    let offset_secs = (pos_ms / 1000).max(0) as u64;
                                    let url = sub.stream_url_with_offset(&tid, offset_secs);
                                    let ac = audio_config.clone();
                                    current_decode = Some(tokio::spawn(async move {
                                        if let Err(e) = audio::decode_http_stream(url, tx, ac, None).await {
                                            tracing::error!(error = %e, "Seek decode failed");
                                        }
                                    }));
                                    false
                                };

                                update_and_notify(store, zone_index, notify, |z| {
                                    z.playback = PlaybackState::Playing;
                                    if let Some(ref mut t) = z.track { t.position_ms = pos_ms; }
                                }).await;
                                // When decoding from cache, symphonia reports absolute position
                                position_offset_ms = if used_cache { 0 } else { pos_ms };
                                tracing::debug!(zone = %zone_config.name, position_ms = pos_ms, "Seeked");
                            }
                        }
                    }
                    ZoneCommand::SeekProgress(progress) => {
                        let duration = store.read().await.zones.get(&zone_index)
                            .and_then(|z| z.track.as_ref().map(|t| t.duration_ms)).unwrap_or(0);
                        if duration > 0 {
                            let pos_ms = (progress.clamp(0.0, 1.0) * duration as f64) as i64;
                            let _ = self_tx.send(ZoneCommand::Seek(pos_ms)).await;
                        }
                    }
                    ZoneCommand::SeekRelative(offset_ms) => {
                        let (pos, dur) = store.read().await.zones.get(&zone_index)
                            .and_then(|z| z.track.as_ref().map(|t| (t.position_ms, t.duration_ms)))
                            .unwrap_or((0, 0));
                        if dur > 0 {
                            let target = (pos + offset_ms).clamp(0, dur);
                            let _ = self_tx.send(ZoneCommand::Seek(target)).await;
                        }
                    }
                    ZoneCommand::SetVolume(v) => {
                        update_volume_and_notify(store, zone_index, notify, |z| z.volume = v.clamp(0, 100)).await;
                        let gid = store.read().await.zones.get(&zone_index).and_then(|z| z.snapcast_group_id.clone());
                        if let Some(gid) = gid {
                            let _ = ctx.snap_tx.send(SnapcastCmd::Group { group_id: gid, action: GroupAction::Volume(v) }).await;
                        }
                    }
                    ZoneCommand::AdjustVolume(delta) => {
                        let new_vol = {
                            let s = store.read().await;
                            s.zones.get(&zone_index).map_or(crate::state::DEFAULT_VOLUME, |z| (z.volume + delta).clamp(0, 100))
                        };
                        update_volume_and_notify(store, zone_index, notify, |z| z.volume = new_vol).await;
                        let gid = store.read().await.zones.get(&zone_index).and_then(|z| z.snapcast_group_id.clone());
                        if let Some(gid) = gid {
                            let _ = ctx.snap_tx.send(SnapcastCmd::Group { group_id: gid, action: GroupAction::Volume(new_vol) }).await;
                        }
                    }
                    ZoneCommand::SetMute(m) => {
                        let gid = store.read().await.zones.get(&zone_index).and_then(|z| z.snapcast_group_id.clone());
                        if let Some(gid) = gid {
                            let _ = ctx.snap_tx.send(SnapcastCmd::Group { group_id: gid.clone(), action: GroupAction::Mute(m) }).await;
                        }
                    }
                    ZoneCommand::ToggleMute => {
                        let muted = { store.read().await.zones.get(&zone_index).is_some_and(|z| !z.muted) };
                        let gid = store.read().await.zones.get(&zone_index).and_then(|z| z.snapcast_group_id.clone());
                        if let Some(gid) = gid {
                            let _ = ctx.snap_tx.send(SnapcastCmd::Group { group_id: gid.clone(), action: GroupAction::Mute(muted) }).await;
                        }
                    }
                    ZoneCommand::SetShuffle(v) => { update_and_notify(store, zone_index, notify, |z| z.shuffle = v).await; }
                    ZoneCommand::ToggleShuffle => { update_and_notify(store, zone_index, notify, |z| z.shuffle = !z.shuffle).await; }
                    ZoneCommand::SetRepeat(mode) => { update_and_notify(store, zone_index, notify, |z| z.repeat = mode).await; }
                    ZoneCommand::CycleRepeat => {
                        update_and_notify(store, zone_index, notify, |z| {
                            z.repeat = match z.repeat {
                                snapdog_common::RepeatMode::Off => snapdog_common::RepeatMode::Playlist,
                                snapdog_common::RepeatMode::Playlist => snapdog_common::RepeatMode::Track,
                                snapdog_common::RepeatMode::Track => snapdog_common::RepeatMode::Off,
                            };
                        }).await;
                    }
                    ZoneCommand::SetEq(eq_config) => {
                        zone_eq.set_config(&eq_config);
                        ctx.eq_store.lock().unwrap_or_else(std::sync::PoisonError::into_inner).set(zone_index, eq_config.clone());
                        crate::api::ws::broadcast_notification(notify, &crate::api::ws::Notification::ZoneEqChanged {
                            zone: zone_index,
                            config: eq_config,
                        });
                        tracing::debug!(zone = zone_index, "EQ updated");
                    }
                    ZoneCommand::SetPresence(present) => {
                        let enabled = store.read().await.zones.get(&zone_index).is_some_and(|z| z.presence_enabled);
                        if !enabled {
                            update_and_notify(store, zone_index, notify, |z| z.presence = present).await;
                        } else if present {
                            auto_off_armed = false;
                            let is_idle = store.read().await.zones.get(&zone_index).is_some_and(|z| z.playback == crate::state::PlaybackState::Stopped);
                            update_and_notify(store, zone_index, notify, |z| {
                                z.presence = true;
                                z.auto_off_active = false;
                            }).await;
                            if is_idle {
                                // Resolve source: schedule → default → resume
                                let resolved = resolve_presence_source(config, zone_index);
                                match resolved {
                                    Some(crate::config::PresenceSource::Radio(idx)) => {
                                        update_and_notify(store, zone_index, notify, |z| z.presence_source = true).await;
                                        // Unified index 0 = radio, idx = station within radio
                                        let _ = self_tx.send(ZoneCommand::PlayPlaylist(0, idx)).await;
                                        tracing::info!(zone = zone_index, "Presence: playback started");
                                    }
                                    Some(crate::config::PresenceSource::Playlist(ref id)) => {
                                        update_and_notify(store, zone_index, notify, |z| z.presence_source = true).await;
                                        let _ = self_tx.send(ZoneCommand::PlaySubsonicPlaylist(id.clone(), 0)).await;
                                        tracing::info!(zone = zone_index, "Presence: playback started");
                                    }
                                    Some(crate::config::PresenceSource::None) => {
                                        update_and_notify(store, zone_index, notify, |z| z.presence_source = false).await;
                                    }
                                    None => {
                                        update_and_notify(store, zone_index, notify, |z| z.presence_source = true).await;
                                        let _ = self_tx.send(ZoneCommand::Play).await;
                                        tracing::info!(zone = zone_index, "Presence: playback started");
                                    }
                                }
                            }
                        } else {
                            let should_timer = store.read().await.zones.get(&zone_index).is_some_and(|z| z.presence_source && z.playback == crate::state::PlaybackState::Playing);
                            update_and_notify(store, zone_index, notify, |z| z.presence = false).await;
                            if should_timer {
                                let delay = store.read().await.zones.get(&zone_index).map_or(crate::config::DEFAULT_AUTO_OFF_DELAY, |z| z.auto_off_delay);
                                auto_off_timer.as_mut().reset(tokio::time::Instant::now() + std::time::Duration::from_secs(u64::from(delay))); auto_off_armed = true;
                                update_and_notify(store, zone_index, notify, |z| z.auto_off_active = true).await;
                                tracing::debug!(zone = zone_index, delay, "Presence auto-off timer started");
                            }
                        }
                        notify_presence(store, zone_index, notify).await;
                    }
                    ZoneCommand::SetPresenceEnabled(v) => {
                        if v {
                            update_and_notify(store, zone_index, notify, |z| z.presence_enabled = v).await;
                        } else {
                            auto_off_armed = false;
                            update_and_notify(store, zone_index, notify, |z| {
                                z.presence_enabled = false;
                                z.auto_off_active = false;
                            }).await;
                        }
                        notify_presence(store, zone_index, notify).await;
                    }
                    ZoneCommand::SetAutoOffDelay(delay) => {
                        update_and_notify(store, zone_index, notify, |z| z.auto_off_delay = delay).await;
                    }
                }
            }
            pcm = async { match &mut decode_rx { Some(rx) => rx.recv().await, None => std::future::pending().await } } => {
                match pcm {
                    Some(audio::PcmMessage::Format { sample_rate, channels }) => {
                        resampler = audio::resample::F32Resampling::new(sample_rate, config.audio.sample_rate, channels);
                        tracing::debug!(from = sample_rate, to = config.audio.sample_rate, "Resampler configured");
                    }
                    Some(audio::PcmMessage::Audio(samples)) => {
                        let mut samples = resampler.process_or_passthrough(samples);
                        zone_eq.process(&mut samples);
                        if let Some(ref mut fade) = zone_fade {
                            if fade.process(&mut samples, config.audio.channels) {
                                if fade.fading_out {
                                    // Fade-out complete — don't send silence, wait for new stream
                                    zone_fade = None;
                                } else {
                                    // Fade-in complete
                                    zone_fade = None;
                                }
                            }
                        }
                        if let Err(e) = backend.send_audio(zone_index, &samples, config.audio.sample_rate, config.audio.channels).await {
                            tracing::error!(zone = zone_index, error = %e, "Audio send failed");
                        }
                    }
                    Some(audio::PcmMessage::Position(ms)) => {
                        update_and_notify(store, zone_index, notify, |z| {
                            if let Some(ref mut t) = z.track { t.position_ms = ms + position_offset_ms; }
                        }).await;
                    }
                    Some(audio::PcmMessage::BufferProgress { buffered_bytes, total_bytes }) => {
                        if let Some(total) = total_bytes {
                            let now = std::time::Instant::now();
                            if now.duration_since(last_buffer_progress).as_millis() >= 500 {
                                last_buffer_progress = now;
                                let duration = store.read().await.zones.get(&zone_index)
                                    .and_then(|z| z.track.as_ref().map(|t| t.duration_ms))
                                    .unwrap_or(0);
                                let buffered_ms = if total > 0 {
                                    (buffered_bytes as f64 / total as f64 * duration as f64) as i64
                                } else { 0 };
                                update_and_notify(store, zone_index, notify, |z| {
                                    z.buffered_ms = Some(buffered_ms);
                                }).await;
                            }
                        }
                    }
                    Some(audio::PcmMessage::Error { message, details }) => {
                        tracing::error!(zone = zone_index, error = %message, details = ?details, "Playback error in decoder");
                        stop_decode(&mut current_decode, &mut decode_rx).await;
                        update_and_notify(store, zone_index, notify, |z| {
                            z.playback = PlaybackState::Stopped;
                            z.source = SourceType::Idle;
                            z.track = None;
                            z.cover_url = None;
                            z.buffered_ms = None;
                        }).await;
                        let err_notif = crate::api::ws::Notification::PlaybackError {
                            zone: zone_index,
                            message,
                            details,
                            recoverable: false,
                        };
                        crate::api::ws::broadcast_notification(notify, &err_notif);
                    }
                    None => {
                        current_decode = None;
                        decode_rx = None;
                        position_offset_ms = 0;
                        handle_track_complete(&mut DecodeState { current_decode: &mut current_decode, current_cover: &mut current_cover, decode_rx: &mut decode_rx, source: &mut source }, &PlaybackCtx { config, subsonic: &subsonic, store, zone_index, notify, covers, track_cache: &track_cache }).await;
                    }
                }
            }
            Some(samples) = airplay_audio_rx.recv() => {
                handle_receiver_audio!(samples, ActiveSource::AirPlay, SourceType::AirPlay, "AirPlay");
            }
            Some(event) = airplay_event_rx.recv() => {
                handle_receiver_event!(event, ActiveSource::AirPlay, SourceType::AirPlay);
            }
            // ── Spotify Connect: audio ────────────────────────────
            Some(samples) = spotify_audio_rx.recv() => {
                handle_receiver_audio!(samples, ActiveSource::Spotify, SourceType::Spotify, "Spotify");
            }
            // ── Spotify Connect: events ───────────────────────────
            Some(event) = spotify_event_rx.recv() => {
                handle_receiver_event!(event, ActiveSource::Spotify, SourceType::Spotify);
            }
            // Auto-off timer expired
            () = &mut auto_off_timer, if auto_off_armed => {
                auto_off_armed = false;
                let should_stop = store.read().await.zones.get(&zone_index).is_some_and(|z| z.presence_source && !z.presence);
                if should_stop {
                    tracing::info!(zone = zone_index, "Presence auto-off: stopping playback");
                    stop_decode(&mut current_decode, &mut decode_rx).await;
                    source = ActiveSource::Idle;
                    update_and_notify(store, zone_index, notify, |z| {
                        z.playback = PlaybackState::Stopped;
                        z.source = SourceType::Idle;
                        z.track = None;
                        z.cover_url = None;
                        z.buffered_ms = None;
                        z.presence_source = false;
                        z.auto_off_active = false;
                    }).await;
                    notify_presence(store, zone_index, notify).await;
                }
            }
        }
    }
}

/// Emit a presence notification to WebSocket clients.
async fn notify_presence(
    store: &crate::state::SharedState,
    zone_index: usize,
    notify: &crate::api::ws::NotifySender,
) {
    let s = store.read().await;
    if let Some(z) = s.zones.get(&zone_index) {
        crate::api::ws::broadcast_notification(
            notify,
            &crate::api::ws::Notification::ZonePresenceChanged {
                zone: zone_index,
                presence: z.presence,
                enabled: z.presence_enabled,
                timer_active: z.auto_off_active,
            },
        );
    }
}

/// Resolve which source to play for presence-triggered playback.
/// Checks schedule (by current time) → `default_source` → None (resume).
fn resolve_presence_source(
    config: &crate::config::AppConfig,
    zone_index: usize,
) -> Option<crate::config::PresenceSource> {
    let zone_cfg = config.zones.get(zone_index - 1)?;
    let presence = zone_cfg.presence.as_ref()?;

    // Check schedule
    let now = chrono::Local::now();
    let now_minutes = (now.hour() * 60 + now.minute()) as u16;

    for entry in &presence.schedule {
        let from = crate::config::parse_time(&entry.from).unwrap_or_else(|_| {
            tracing::warn!(value = %entry.from, "Invalid schedule time, defaulting to 00:00");
            0
        });
        let to = crate::config::parse_time(&entry.to).unwrap_or_else(|_| {
            tracing::warn!(value = %entry.to, "Invalid schedule time, defaulting to 00:00");
            0
        });
        if now_minutes >= from && now_minutes < to {
            return Some(entry.source.clone());
        }
    }

    // Fallback to default_source
    presence.default_source.clone()
}

#[cfg(test)]
mod fade_tests {
    use super::ZoneFade;

    #[test]
    fn fade_out_stereo() {
        let mut fade = ZoneFade::new(100, 1000); // 100ms at 1kHz = 100 frames
        let mut buf = vec![1.0f32; 200]; // 100 frames * 2 channels
        let done = fade.process(&mut buf, 2);
        assert!(done);
        // First frame should be near 1.0, last frame should be near 0.0
        assert!(buf[0] > 0.9);
        assert!(buf[198] < 0.02);
    }

    #[test]
    fn fade_in_stereo() {
        let mut fade = ZoneFade::new(100, 1000);
        fade.start_fade_in();
        let mut buf = vec![1.0f32; 200];
        let done = fade.process(&mut buf, 2);
        assert!(done);
        assert!(buf[0] < 0.02);
        assert!(buf[198] > 0.9);
    }
}
