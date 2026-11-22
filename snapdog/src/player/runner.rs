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
    update_and_notify, update_progress_and_notify, update_volume_and_notify,
};

use super::helpers::{self, DecodeState, PlaybackCtx};
use super::helpers::{
    handle_next, handle_previous, handle_track_complete, prefetch_next_tracks, start_radio_decode,
    start_subsonic_track_decode, subsonic_track_info,
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
/// Cap on the exponential restart backoff for a repeatedly-crashing zone player.
const ZONE_RESTART_MAX_DELAY: std::time::Duration = std::time::Duration::from_mins(1);
/// A zone run lasting at least this long is "stable" — it resets the crash counter.
const ZONE_STABLE_RUN: std::time::Duration = std::time::Duration::from_mins(1);
/// Consecutive crashes (without a stable run) before giving up on a zone player.
const ZONE_MAX_CONSECUTIVE_CRASHES: u32 = 8;

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

    // One track cache for all zones (if Subsonic is configured): one index over the shared
    // directory, one size limit, and one download per track at a time (#495).
    let track_cache = ctx.config.subsonic.as_ref().and_then(|s| {
        crate::audio::cache::TrackCache::new(&s.cache)
            .map_err(|e| tracing::warn!(error = %e, "Failed to initialize track cache"))
            .ok()
    });

    for zone in &ctx.config.zones {
        let (cmd_tx, cmd_rx) = mpsc::channel(ZONE_CMD_CHANNEL_SIZE); // zone command backlog
        senders.insert(zone.index, cmd_tx);

        let ctx = ctx.clone();
        let track_cache = track_cache.clone();
        let zone_index = zone.index;

        tokio::spawn(async move {
            let mut cmd_rx = cmd_rx;
            supervise(zone_index, async move || {
                run(zone_index, &mut cmd_rx, ctx.clone(), track_cache.clone()).await
            })
            .await;
        });

        tracing::info!(zone = %zone.name, "Zone started");
    }

    Ok(senders)
}

/// Commands that start, change or stop playback. Sent by the user, they end a presence session.
const fn is_user_playback_command(cmd: &ZoneCommand) -> bool {
    matches!(
        cmd,
        ZoneCommand::Play
            | ZoneCommand::Pause
            | ZoneCommand::Stop
            | ZoneCommand::PlayUrl(_)
            | ZoneCommand::PlaySubsonicTrack(_)
            | ZoneCommand::PlaySubsonicPlaylist(..)
            | ZoneCommand::PlayPlaylist(..)
            | ZoneCommand::PlayPlaylistById(..)
            | ZoneCommand::SetPlaylist(..)
            | ZoneCommand::SetPlaylistById(..)
            | ZoneCommand::SetTrack(_)
            | ZoneCommand::Next
            | ZoneCommand::Previous
            | ZoneCommand::NextPlaylist
            | ZoneCommand::PreviousPlaylist
    )
}

/// The next command for the zone: a pending follow-up first, then the channel.
async fn next_command(
    follow_ups: &mut std::collections::VecDeque<ZoneCommand>,
    commands: &mut mpsc::Receiver<ZoneCommand>,
) -> Option<ZoneCommand> {
    if let Some(cmd) = follow_ups.pop_front() {
        return Some(cmd);
    }
    commands.recv().await
}

/// Supervise a zone player: run `attempt` to completion, restarting on a returned
/// error OR a panic (caught via `catch_unwind`) with capped exponential backoff.
///
/// A run lasting at least [`ZONE_STABLE_RUN`] resets the backoff and crash counter;
/// after [`ZONE_MAX_CONSECUTIVE_CRASHES`] rapid crashes it gives up (the zone stays
/// offline, logged loudly) rather than busy-restarting a deterministic panic. A clean
/// `Ok(())` ends supervision. (The previous `while let Err(..) = run(..)` loop only
/// caught `Err` — which `run` never returns — so a panicking zone died silently.)
async fn supervise<F>(zone_index: usize, mut attempt: F)
where
    F: AsyncFnMut() -> Result<()>,
{
    use futures_util::FutureExt as _;

    let mut backoff = ZONE_RESTART_DELAY;
    let mut consecutive: u32 = 0;
    loop {
        let started = tokio::time::Instant::now();
        match std::panic::AssertUnwindSafe(attempt()).catch_unwind().await {
            Ok(Ok(())) => break, // clean shutdown
            Ok(Err(e)) => tracing::error!(zone = zone_index, error = %e, "Zone player crashed"),
            Err(_) => tracing::error!(zone = zone_index, "Zone player panicked"),
        }

        // A long, healthy run before the crash counts as a fresh failure.
        if started.elapsed() >= ZONE_STABLE_RUN {
            consecutive = 0;
            backoff = ZONE_RESTART_DELAY;
        }
        consecutive += 1;
        if consecutive >= ZONE_MAX_CONSECUTIVE_CRASHES {
            tracing::error!(
                zone = zone_index,
                crashes = consecutive,
                "Zone player failed repeatedly — giving up (offline until process restart)"
            );
            break;
        }
        tracing::warn!(
            zone = zone_index,
            delay_s = backoff.as_secs(),
            "Restarting zone player"
        );
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(ZONE_RESTART_MAX_DELAY);
    }
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
        drain_with_fade_out(
            rx,
            &mut fade,
            deadline,
            |samples| {
                let mut samples = resampler.process_or_passthrough(samples);
                zone_eq.process(&mut samples);
                samples
            },
            backend,
            zone_index,
            sample_rate,
            channels,
        )
        .await;
    }

    stop_decode(current_decode, decode_rx).await;
    *position_offset_ms = 0;

    // Prepare fade-in for the next stream
    let mut fade_in = ZoneFade::new(fade_ms, sample_rate);
    fade_in.start_fade_in();
    *zone_fade = Some(fade_in);
}

/// Play the old stream out under a fade-out until the fade completes, `deadline` passes or the
/// stream ends. Non-audio messages (`Position`, `BufferProgress`, `Format`) are skipped: they
/// arrive between audio chunks and must not end the fade early, which would cut the stream.
#[allow(clippy::too_many_arguments)]
async fn drain_with_fade_out(
    rx: &mut mpsc::Receiver<audio::PcmMessage>,
    fade: &mut ZoneFade,
    deadline: tokio::time::Instant,
    mut prepare: impl FnMut(Vec<f32>) -> Vec<f32>,
    backend: &dyn crate::snapcast::backend::SnapcastBackend,
    zone_index: usize,
    sample_rate: u32,
    channels: u16,
) {
    loop {
        match tokio::time::timeout_at(deadline, rx.recv()).await {
            Ok(Some(audio::PcmMessage::Audio(samples))) => {
                let mut samples = prepare(samples);
                let done = fade.process(&mut samples, channels);
                let _ = backend
                    .send_audio(zone_index, &samples, sample_rate, channels)
                    .await;
                if done {
                    break;
                }
            }
            Ok(Some(_)) => {}           // not audio: keep draining
            Ok(None) | Err(_) => break, // stream ended or deadline passed
        }
    }
}

/// Check if a receiver (AirPlay/Spotify) is active and the source conflict
/// policy blocks local playback. Returns `true` if the command should proceed.
///
/// A paused receiver does not hold the zone, whatever the policy.
const fn may_start_local_playback(
    source: &ActiveSource,
    policy: crate::config::SourceConflict,
    receiver_paused: bool,
) -> bool {
    if !matches!(source, ActiveSource::AirPlay | ActiveSource::Spotify) || receiver_paused {
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
    ///
    /// After a completed fade-out every further sample is silent, so the rest of the buffer
    /// that holds the end of the fade does not play at full gain. After a completed fade-in
    /// the rest of the buffer is left as it is.
    fn process(&mut self, samples: &mut [f32], channels: u16) -> bool {
        if self.total == 0 {
            return true;
        }
        let ch = usize::from(channels.max(1));
        for frame in samples.chunks_mut(ch) {
            let gain = if self.remaining > 0 {
                snapdog_common::fade_gain(self.remaining, self.total, self.fading_out)
            } else if self.fading_out {
                0.0
            } else {
                break;
            };
            for sample in frame.iter_mut() {
                *sample *= gain;
            }
            self.remaining = self.remaining.saturating_sub(1);
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
    ctx: Arc<ZonePlayerContext>,
    track_cache: Option<crate::audio::cache::TrackCache>,
) -> Result<()> {
    const TIMER_INACTIVE: std::time::Duration = std::time::Duration::from_hours(24);

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

    // AirPlay: F32 audio channel + event channel + receiver instance
    let (airplay_audio_tx, mut airplay_audio_rx) =
        crate::receiver::audio_channel(RECEIVER_AUDIO_CHANNEL_SIZE);
    let (airplay_event_tx, mut airplay_event_rx) =
        mpsc::channel::<crate::receiver::ReceiverEvent>(RECEIVER_EVENT_CHANNEL_SIZE);
    let mut _airplay_receiver = if ctx.start_receivers && zone_config.airplay {
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
    } else {
        // Receivers disabled (tests/headless): keep the channels (the select loop
        // owns the rx halves) but never bind the RAOP socket or register mDNS.
        drop((airplay_audio_tx, airplay_event_tx));
        None
    };
    #[cfg(feature = "test-harness")]
    {
        let injected = ctx
            .test_airplay_rx
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&zone_index);
        if let Some((audio, events)) = injected {
            airplay_audio_rx = audio;
            airplay_event_rx = events;
        }
    }

    // Spotify Connect: F32 audio channel + event channel + receiver instance
    #[cfg(feature = "spotify")]
    let (mut spotify_audio_rx, mut spotify_event_rx, mut _spotify_receiver) = {
        let (audio_tx, audio_rx) = crate::receiver::audio_channel(RECEIVER_AUDIO_CHANNEL_SIZE);
        let (event_tx, event_rx) =
            mpsc::channel::<crate::receiver::ReceiverEvent>(RECEIVER_EVENT_CHANNEL_SIZE);
        let receiver = if let Some(sp_config) = config
            .spotify
            .as_ref()
            .filter(|_| ctx.start_receivers && zone_config.spotify)
        {
            let mut sp = crate::receiver::spotify::SpotifyReceiver::new(
                crate::config::SpotifyConfig {
                    name: zone_config.spotify_name.clone(),
                    bitrate: sp_config.bitrate,
                    port: sp_config.port,
                    zeroconf_backend: sp_config.zeroconf_backend.clone(),
                    bind: None,
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
    // Test-only (feature `test-harness`): adopt a pre-seeded PCM receiver so the
    // harness can drive the real decode→resample→EQ→send_audio path (IT-T55)
    // without a network decode. In the production binary this is simply `None`.
    #[cfg(not(feature = "test-harness"))]
    let mut decode_rx: Option<mpsc::Receiver<audio::PcmMessage>> = None;
    #[cfg(feature = "test-harness")]
    let mut decode_rx: Option<mpsc::Receiver<audio::PcmMessage>> = ctx
        .test_pcm_rx
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .remove(&zone_index);
    let mut source = ActiveSource::Idle;
    // Per receiver: its remote control, its resampler and its gate. A receiver whose gate is
    // closed has its audio drained and dropped: it lost the zone to a later action (local
    // playback, Stop, auto-off or the other receiver) and gets it back only once it reports
    // playing again. The last action wins.
    let mut airplay_remote: Option<std::sync::Arc<dyn crate::receiver::RemoteControl>> = None;
    let mut spotify_remote: Option<std::sync::Arc<dyn crate::receiver::RemoteControl>> = None;
    let mut airplay_open = true;
    let mut spotify_open = true;
    // The receiver that holds the zone has paused: it does not block local playback.
    let mut receiver_paused = false;
    let mut last_source = ActiveSource::Idle;
    let mut position_offset_ms: i64 = 0;
    let mut last_buffer_progress = std::time::Instant::now();
    let mut zone_fade: Option<ZoneFade> = None;
    let mut resampler = audio::resample::F32Resampling::new(
        config.audio.sample_rate,
        config.audio.sample_rate,
        config.audio.channels,
    );
    let mut airplay_resampler: Option<audio::resample::F32Resampling> = None;
    let mut spotify_resampler: Option<audio::resample::F32Resampling> = None;
    // Channel counts of the current decode and of each receiver. Their audio is mapped to the
    // configured count before resampling, so EQ, fade and the stream see whole output frames.
    let mut source_channels = config.audio.channels;
    let mut airplay_channels = config.audio.channels;
    let mut spotify_channels = config.audio.channels;

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

    /// Leave the zone idle ([`helpers::enter_idle_with`]) and reset the position offset. With an
    /// argument, that closure is applied in the same store update.
    macro_rules! enter_idle {
        () => {
            enter_idle!(|_: &mut crate::state::ZoneState| {})
        };
        ($also:expr) => {{
            helpers::enter_idle_with(
                &mut DecodeState {
                    current_decode: &mut current_decode,
                    current_cover: &mut current_cover,
                    decode_rx: &mut decode_rx,
                    source: &mut source,
                },
                &PlaybackCtx {
                    config,
                    subsonic: &subsonic,
                    store,
                    zone_index,
                    notify,
                    covers,
                    track_cache: &track_cache,
                    playlist_cache: &ctx.playlist_cache,
                },
                $also,
            )
            .await;
            position_offset_ms = 0;
        }};
    }

    /// Handle audio samples from an external receiver (AirPlay/Spotify).
    macro_rules! handle_receiver_audio {
        ($samples:expr, $active:path, $source_type:expr, $label:literal, $open:ident, $resampler:ident, $channels:ident) => {{
            if !$open {
                continue; // drained and dropped until the receiver plays again
            }
            if !matches!(source, $active) {
                reset_playback(&mut current_decode, &mut decode_rx, &mut position_offset_ms).await;
                source = $active;
                update_and_notify(store, zone_index, notify, |z| { z.playback = PlaybackState::Playing; z.source = $source_type; }).await;
            }
            receiver_paused = false;
            let mapped = audio::channels::map_channels($samples, $channels, config.audio.channels);
            let mut samples = match &mut $resampler {
                Some(r) => match r.process(&mapped) { Some(resampled) => resampled, None => continue },
                None => mapped,
            };
            zone_eq.process(&mut samples);
            if let Err(e) = backend.send_audio(zone_index, &samples, config.audio.sample_rate, config.audio.channels).await {
                tracing::error!(zone = zone_index, error = %e, concat!("Audio send failed (", $label, ")"));
            }
        }};
    }

    /// Handle events from an external receiver (AirPlay/Spotify).
    macro_rules! handle_receiver_event {
        ($event:expr, $active:path, $source_type:expr, $open:ident, $resampler:ident, $remote:ident, $channels:ident) => {{
            use crate::receiver::ReceiverEvent;
            match $event {
                ReceiverEvent::PlaybackState { playing } => {
                    if playing {
                        $open = true;
                        if matches!(source, $active) {
                            receiver_paused = false;
                            update_and_notify(store, zone_index, notify, |z| {
                                z.playback = PlaybackState::Playing
                            })
                            .await;
                        }
                    } else if matches!(source, $active) {
                        receiver_paused = true;
                        update_and_notify(store, zone_index, notify, |z| {
                            z.playback = PlaybackState::Paused
                        })
                        .await;
                    }
                }
                ReceiverEvent::SessionStarted { format } => {
                    $channels = format.channels;
                    $resampler = Some(audio::resample::F32Resampling::new(
                        format.sample_rate,
                        config.audio.sample_rate,
                        config.audio.channels,
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
                        let url = crate::state::cover::zone_cover_url(zone_index, &h);
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
                    update_progress_and_notify(store, zone_index, notify, |z| {
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
                    let percent = percent.clamp(0, zone_config.max_volume);
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
                    $remote = Some(remote);
                }
                ReceiverEvent::SessionEnded => {
                    $remote = None;
                    if matches!(source, $active) {
                        enter_idle!();
                    }
                }
            }
        }};
    }

    // Commands a handler derives from another one (seek steps, presence starts). They are
    // handled before the next command from the channel instead of being sent back into it:
    // awaiting a send on its own bounded channel could block this task forever (#494).
    let mut follow_ups: std::collections::VecDeque<ZoneCommand> = std::collections::VecDeque::new();
    // The next playback command was queued by a presence trigger, not by the user.
    let mut presence_derived = false;
    // A user Stop ends presence playback until the room was empty once: a `false` telegram,
    // or no trigger for the auto-off delay (pulse-only detectors never send `false`).
    let mut presence_blocked = false;

    loop {
        // A receiver that lost the zone since the last iteration (to local playback, Stop,
        // auto-off, the other receiver or the end of its session) is shut out and asked to pause.
        if std::mem::discriminant(&last_source) != std::mem::discriminant(&source) {
            if matches!(last_source, ActiveSource::AirPlay) {
                airplay_open = false;
                if let Some(rc) = &airplay_remote {
                    let _ = rc.send_command(crate::receiver::RemoteCommand::Pause);
                }
            }
            if matches!(last_source, ActiveSource::Spotify) {
                spotify_open = false;
                if let Some(rc) = &spotify_remote {
                    let _ = rc.send_command(crate::receiver::RemoteCommand::Pause);
                }
            }
            receiver_paused = false;
            last_source = source.clone();
        }
        tokio::select! {
            Some(cmd) = next_command(&mut follow_ups, commands) => {
                // A checked command carries a reply channel: answer whether its index exists.
                let (cmd, mut reply) = match cmd {
                    ZoneCommand::Checked(inner, reply) => (*inner, Some(reply)),
                    other => (other, None),
                };
                let mut answer = |ok: bool| {
                    if let Some(r) = reply.take() {
                        let _ = r.send(ok);
                    }
                };
                if is_user_playback_command(&cmd) && !std::mem::take(&mut presence_derived) {
                    // The user takes over: presence no longer owns this playback, so its
                    // auto-off timer must not stop what the user starts now.
                    let (owned, enabled) = store.read().await.zones.get(&zone_index)
                        .map_or((false, false), |z| (z.presence_source, z.presence_enabled));
                    if owned || auto_off_armed {
                        auto_off_armed = false;
                        update_and_notify(store, zone_index, notify, |z| { z.presence_source = false; z.auto_off_active = false; }).await;
                        notify_presence(store, zone_index, notify).await;
                        tracing::info!(zone = zone_index, "Presence: playback handed over to the user");
                    }
                    if matches!(cmd, ZoneCommand::Stop) && enabled {
                        presence_blocked = true;
                        let delay = store.read().await.zones.get(&zone_index).map_or(crate::config::DEFAULT_AUTO_OFF_DELAY, |z| z.auto_off_delay);
                        auto_off_timer.as_mut().reset(tokio::time::Instant::now() + std::time::Duration::from_secs(u64::from(delay)));
                        auto_off_armed = true;
                        tracing::info!(zone = zone_index, "Presence: starts suspended until the room was empty");
                    } else {
                        presence_blocked = false;
                    }
                }
                match cmd {
                    ZoneCommand::Checked(..) => answer(false), // a checked command is never nested
                    ZoneCommand::PlaySubsonicPlaylist(playlist_id, track_idx) => {
                        if !may_start_local_playback(&source, config.audio.source_conflict, receiver_paused) {
                            tracing::info!(zone = zone_index, "Playback blocked: receiver has priority");
                            continue;
                        }
                        fade_transition(&mut current_decode, &mut decode_rx, &mut position_offset_ms, &mut zone_fade, config.audio.source_switch_fade_ms, config.audio.sample_rate, &mut zone_eq, ctx.backend.as_ref(), zone_index, config.audio.channels, &mut resampler).await;
                        if let Some(sub) = &subsonic {
                            let playlist_entries = ctx.playlist_cache.get(Some(sub)).await;
                            let playlist_index = playlist_entries
                                .iter()
                                .position(|p| p.id == playlist_id)
                                .map(|sub_idx| if config.has_radio_playlist() { sub_idx + 1 } else { sub_idx });
                            let playlist_total = config.unified_playlist_count(playlist_entries.len());
                            let is_album = crate::subsonic::is_album_queue(&playlist_id);
                            if let Ok(playlist) = sub.get_queue(&playlist_id).await {
                                let track_count = playlist.entry.len();
                                let track_idx = track_idx.min(track_count.saturating_sub(1));
                                if let Some(track) = playlist.entry.get(track_idx) {
                                    start_subsonic_track_decode(sub, track, &mut DecodeState { current_decode: &mut current_decode, current_cover: &mut current_cover, decode_rx: &mut decode_rx, source: &mut source }, &PlaybackCtx { config, subsonic: &subsonic, store, zone_index, notify, covers, track_cache: &track_cache, playlist_cache: &ctx.playlist_cache }).await;
                                    source = ActiveSource::SubsonicPlaylist { playlist_id, track_index: track_idx, track_count };
                                    update_and_notify(store, zone_index, notify, |z| {
                                        z.playback = PlaybackState::Playing;
                                        z.source = SourceType::SubsonicPlaylist;
                                        z.playlist_index = playlist_index;
                                        z.playlist_id = (!is_album).then(|| crate::config::subsonic_playlist_id(&playlist.id));
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
                                } else {
                                    tracing::warn!(zone = zone_index, "Queue has no such track");
                                    enter_idle!();
                                }
                            } else {
                                tracing::warn!(zone = zone_index, "Could not fetch the queue");
                                enter_idle!();
                            }
                        }
                    }
                    ZoneCommand::PlaySubsonicTrack(track_id) => {
                        if !may_start_local_playback(&source, config.audio.source_conflict, receiver_paused) {
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
                            // The zone needs a track entry for position updates and for resuming
                            // at the paused position; without metadata it still gets one.
                            let info = match sub.get_song(&track_id).await {
                                Ok(song) => subsonic_track_info(&song),
                                Err(e) => {
                                    tracing::warn!(zone = zone_index, error = %e, "Could not read the track metadata");
                                    subsonic_track_info(&crate::subsonic::Track::untitled(&track_id))
                                }
                            };
                            source = ActiveSource::SubsonicTrack { track_id };
                            update_and_notify(store, zone_index, notify, |z| {
                                z.playback = PlaybackState::Playing;
                                z.source = SourceType::SubsonicTrack;
                                z.track = Some(info.clone());
                                if let Some(t) = z.track.as_mut() { t.position_ms = 0; t.source = SourceType::SubsonicTrack; }
                            }).await;
                        }
                    }
                    ZoneCommand::PlayUrl(url) => {
                        if !may_start_local_playback(&source, config.audio.source_conflict, receiver_paused) {
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
                        source = ActiveSource::Url { url };
                        update_and_notify(store, zone_index, notify, |z| { z.playback = PlaybackState::Playing; z.source = SourceType::Url; }).await;
                    }
                    ZoneCommand::SetTrack(track_idx) => {
                        let exists = match source {
                            ActiveSource::Radio { .. } => track_idx < config.radios.len(),
                            ActiveSource::SubsonicPlaylist { track_count, .. } => track_idx < track_count,
                            _ => false,
                        };
                        answer(exists);
                        if !exists {
                            tracing::warn!(zone = zone_index, track = track_idx, "No such track in the active playlist");
                        }
                        if let ActiveSource::Radio { .. } = source {
                            if track_idx < config.radios.len()
                                && let Some(radio) = config.radios.get(track_idx) {
                                    reset_playback(&mut current_decode, &mut decode_rx, &mut position_offset_ms).await;
                                    start_radio_decode(radio, &mut DecodeState { current_decode: &mut current_decode, current_cover: &mut current_cover, decode_rx: &mut decode_rx, source: &mut source }, &PlaybackCtx { config, subsonic: &subsonic, store, zone_index, notify, covers, track_cache: &track_cache, playlist_cache: &ctx.playlist_cache }).await;
                                    source = ActiveSource::Radio { index: track_idx };
                                    let total = helpers::radio_playlist_total(&PlaybackCtx { config, subsonic: &subsonic, store, zone_index, notify, covers, track_cache: &track_cache, playlist_cache: &ctx.playlist_cache }).await;
                                    update_and_notify(store, zone_index, notify, |z| {
                                        z.playback = PlaybackState::Playing;
                                        helpers::apply_radio_state(z, config, total, track_idx, radio);
                                    }).await;
                                    tracing::info!(zone = %zone_config.name, radio = %radio.name, "Radio set");
                                }
                        } else if let ActiveSource::SubsonicPlaylist { ref playlist_id, track_count, .. } = source
                            && track_idx < track_count {
                                let pid = playlist_id.clone();
                                reset_playback(&mut current_decode, &mut decode_rx, &mut position_offset_ms).await;
                                if let Some(sub) = &subsonic
                                    && let Ok(playlist) = sub.get_queue(&pid).await
                                        && let Some(track) = playlist.entry.get(track_idx) {
                                            start_subsonic_track_decode(sub, track, &mut DecodeState { current_decode: &mut current_decode, current_cover: &mut current_cover, decode_rx: &mut decode_rx, source: &mut source }, &PlaybackCtx { config, subsonic: &subsonic, store, zone_index, notify, covers, track_cache: &track_cache, playlist_cache: &ctx.playlist_cache }).await;
                                            source = ActiveSource::SubsonicPlaylist { playlist_id: pid, track_index: track_idx, track_count };
                                            update_and_notify(store, zone_index, notify, |z| {
                                                z.playback = PlaybackState::Playing;
                                                z.source = SourceType::SubsonicPlaylist;
                                                z.playlist_track_index = Some(track_idx);
                                                z.track = Some(subsonic_track_info(track));
                                            }).await;
                                        } else {
                                            tracing::warn!(zone = zone_index, track = track_idx, "Could not fetch the track");
                                            enter_idle!();
                                        }
                            }
                    }
                    ZoneCommand::Play => {
                        if matches!(source, ActiveSource::AirPlay | ActiveSource::Spotify) {
                            if let Some(rc) = match source { ActiveSource::AirPlay => airplay_remote.as_ref(), ActiveSource::Spotify => spotify_remote.as_ref(), _ => None }
                                && let Err(e) = rc.send_command(crate::receiver::RemoteCommand::Play) { tracing::warn!(error = %e, "Remote play failed"); }
                        } else if matches!(source, ActiveSource::SubsonicPlaylist { .. } | ActiveSource::SubsonicTrack { .. }) {
                            // Resume Subsonic from last position
                            if let Some(sub) = &subsonic {
                                let track_id = match &source {
                                    ActiveSource::SubsonicPlaylist { playlist_id, track_index, .. } => {
                                        sub.get_queue(playlist_id).await.ok()
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
                                    position_offset_ms = helpers::start_track_at(sub, &tid, pos_ms, &PlaybackCtx { config, subsonic: &subsonic, store, zone_index, notify, covers, track_cache: &track_cache, playlist_cache: &ctx.playlist_cache }, &mut current_decode, &mut decode_rx);
                                    update_and_notify(store, zone_index, notify, |z| { z.playback = PlaybackState::Playing; }).await;
                                }
                            }
                        } else if let ActiveSource::Url { url } = &source {
                            // A URL stream has no position to resume at: start it again.
                            let u = url.clone();
                            reset_playback(&mut current_decode, &mut decode_rx, &mut position_offset_ms).await;
                            let (tx, rx) = audio::pcm_channel(PCM_DECODE_CHANNEL_SIZE);
                            decode_rx = Some(rx);
                            let ac = audio_config.clone();
                            current_decode = Some(tokio::spawn(async move {
                                if let Err(e) = audio::decode_http_stream(u, tx, ac, None).await { tracing::error!(error = %e, "URL decode failed"); }
                            }));
                            update_and_notify(store, zone_index, notify, |z| { z.playback = PlaybackState::Playing; z.source = SourceType::Url; }).await;
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
                                start_radio_decode(radio, &mut DecodeState { current_decode: &mut current_decode, current_cover: &mut current_cover, decode_rx: &mut decode_rx, source: &mut source }, &PlaybackCtx { config, subsonic: &subsonic, store, zone_index, notify, covers, track_cache: &track_cache, playlist_cache: &ctx.playlist_cache }).await;
                                source = ActiveSource::Radio { index: radio_idx };
                                let total = helpers::radio_playlist_total(&PlaybackCtx { config, subsonic: &subsonic, store, zone_index, notify, covers, track_cache: &track_cache, playlist_cache: &ctx.playlist_cache }).await;
                                update_and_notify(store, zone_index, notify, |z| {
                                    z.playback = PlaybackState::Playing;
                                    helpers::apply_radio_state(z, config, total, radio_idx, radio);
                                }).await;
                            }
                        }
                    }
                    ZoneCommand::Pause => {
                        if matches!(source, ActiveSource::AirPlay | ActiveSource::Spotify) {
                            if let Some(rc) = match source { ActiveSource::AirPlay => airplay_remote.as_ref(), ActiveSource::Spotify => spotify_remote.as_ref(), _ => None }
                                && let Err(e) = rc.send_command(crate::receiver::RemoteCommand::Pause) { tracing::warn!(error = %e, "Remote pause failed"); }
                        } else {
                            reset_playback(&mut current_decode, &mut decode_rx, &mut position_offset_ms).await;
                            update_and_notify(store, zone_index, notify, |z| { z.playback = PlaybackState::Paused; }).await;
                        }
                    }
                    ZoneCommand::Stop => enter_idle!(),
                    ZoneCommand::Next => {
                        if matches!(source, ActiveSource::AirPlay | ActiveSource::Spotify) {
                            if let Some(rc) = match source { ActiveSource::AirPlay => airplay_remote.as_ref(), ActiveSource::Spotify => spotify_remote.as_ref(), _ => None } { let _ = rc.send_command(crate::receiver::RemoteCommand::NextTrack); }
                        } else {
                            handle_next(&mut DecodeState { current_decode: &mut current_decode, current_cover: &mut current_cover, decode_rx: &mut decode_rx, source: &mut source }, &PlaybackCtx { config, subsonic: &subsonic, store, zone_index, notify, covers, track_cache: &track_cache, playlist_cache: &ctx.playlist_cache }).await;
                        }
                    }
                    ZoneCommand::Previous => {
                        if matches!(source, ActiveSource::AirPlay | ActiveSource::Spotify) {
                            if let Some(rc) = match source { ActiveSource::AirPlay => airplay_remote.as_ref(), ActiveSource::Spotify => spotify_remote.as_ref(), _ => None } { let _ = rc.send_command(crate::receiver::RemoteCommand::PreviousTrack); }
                        } else {
                            handle_previous(&mut DecodeState { current_decode: &mut current_decode, current_cover: &mut current_cover, decode_rx: &mut decode_rx, source: &mut source }, &PlaybackCtx { config, subsonic: &subsonic, store, zone_index, notify, covers, track_cache: &track_cache, playlist_cache: &ctx.playlist_cache }).await;
                        }
                    }
                    ZoneCommand::NextPlaylist | ZoneCommand::PreviousPlaylist | ZoneCommand::SetPlaylist(..) | ZoneCommand::PlayPlaylist(..) | ZoneCommand::SetPlaylistById(..) | ZoneCommand::PlayPlaylistById(..) => {
                        if !may_start_local_playback(&source, config.audio.source_conflict, receiver_paused) {
                            tracing::info!(zone = zone_index, "Playback blocked: receiver has priority");
                            continue;
                        }

                        // Unified playlist model: index 0 = radio (from config), index 1+ = Subsonic playlists
                        let subsonic_playlists = ctx.playlist_cache.get(subsonic.as_ref()).await;
                        let total_count = config.unified_playlist_count(subsonic_playlists.len());
                        if total_count == 0 {
                            tracing::warn!(zone = zone_index, "No playlists available");
                            answer(false);
                            continue;
                        }
                        if let ZoneCommand::SetPlaylist(i, _) | ZoneCommand::PlayPlaylist(i, _) = cmd
                            && i >= total_count
                        {
                            tracing::warn!(zone = zone_index, playlist = i, total = total_count, "No such playlist");
                            answer(false);
                            continue;
                        }
                        answer(true);

                        let has_radio = config.has_radio_playlist();
                        let current_unified = match &source {
                            ActiveSource::SubsonicPlaylist { playlist_id, .. } => {
                                let sub_idx = subsonic_playlists.iter().position(|p| p.id == *playlist_id).unwrap_or(0);
                                if has_radio { sub_idx + 1 } else { sub_idx }
                            }
                            _ => 0,
                        };

                        let force_play = matches!(&cmd, ZoneCommand::PlayPlaylist(..) | ZoneCommand::PlayPlaylistById(..));
                        let (target_unified, start_track) = match cmd {
                            ZoneCommand::NextPlaylist => ((current_unified + 1) % total_count, 0),
                            ZoneCommand::PreviousPlaylist => (if current_unified == 0 { total_count - 1 } else { current_unified - 1 }, 0),
                            ZoneCommand::SetPlaylist(i, t) | ZoneCommand::PlayPlaylist(i, t) => (i, t),
                            ZoneCommand::SetPlaylistById(ref id, t) | ZoneCommand::PlayPlaylistById(ref id, t) => {
                                let ids: Vec<&str> = subsonic_playlists.iter().map(|p| p.id.as_str()).collect();
                                let Some(i) = config.playlist_index_by_id(id, &ids) else {
                                    tracing::warn!(zone = zone_index, playlist_id = %id, "Unknown playlist_id");
                                    continue;
                                };
                                (i, t)
                            }
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
                                        start_radio_decode(radio, &mut DecodeState { current_decode: &mut current_decode, current_cover: &mut current_cover, decode_rx: &mut decode_rx, source: &mut source }, &PlaybackCtx { config, subsonic: &subsonic, store, zone_index, notify, covers, track_cache: &track_cache, playlist_cache: &ctx.playlist_cache }).await;
                                    }
                                    source = ActiveSource::Radio { index: radio_idx };
                                    update_and_notify(store, zone_index, notify, |z| {
                                        z.playback = if should_start { PlaybackState::Playing } else { PlaybackState::Stopped };
                                        helpers::apply_radio_state(z, config, total_count, radio_idx, radio);
                                    }).await;
                                    if should_start {
                                        tracing::info!(zone = %zone_config.name, radio = %radio.name, "Radio playlist started");
                                    } else {
                                        tracing::info!(zone = %zone_config.name, radio = %radio.name, "Radio playlist selected (stopped)");
                                    }
                                }
                            }
                            Some(crate::config::ResolvedPlaylist::Subsonic(sub_idx)) => {
                                if let Some(sub) = &subsonic
                                    && let Some(pl) = subsonic_playlists.get(sub_idx) {
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
                                                        start_subsonic_track_decode(sub, track, &mut DecodeState { current_decode: &mut current_decode, current_cover: &mut current_cover, decode_rx: &mut decode_rx, source: &mut source }, &PlaybackCtx { config, subsonic: &subsonic, store, zone_index, notify, covers, track_cache: &track_cache, playlist_cache: &ctx.playlist_cache }).await;
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
                                                    z.playlist_id = Some(crate::config::subsonic_playlist_id(&playlist.id));
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
                            _ => {}
                        }
                    }
                    ZoneCommand::Seek(pos_ms) => {
                        if let Some(sub) = &subsonic {
                            let track_id = match &source {
                                ActiveSource::SubsonicTrack { track_id } => Some(track_id.clone()),
                                ActiveSource::SubsonicPlaylist { playlist_id, track_index, .. } => {
                                    sub.get_queue(playlist_id).await.ok()
                                        .and_then(|p| p.entry.get(*track_index).map(|t| t.id.clone()))
                                }
                                _ => None,
                            };
                            if let Some(tid) = track_id {
                                stop_decode(&mut current_decode, &mut decode_rx).await;
                                let offset = helpers::start_track_at(sub, &tid, pos_ms, &PlaybackCtx { config, subsonic: &subsonic, store, zone_index, notify, covers, track_cache: &track_cache, playlist_cache: &ctx.playlist_cache }, &mut current_decode, &mut decode_rx);

                                update_and_notify(store, zone_index, notify, |z| {
                                    z.playback = PlaybackState::Playing;
                                    if let Some(ref mut t) = z.track { t.position_ms = pos_ms; }
                                }).await;
                                // A decode from the cache reports absolute positions, a stream relative ones.
                                position_offset_ms = offset;
                                tracing::debug!(zone = %zone_config.name, position_ms = pos_ms, "Seeked");
                            }
                        }
                    }
                    ZoneCommand::SeekProgress(progress) => {
                        let duration = store.read().await.zones.get(&zone_index)
                            .and_then(|z| z.track.as_ref().map(|t| t.duration_ms)).unwrap_or(0);
                        if duration > 0 {
                            let pos_ms = (progress.clamp(0.0, 1.0) * duration as f64) as i64;
                            follow_ups.push_back(ZoneCommand::Seek(pos_ms));
                        }
                    }
                    ZoneCommand::SeekStep(forward) => {
                        follow_ups.push_back(ZoneCommand::seek_by_interval(zone_config.seek_interval_secs, forward));
                    }
                    ZoneCommand::SeekRelative(offset_ms) => {
                        let (pos, dur) = store.read().await.zones.get(&zone_index)
                            .and_then(|z| z.track.as_ref().map(|t| (t.position_ms, t.duration_ms)))
                            .unwrap_or((0, 0));
                        if dur > 0 {
                            let target = pos.saturating_add(offset_ms).clamp(0, dur);
                            follow_ups.push_back(ZoneCommand::Seek(target));
                        }
                    }
                    ZoneCommand::SetVolume(v) => {
                        // max_volume is a hard limit for every source of a volume.
                        let v = v.clamp(0, zone_config.max_volume);
                        update_volume_and_notify(store, zone_index, notify, |z| z.volume = v).await;
                        let gid = store.read().await.zones.get(&zone_index).and_then(|z| z.snapcast_group_id.clone());
                        if let Some(gid) = gid {
                            let _ = ctx.snap_tx.send(SnapcastCmd::Group { group_id: gid, action: GroupAction::Volume(v) }).await;
                        }
                    }
                    ZoneCommand::AdjustVolume(delta) => {
                        let new_vol = {
                            let s = store.read().await;
                            s.zones.get(&zone_index).map_or(crate::state::DEFAULT_VOLUME, |z| z.volume.saturating_add(delta).clamp(0, zone_config.max_volume))
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
                    ZoneCommand::ToggleRepeatMode(mode) => {
                        update_and_notify(store, zone_index, notify, |z| z.repeat = super::commands::toggled_repeat(z.repeat, mode)).await;
                    }
                    ZoneCommand::ClearRepeatMode(mode) => {
                        update_and_notify(store, zone_index, notify, |z| z.repeat = super::commands::cleared_repeat(z.repeat, mode)).await;
                    }
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
                    ZoneCommand::SetEqBand { band_index, band } => {
                        let current = ctx.eq_store.lock().unwrap_or_else(std::sync::PoisonError::into_inner).get(zone_index);
                        let merged = (band_index < current.bands.len()).then(|| {
                            let mut config = current;
                            config.bands[band_index] = band;
                            config.preset = None;
                            config
                        });
                        if let Some(ref config) = merged {
                            zone_eq.set_config(config);
                            ctx.eq_store.lock().unwrap_or_else(std::sync::PoisonError::into_inner).set(zone_index, config.clone());
                            crate::api::ws::broadcast_notification(notify, &crate::api::ws::Notification::ZoneEqChanged {
                                zone: zone_index,
                                config: config.clone(),
                            });
                            tracing::debug!(zone = zone_index, band = band_index, "EQ band updated");
                        }
                    }
                    ZoneCommand::SetPresence(present) => {
                        // Retriggerable model: every presence trigger (`true`) starts playback
                        // if idle and (re)arms the auto-off timer to `now + delay`. Playback runs
                        // until the timer expires — i.e. no trigger for the configured delay — so
                        // the presence/motion detector must send cyclically while occupied. A
                        // `false` telegram is informational only; the timer, not the 0, stops
                        // playback (works with pulse-only motion detectors that never send 0).
                        let enabled = store.read().await.zones.get(&zone_index).is_some_and(|z| z.presence_enabled);
                        if !enabled {
                            update_and_notify(store, zone_index, notify, |z| z.presence = present).await;
                        } else if present && presence_blocked {
                            // After a user Stop: keep the room "occupied" (re-arm), start nothing.
                            let delay = store.read().await.zones.get(&zone_index).map_or(crate::config::DEFAULT_AUTO_OFF_DELAY, |z| z.auto_off_delay);
                            auto_off_timer.as_mut().reset(tokio::time::Instant::now() + std::time::Duration::from_secs(u64::from(delay)));
                            auto_off_armed = true;
                            update_and_notify(store, zone_index, notify, |z| z.presence = true).await;
                        } else if present {
                            let is_idle = store.read().await.zones.get(&zone_index).is_some_and(|z| z.playback == crate::state::PlaybackState::Stopped);
                            if is_idle {
                                // Resolve source: schedule → default → resume
                                let resolved = resolve_presence_source(config, zone_index);
                                match resolved {
                                    Some(crate::config::PresenceSource::Radio(idx)) => {
                                        update_and_notify(store, zone_index, notify, |z| z.presence_source = true).await;
                                        // Unified index 0 = radio, idx = station within radio
                                        follow_ups.push_back(ZoneCommand::PlayPlaylist(0, idx));
                                        presence_derived = true;
                                        tracing::info!(zone = zone_index, "Presence: playback started");
                                    }
                                    Some(crate::config::PresenceSource::Playlist(ref id)) => {
                                        update_and_notify(store, zone_index, notify, |z| z.presence_source = true).await;
                                        follow_ups.push_back(ZoneCommand::PlaySubsonicPlaylist(id.clone(), 0));
                                        presence_derived = true;
                                        tracing::info!(zone = zone_index, "Presence: playback started");
                                    }
                                    Some(crate::config::PresenceSource::None) => {
                                        update_and_notify(store, zone_index, notify, |z| z.presence_source = false).await;
                                    }
                                    None => {
                                        update_and_notify(store, zone_index, notify, |z| z.presence_source = true).await;
                                        follow_ups.push_back(ZoneCommand::Play);
                                        presence_derived = true;
                                        tracing::info!(zone = zone_index, "Presence: playback started");
                                    }
                                }
                            }
                            // (Re)arm the retriggerable auto-off timer while presence-initiated
                            // playback is active. User-started playback (presence_source = false)
                            // is left untouched — presence never hijacks or stops it.
                            let armed = store.read().await.zones.get(&zone_index).is_some_and(|z| z.presence_source);
                            if armed {
                                let delay = store.read().await.zones.get(&zone_index).map_or(crate::config::DEFAULT_AUTO_OFF_DELAY, |z| z.auto_off_delay);
                                auto_off_timer.as_mut().reset(tokio::time::Instant::now() + std::time::Duration::from_secs(u64::from(delay)));
                                auto_off_armed = true;
                                update_and_notify(store, zone_index, notify, |z| { z.presence = true; z.auto_off_active = true; }).await;
                                tracing::debug!(zone = zone_index, delay, "Presence trigger: auto-off timer (re)armed");
                            } else {
                                update_and_notify(store, zone_index, notify, |z| z.presence = true).await;
                            }
                        } else {
                            // No-motion telegram: display only. The timer from the last trigger
                            // runs on. It does mean the room is empty, which lifts a user Stop.
                            if presence_blocked {
                                presence_blocked = false;
                                auto_off_armed = false;
                            }
                            update_and_notify(store, zone_index, notify, |z| z.presence = false).await;
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
                        notify_presence(store, zone_index, notify).await;
                    }
                }
            }
            pcm = async { match &mut decode_rx { Some(rx) => rx.recv().await, None => std::future::pending().await } } => {
                match pcm {
                    Some(audio::PcmMessage::Format { sample_rate, channels }) => {
                        source_channels = channels;
                        resampler = audio::resample::F32Resampling::new(sample_rate, config.audio.sample_rate, config.audio.channels);
                        tracing::debug!(from = sample_rate, to = config.audio.sample_rate, channels, output_channels = config.audio.channels, "Resampler configured");
                    }
                    Some(audio::PcmMessage::Audio(samples)) => {
                        let samples = audio::channels::map_channels(samples, source_channels, config.audio.channels);
                        let mut samples = resampler.process_or_passthrough(samples);
                        zone_eq.process(&mut samples);
                        // Only a fade-in is ever pending here; fade-outs run in `fade_transition`.
                        if let Some(ref mut fade) = zone_fade
                            && fade.process(&mut samples, config.audio.channels) {
                                zone_fade = None;
                            }
                        if let Err(e) = backend.send_audio(zone_index, &samples, config.audio.sample_rate, config.audio.channels).await {
                            tracing::error!(zone = zone_index, error = %e, "Audio send failed");
                        }
                    }
                    Some(audio::PcmMessage::Position(ms)) => {
                        update_progress_and_notify(store, zone_index, notify, |z| {
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
                                update_progress_and_notify(store, zone_index, notify, |z| {
                                    z.buffered_ms = Some(buffered_ms);
                                }).await;
                            }
                        }
                    }
                    Some(audio::PcmMessage::Error { message, details }) => {
                        tracing::error!(zone = zone_index, error = %message, details = ?details, "Playback error in decoder");
                        enter_idle!();
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
                        handle_track_complete(&mut DecodeState { current_decode: &mut current_decode, current_cover: &mut current_cover, decode_rx: &mut decode_rx, source: &mut source }, &PlaybackCtx { config, subsonic: &subsonic, store, zone_index, notify, covers, track_cache: &track_cache, playlist_cache: &ctx.playlist_cache }).await;
                    }
                }
            }
            Some(samples) = airplay_audio_rx.recv() => {
                handle_receiver_audio!(samples, ActiveSource::AirPlay, SourceType::AirPlay, "AirPlay", airplay_open, airplay_resampler, airplay_channels);
            }
            Some(event) = airplay_event_rx.recv() => {
                handle_receiver_event!(event, ActiveSource::AirPlay, SourceType::AirPlay, airplay_open, airplay_resampler, airplay_remote, airplay_channels);
            }
            // ── Spotify Connect: audio ────────────────────────────
            Some(samples) = spotify_audio_rx.recv() => {
                handle_receiver_audio!(samples, ActiveSource::Spotify, SourceType::Spotify, "Spotify", spotify_open, spotify_resampler, spotify_channels);
            }
            // ── Spotify Connect: events ───────────────────────────
            Some(event) = spotify_event_rx.recv() => {
                handle_receiver_event!(event, ActiveSource::Spotify, SourceType::Spotify, spotify_open, spotify_resampler, spotify_remote, spotify_channels);
            }
            // Auto-off timer expired
            () = &mut auto_off_timer, if auto_off_armed => {
                auto_off_armed = false;
                if std::mem::take(&mut presence_blocked) {
                    // No trigger for the whole delay after a user Stop: the room was empty,
                    // presence may start playback again. Nothing to stop.
                    continue;
                }
                // Retrigger model: the timer only fires when no trigger arrived for the whole
                // delay, so presence-initiated playback stops regardless of the last display flag.
                let should_stop = store.read().await.zones.get(&zone_index).is_some_and(|z| z.presence_source);
                if should_stop {
                    tracing::info!(zone = zone_index, "Presence auto-off: stopping playback");
                    enter_idle!(|z: &mut crate::state::ZoneState| {
                        z.presence_source = false;
                        z.auto_off_active = false;
                    });
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
                auto_off_delay: Some(z.auto_off_delay),
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

    // Check schedule against the KNX-synced clock (offset is 0 unless a KNX Time GO set it).
    let offset = crate::knx::KNX_TIME_OFFSET_SECS.load(std::sync::atomic::Ordering::Relaxed);
    let now = chrono::Local::now() + chrono::Duration::seconds(offset);
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
    use crate::snapcast::backend::{BoxFuture, SnapcastBackend};

    /// Records the audio it is sent.
    #[derive(Default)]
    struct Recorder(std::sync::Mutex<Vec<f32>>);

    impl SnapcastBackend for Recorder {
        fn send_audio(
            &self,
            _: usize,
            samples: &[f32],
            _: u32,
            _: u16,
        ) -> BoxFuture<'_, anyhow::Result<()>> {
            self.0.lock().unwrap().extend_from_slice(samples);
            Box::pin(async { Ok(()) })
        }
        fn execute(&self, _: crate::player::SnapcastCmd) -> BoxFuture<'_, anyhow::Result<()>> {
            Box::pin(async { Ok(()) })
        }
        fn stop(&self) -> BoxFuture<'_, anyhow::Result<()>> {
            Box::pin(async { Ok(()) })
        }
        fn get_status(&self) -> BoxFuture<'_, anyhow::Result<serde_json::Value>> {
            Box::pin(async { Ok(serde_json::Value::Null) })
        }
        fn delete_client(&self, _: &str) -> BoxFuture<'_, anyhow::Result<()>> {
            Box::pin(async { Ok(()) })
        }
    }

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
    fn fade_out_silences_the_rest_of_a_longer_buffer() {
        let mut fade = ZoneFade::new(100, 1000); // 100 frames
        let mut buf = vec![1.0f32; 600]; // 300 frames * 2 channels
        assert!(fade.process(&mut buf, 2));
        assert!(buf[0] > 0.9);
        assert!(
            buf[200..].iter().all(|&s| s == 0.0),
            "no burst after the fade end"
        );
        // A further buffer after the completed fade-out is silent too.
        let mut next = vec![1.0f32; 10];
        assert!(fade.process(&mut next, 2));
        assert!(next.iter().all(|&s| s == 0.0));
    }

    #[test]
    fn fade_in_leaves_the_rest_of_a_longer_buffer() {
        let mut fade = ZoneFade::new(100, 1000);
        fade.start_fade_in();
        let mut buf = vec![1.0f32; 600];
        assert!(fade.process(&mut buf, 2));
        assert!(buf[200..].iter().all(|&s| s == 1.0));
    }

    #[tokio::test]
    async fn the_fade_out_runs_through_position_and_progress_messages() {
        use crate::audio::PcmMessage;
        let (tx, mut rx) = tokio::sync::mpsc::channel(64);
        // 100 frames of fade at 1 kHz, mono; audio in chunks of 30 frames with progress
        // and position messages in between, the way a caching stream sends them.
        for i in 0..6 {
            tx.send(PcmMessage::Audio(vec![1.0; 30])).await.unwrap();
            tx.send(PcmMessage::BufferProgress {
                buffered_bytes: i,
                total_bytes: None,
            })
            .await
            .unwrap();
            tx.send(PcmMessage::Position(i64::from(i as u8)))
                .await
                .unwrap();
        }
        let mut fade = ZoneFade::new(100, 1000);
        let backend = Recorder::default();
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
        super::drain_with_fade_out(&mut rx, &mut fade, deadline, |s| s, &backend, 1, 1000, 1).await;
        let played = backend.0.lock().unwrap().clone();
        assert_eq!(fade.remaining, 0, "the fade completed");
        assert_eq!(
            played.len(),
            120,
            "four chunks: the fade ends in the fourth"
        );
        assert!(played[100..].iter().all(|&s| s == 0.0));
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

    #[test]
    fn fade_total_samples_from_ms() {
        assert_eq!(ZoneFade::new(300, 48000).total, 14400); // 48000 * 300 / 1000
        assert_eq!(ZoneFade::new(100, 1000).total, 100);
    }

    #[test]
    fn fade_zero_duration_completes_and_preserves_buffer() {
        let mut fade = ZoneFade::new(0, 48000);
        let mut buf = vec![1.0f32; 8];
        assert!(
            fade.process(&mut buf, 2),
            "zero-duration fade completes immediately"
        );
        assert_eq!(buf, vec![1.0f32; 8], "buffer left unchanged");
    }

    #[test]
    fn fade_gain_is_per_frame_across_channels() {
        // Both samples of a stereo frame receive the same gain (per-frame decrement).
        let mut fade = ZoneFade::new(100, 1000);
        let mut buf = vec![1.0f32; 200];
        fade.process(&mut buf, 2);
        for f in 0..100 {
            assert_eq!(buf[2 * f], buf[2 * f + 1], "L and R share the frame's gain");
        }
    }
}

#[cfg(test)]
mod conflict_tests {
    use super::may_start_local_playback;
    use crate::config::SourceConflict;
    use crate::player::commands::ActiveSource;

    #[test]
    fn non_receiver_sources_always_allow_local_playback() {
        for src in [
            ActiveSource::Idle,
            ActiveSource::Url { url: String::new() },
            ActiveSource::Radio { index: 0 },
        ] {
            assert!(may_start_local_playback(
                &src,
                SourceConflict::LastWins,
                false
            ));
            assert!(may_start_local_playback(
                &src,
                SourceConflict::ReceiverWins,
                false
            ));
        }
    }

    #[test]
    fn receiver_active_blocks_local_playback_only_under_receiver_wins() {
        for src in [ActiveSource::AirPlay, ActiveSource::Spotify] {
            assert!(
                may_start_local_playback(&src, SourceConflict::LastWins, false),
                "LastWins lets local playback take over an active receiver"
            );
            assert!(
                !may_start_local_playback(&src, SourceConflict::ReceiverWins, false),
                "ReceiverWins blocks local playback while a receiver is active"
            );
        }
    }

    #[test]
    fn a_paused_receiver_never_blocks_local_playback() {
        for src in [ActiveSource::AirPlay, ActiveSource::Spotify] {
            assert!(may_start_local_playback(
                &src,
                SourceConflict::ReceiverWins,
                true
            ));
            assert!(may_start_local_playback(
                &src,
                SourceConflict::LastWins,
                true
            ));
        }
    }
}

#[cfg(test)]
mod supervise_tests {
    use super::{ZONE_MAX_CONSECUTIVE_CRASHES, supervise};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};

    #[tokio::test(start_paused = true)]
    #[allow(clippy::manual_assert)] // the panic deliberately simulates a zone crash
    async fn crashed_zone_restarts_then_recovers() {
        let calls = Arc::new(AtomicU32::new(0));
        let c = calls.clone();
        // First attempt panics → supervisor catches it, backs off, restarts; the
        // second attempt returns Ok and supervision ends. (Without catch_unwind the
        // panic would kill the zone task silently — the bug this guards.)
        supervise(1, async move || {
            if c.fetch_add(1, Ordering::SeqCst) == 0 {
                panic!("boom");
            }
            Ok(())
        })
        .await;
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn repeated_crashes_give_up_without_looping_forever() {
        let calls = Arc::new(AtomicU32::new(0));
        let c = calls.clone();
        // Always panics → supervisor gives up after the consecutive-crash cap rather
        // than busy-restarting a deterministic panic forever.
        supervise(1, async move || {
            c.fetch_add(1, Ordering::SeqCst);
            panic!("always");
        })
        .await;
        assert_eq!(calls.load(Ordering::SeqCst), ZONE_MAX_CONSECUTIVE_CRASHES);
    }
}
