// Copyright (C) 2026 Fabian Schmieder

//! Spotify Connect receiver implementing [`ReceiverProvider`].
//!
//! Uses [`librespot`] 0.8 for Zeroconf discovery, Spotify Connect protocol,
//! and audio decoding. Audio is delivered as F32 interleaved PCM via a
//! custom sink that writes to the receiver's audio channel.

use std::sync::Arc;

use anyhow::Result;

use librespot_connect::{ConnectConfig, Spirc};

use librespot_core::SessionConfig;

use librespot_core::session::Session;

use librespot_discovery::{DeviceType, Discovery};

use librespot_metadata::audio::item::UniqueFields;

use librespot_playback::audio_backend::{Sink, SinkError, SinkResult};

use librespot_playback::config::PlayerConfig;

use librespot_playback::convert::Converter;

use librespot_playback::decoder::AudioPacket;

use librespot_playback::mixer::{MixerConfig, NoOpVolume};

use librespot_playback::player::{Player, PlayerEvent};

use super::{
    AudioFormat, AudioSender, ReceiverEvent, ReceiverEventTx, ReceiverProvider, RemoteCommand,
    RemoteControl,
};
use crate::config::SpotifyConfig;

// ── SpotifyReceiver ───────────────────────────────────────────

/// Spotify Connect always outputs 44.1 kHz.
const SPOTIFY_SAMPLE_RATE: u32 = 44100;

/// Interval for polling session validity.
const SESSION_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(1);

/// Spotify Connect receiver wrapping librespot.
pub struct SpotifyReceiver {
    config: SpotifyConfig,
    zone_index: usize,
    task: Option<tokio::task::JoinHandle<()>>,
}

impl SpotifyReceiver {
    /// Create a new (stopped) Spotify Connect receiver for the given zone.
    pub const fn new(config: SpotifyConfig, zone_index: usize) -> Self {
        Self {
            config,
            zone_index,
            task: None,
        }
    }
}

impl ReceiverProvider for SpotifyReceiver {
    fn name(&self) -> &'static str {
        "Spotify Connect"
    }

    async fn start(&mut self, audio_tx: AudioSender, event_tx: ReceiverEventTx) -> Result<()> {
        let config = self.config.clone();
        let zone_index = self.zone_index;
        let task = tokio::spawn(async move {
            if let Err(e) = run_spotify(config, zone_index, audio_tx, event_tx).await {
                tracing::error!(zone = zone_index, error = %e, "Spotify receiver failed");
            }
        });
        self.task = Some(task);
        tracing::info!(zone = self.zone_index, name = %self.config.name, "Spotify Connect receiver started");
        Ok(())
    }

    async fn stop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }

    fn is_running(&self) -> bool {
        self.task.as_ref().is_some_and(|t| !t.is_finished())
    }
}

// ── Remote control bridge ─────────────────────────────────────

/// Bridges [`RemoteCommand`] to Spirc methods.
struct SpircRemote(Arc<Spirc>);

impl RemoteControl for SpircRemote {
    fn send_command(&self, cmd: RemoteCommand) -> Result<()> {
        match cmd {
            RemoteCommand::Play => self.0.play()?,
            RemoteCommand::Pause | RemoteCommand::Stop => self.0.pause()?, // Spirc has no stop
            RemoteCommand::NextTrack => self.0.next()?,
            RemoteCommand::PreviousTrack => self.0.prev()?,
            RemoteCommand::SetVolume(v) => {
                let volume = (u16::from(v) * u16::MAX) / 100;
                self.0.set_volume(volume)?;
            }
            RemoteCommand::ToggleShuffle => {
                // Spirc::shuffle takes a bool — we toggle by passing true
                // (Spirc reshuffles if already shuffled, which is acceptable)
                self.0.shuffle(true)?;
            }
            RemoteCommand::ToggleRepeat => {
                // Toggle context repeat
                self.0.repeat(true)?;
            }
        }
        Ok(())
    }
}

// ── Main loop ─────────────────────────────────────────────────

async fn run_spotify(
    config: SpotifyConfig,
    zone_index: usize,
    audio_tx: AudioSender,
    event_tx: ReceiverEventTx,
) -> Result<()> {
    loop {
        tracing::info!(zone = zone_index, name = %config.name, "Waiting for Spotify Connect client");

        let device_id = config.device_id();
        let mut discovery = Discovery::builder(&config.name, &device_id)
            .device_type(DeviceType::Speaker)
            .launch()
            .map_err(|e| anyhow::anyhow!("Discovery failed: {e}"))?;

        use futures_util::StreamExt;
        let Some(credentials) = discovery.next().await else {
            return Ok(());
        };

        tracing::info!(zone = zone_index, "Spotify client connected");

        let session = Session::new(SessionConfig::default(), None);
        session
            .connect(credentials.clone(), true)
            .await
            .map_err(|e| anyhow::anyhow!("Session connect failed: {e}"))?;

        let tx = audio_tx.clone();
        let player = Player::new(
            PlayerConfig {
                bitrate: config.bitrate_enum(),
                ..PlayerConfig::default()
            },
            session.clone(),
            Box::new(NoOpVolume),
            move || Box::new(ChannelSink::new(tx)) as Box<dyn Sink>,
        );

        let mut event_rx = player.get_player_event_channel();

        let mixer = librespot_playback::mixer::find(None)
            .ok_or_else(|| anyhow::anyhow!("No default mixer available"))?(
            MixerConfig::default()
        )
        .map_err(|e| anyhow::anyhow!("Mixer failed: {e}"))?;

        let (spirc, spirc_task) = Spirc::new(
            ConnectConfig {
                name: config.name.clone(),
                device_type: DeviceType::Speaker,
                initial_volume: u16::MAX / 2,
                ..ConnectConfig::default()
            },
            session.clone(),
            credentials,
            player.clone(),
            mixer,
        )
        .await
        .map_err(|e| anyhow::anyhow!("Spirc failed: {e}"))?;

        let _ = event_tx.try_send(ReceiverEvent::SessionStarted {
            format: AudioFormat {
                sample_rate: SPOTIFY_SAMPLE_RATE,
                channels: 2,
            },
        });

        // Send remote control handle — enables play/pause/next/prev from SnapDog UI
        let spirc = Arc::new(spirc);
        let _ = event_tx.try_send(ReceiverEvent::RemoteAvailable {
            remote: Arc::new(SpircRemote(spirc.clone())),
        });

        let spirc_handle = tokio::spawn(spirc_task);

        // Track last known duration for Paused events
        let mut last_duration_ms: u64 = 0;

        loop {
            tokio::select! {
                event = event_rx.recv() => {
                    match event {
                        Some(event) => handle_player_event(event, &event_tx, &mut last_duration_ms).await,
                        None => break,
                    }
                }
                () = tokio::time::sleep(SESSION_POLL_INTERVAL) => {
                    if player.is_invalid() {
                        tracing::info!(zone = zone_index, "Spotify session ended");
                        break;
                    }
                }
            }
        }

        let _ = event_tx.try_send(ReceiverEvent::SessionEnded);
        let _ = spirc.shutdown();
        spirc_handle.abort();

        tracing::info!(
            zone = zone_index,
            "Spotify session closed, restarting discovery"
        );
    }
}

// ── Event handling ────────────────────────────────────────────

async fn handle_player_event(
    event: PlayerEvent,
    event_tx: &ReceiverEventTx,
    last_duration_ms: &mut u64,
) {
    match event {
        // TrackChanged fires on every track switch — contains full metadata
        PlayerEvent::TrackChanged { audio_item } => {
            let (artist, album) = match &audio_item.unique_fields {
                UniqueFields::Track { artists, album, .. } => (
                    artists.first().map(|a| a.name.clone()).unwrap_or_default(),
                    album.clone(),
                ),
                UniqueFields::Episode { show_name, .. } => (show_name.clone(), String::new()),
                UniqueFields::Local { artists, album, .. } => (
                    artists.clone().unwrap_or_default(),
                    album.clone().unwrap_or_default(),
                ),
            };

            *last_duration_ms = u64::from(audio_item.duration_ms);

            let _ = event_tx.try_send(ReceiverEvent::Metadata {
                title: audio_item.name.clone(),
                artist,
                album,
            });

            // Cover art from AudioItem covers
            if let Some(cover) = audio_item.covers.first() {
                if let Some((bytes, _)) = crate::state::cover::fetch_cover(&cover.url).await {
                    let _ = event_tx.try_send(ReceiverEvent::CoverArt { bytes });
                }
            }
        }

        PlayerEvent::Playing { position_ms, .. }
        | PlayerEvent::Paused { position_ms, .. }
        | PlayerEvent::Seeked { position_ms, .. }
        | PlayerEvent::PositionCorrection { position_ms, .. } => {
            let _ = event_tx.try_send(ReceiverEvent::Progress {
                position_ms: u64::from(position_ms),
                duration_ms: *last_duration_ms,
            });
        }

        PlayerEvent::VolumeChanged { volume } => {
            let percent = (i32::from(volume) * 100) / i32::from(u16::MAX);
            let _ = event_tx.try_send(ReceiverEvent::Volume { percent });
        }

        _ => {}
    }
}

// ── Custom audio sink ─────────────────────────────────────────

struct ChannelSink {
    tx: AudioSender,
}

impl ChannelSink {
    const fn new(tx: AudioSender) -> Self {
        Self { tx }
    }
}

impl Sink for ChannelSink {
    fn write(&mut self, packet: AudioPacket, _converter: &mut Converter) -> SinkResult<()> {
        let f32_samples = match packet {
            AudioPacket::Samples(samples) => samples.iter().map(|&s| s as f32).collect(),
            AudioPacket::Raw(_) => return Ok(()),
        };
        self.tx
            .try_send(f32_samples)
            .map_err(|e| SinkError::OnWrite(format!("Channel send failed: {e}")))?;
        Ok(())
    }
}
