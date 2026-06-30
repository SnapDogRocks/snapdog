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
    #[must_use]
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
    use futures_util::StreamExt;

    loop {
        tracing::info!(zone = zone_index, name = %config.name, "Waiting for Spotify Connect client");

        let device_id = config.device_id();
        let mut discovery = Discovery::builder(&config.name, &device_id)
            .device_type(DeviceType::Speaker)
            .launch()
            .map_err(|e| anyhow::anyhow!("Discovery failed: {e}"))?;

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

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc;

    /// Spotify volume-out math (IT-T72): librespot u16 volume → 0–100 percent.
    #[tokio::test]
    async fn volume_changed_maps_to_percent() {
        async fn percent_for(volume: u16) -> i32 {
            let (tx, mut rx) = mpsc::channel(4);
            let mut dur = 0u64;
            handle_player_event(PlayerEvent::VolumeChanged { volume }, &tx, &mut dur).await;
            match rx.try_recv().expect("volume event emitted") {
                ReceiverEvent::Volume { percent } => percent,
                _ => panic!("expected ReceiverEvent::Volume"),
            }
        }
        assert_eq!(percent_for(u16::MAX).await, 100);
        assert_eq!(percent_for(0).await, 0);
        assert_eq!(percent_for(u16::MAX / 2).await, 49); // (32767*100)/65535 = 49
    }

    /// librespot 0.8 `AudioPacket::Samples` is already normalized to [-1, 1]; the
    /// `ChannelSink` must be a plain f64→f32 cast, NOT a `/32768` rescale (IT-T72).
    #[test]
    fn channel_sink_casts_f64_to_f32_without_rescaling() {
        let (tx, mut rx) = mpsc::channel(4);
        let mut sink = ChannelSink::new(tx);
        let mut conv = Converter::new(None);
        sink.write(AudioPacket::Samples(vec![0.5, -0.25, 1.0, 0.0]), &mut conv)
            .expect("sink write ok");
        assert_eq!(
            rx.try_recv().expect("samples forwarded"),
            vec![0.5_f32, -0.25, 1.0, 0.0]
        );
    }

    // ── IT-T72: PlayerEvent → ReceiverEvent mapper goldens ────────────
    // (ReceiverEvent has no Debug/PartialEq derive → assert via matches!/destructure.)

    use librespot_core::SpotifyId;
    use librespot_core::date::Date;
    use librespot_core::spotify_uri::SpotifyUri;
    use librespot_metadata::artist::ArtistsWithRole;
    use librespot_metadata::audio::file::AudioFiles;
    use librespot_metadata::audio::item::AudioItem;

    fn track_uri() -> SpotifyUri {
        SpotifyUri::Track {
            id: SpotifyId::from_base62("4iV5W9uYEdYUVa79Axb7Rh").unwrap(),
        }
    }

    fn audio_item(name: &str, duration_ms: u32, unique_fields: UniqueFields) -> AudioItem {
        AudioItem {
            track_id: track_uri(),
            uri: String::new(),
            files: AudioFiles(std::collections::HashMap::new()),
            name: name.to_string(),
            covers: vec![], // empty → skips the (networked) cover fetch
            language: vec![],
            duration_ms,
            is_explicit: false,
            availability: Ok(()),
            alternatives: None,
            unique_fields,
        }
    }

    async fn metadata_for(unique_fields: UniqueFields) -> (String, String, String) {
        let (tx, mut rx) = mpsc::channel(4);
        let mut dur = 0u64;
        let item = audio_item("Title", 180_000, unique_fields);
        handle_player_event(
            PlayerEvent::TrackChanged {
                audio_item: Box::new(item),
            },
            &tx,
            &mut dur,
        )
        .await;
        assert_eq!(dur, 180_000, "TrackChanged updates last_duration_ms");
        match rx.try_recv().expect("metadata emitted") {
            ReceiverEvent::Metadata {
                title,
                artist,
                album,
            } => {
                assert!(rx.try_recv().is_err(), "no cover event when covers empty");
                (title, artist, album)
            }
            _ => panic!("expected ReceiverEvent::Metadata"),
        }
    }

    #[tokio::test]
    async fn progress_events_emit_progress_with_last_duration() {
        let makers: [fn(SpotifyUri) -> PlayerEvent; 4] = [
            |t| PlayerEvent::Playing {
                play_request_id: 1,
                track_id: t,
                position_ms: 5000,
            },
            |t| PlayerEvent::Paused {
                play_request_id: 1,
                track_id: t,
                position_ms: 5000,
            },
            |t| PlayerEvent::Seeked {
                play_request_id: 1,
                track_id: t,
                position_ms: 5000,
            },
            |t| PlayerEvent::PositionCorrection {
                play_request_id: 1,
                track_id: t,
                position_ms: 5000,
            },
        ];
        for make in makers {
            let (tx, mut rx) = mpsc::channel(4);
            let mut dur = 240_000u64;
            handle_player_event(make(track_uri()), &tx, &mut dur).await;
            assert!(matches!(
                rx.try_recv().expect("progress emitted"),
                ReceiverEvent::Progress {
                    position_ms: 5000,
                    duration_ms: 240_000
                }
            ));
        }
    }

    #[tokio::test]
    async fn track_changed_track_uses_first_artist_and_album() {
        let (title, artist, album) = metadata_for(UniqueFields::Track {
            artists: ArtistsWithRole(vec![]), // empty → "" via unwrap_or_default
            album: "Album X".into(),
            album_artists: vec![],
            popularity: 0,
            number: 0,
            disc_number: 0,
        })
        .await;
        assert_eq!(
            (title.as_str(), artist.as_str(), album.as_str()),
            ("Title", "", "Album X")
        );
    }

    #[tokio::test]
    async fn track_changed_episode_uses_show_name_as_artist() {
        let (_, artist, album) = metadata_for(UniqueFields::Episode {
            description: String::new(),
            publish_time: Date::from_timestamp_ms(0).unwrap(),
            show_name: "My Show".into(),
        })
        .await;
        assert_eq!((artist.as_str(), album.as_str()), ("My Show", ""));
    }

    #[tokio::test]
    async fn track_changed_local_maps_optional_artist_album() {
        let (_, artist, album) = metadata_for(UniqueFields::Local {
            artists: Some("Local Artist".into()),
            album: Some("Local Album".into()),
            album_artists: None,
            number: None,
            disc_number: None,
            path: std::path::PathBuf::new(),
        })
        .await;
        assert_eq!(
            (artist.as_str(), album.as_str()),
            ("Local Artist", "Local Album")
        );

        // None fields → empty strings.
        let (_, artist, album) = metadata_for(UniqueFields::Local {
            artists: None,
            album: None,
            album_artists: None,
            number: None,
            disc_number: None,
            path: std::path::PathBuf::new(),
        })
        .await;
        assert_eq!((artist.as_str(), album.as_str()), ("", ""));
    }

    #[tokio::test]
    async fn unhandled_events_emit_nothing() {
        let (tx, mut rx) = mpsc::channel(4);
        let mut dur = 7u64;
        handle_player_event(
            PlayerEvent::Stopped {
                play_request_id: 0,
                track_id: track_uri(),
            },
            &tx,
            &mut dur,
        )
        .await;
        assert!(rx.try_recv().is_err(), "Stopped emits no ReceiverEvent");
        assert_eq!(dur, 7, "duration unchanged");
    }
}
