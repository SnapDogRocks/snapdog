// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! `AirPlay` 1 + 2 receiver implementing [`ReceiverProvider`].
//!
//! Bridges the [`shairplay`] crate's callback-based API into `SnapDog`'s
//! channel-based receiver model. Audio is delivered as F32 interleaved PCM.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use anyhow::Result;

use shairplay::RaopServer;

#[cfg(feature = "ap2")]
use shairplay::{BindConfig, PairingStore};

use super::{AudioFormat, AudioSender, ReceiverEvent, ReceiverEventTx, ReceiverProvider};

use crate::config::AirplayConfig;

/// Base port for `AirPlay` receivers (each zone gets base + `zone_index`).
const AIRPLAY_BASE_PORT: u16 = 7000;

/// `AirPlay` volume minimum in dB (silence).
const AIRPLAY_VOLUME_MIN_DB: f32 = -144.0;
/// `AirPlay` volume maximum in dB.
const AIRPLAY_VOLUME_MAX_DB: f32 = 30.0;

// ── AirPlayReceiver ───────────────────────────────────────────

/// `AirPlay` receiver wrapping [`shairplay::RaopServer`].
pub struct AirPlayReceiver {
    config: AirplayConfig,
    zone_index: usize,
    airplay_name: String,
    #[cfg(feature = "ap2")]
    state_dir: String,
    server: Option<RaopServer>,
}

impl AirPlayReceiver {
    /// Create a new (stopped) `AirPlay` receiver for the given zone.
    #[must_use]
    pub const fn new(
        config: AirplayConfig,
        zone_index: usize,
        airplay_name: String,
        #[cfg(feature = "ap2")] state_dir: String,
    ) -> Self {
        Self {
            config,
            zone_index,
            airplay_name,
            #[cfg(feature = "ap2")]
            state_dir,
            server: None,
        }
    }
}

impl ReceiverProvider for AirPlayReceiver {
    fn name(&self) -> &'static str {
        "AirPlay"
    }

    async fn start(&mut self, audio_tx: AudioSender, event_tx: ReceiverEventTx) -> Result<()> {
        let mut hwaddr = detect_hwaddr();
        hwaddr[5] = hwaddr[5].wrapping_add(self.zone_index as u8);

        let handler = Arc::new(BridgeHandler {
            audio_tx,
            event_tx,
            sample_rate: AtomicU32::new(44100),
        });

        let mut builder = RaopServer::builder()
            .name(&self.airplay_name)
            .hwaddr(hwaddr.to_vec())
            .port(AIRPLAY_BASE_PORT + self.zone_index as u16)
            .max_clients(1);

        if let Some(ref pw) = self.config.password {
            builder = builder.password(pw);
        }

        #[cfg(feature = "ap2")]
        {
            use crate::config::AirplayMode;
            let mode = match self.config.mode {
                AirplayMode::Airplay1 => shairplay::AirPlayMode::AirPlay1,
                AirplayMode::Airplay2 => shairplay::AirPlayMode::AirPlay2,
            };
            builder = builder.mode(mode);
        }

        #[cfg(feature = "ap2")]
        if let Some(ref addrs) = self.config.bind {
            builder = builder.bind(BindConfig::new().addrs(addrs.clone()));
        }

        #[cfg(feature = "ap2")]
        {
            let path = std::path::PathBuf::from(&self.state_dir).join("airplay-pairing");
            builder = builder.pairing_store(Arc::new(FilePairingStore::new(path)));
        }

        let mut server = builder.build(handler)?;
        server.start().await?;

        let port = server.service_info().port;
        tracing::info!(zone = %self.airplay_name, port, "AirPlay started");

        self.server = Some(server);
        Ok(())
    }

    async fn stop(&mut self) {
        if let Some(ref mut server) = self.server {
            server.stop().await;
        }
        self.server = None;
    }

    fn is_running(&self) -> bool {
        self.server
            .as_ref()
            .is_some_and(shairplay::RaopServer::is_running)
    }
}

// ── AudioHandler bridge (metadata + lifecycle, off audio path) ─

struct BridgeHandler {
    audio_tx: AudioSender,
    event_tx: ReceiverEventTx,
    sample_rate: AtomicU32,
}

impl shairplay::AudioHandler for BridgeHandler {
    fn audio_init(&self, format: shairplay::AudioFormat) -> Box<dyn shairplay::AudioSession> {
        tracing::info!(
            channels = format.channels,
            sample_rate = format.sample_rate,
            "Session started"
        );
        self.sample_rate
            .store(format.sample_rate, Ordering::Relaxed);
        let _ = self.event_tx.try_send(ReceiverEvent::SessionStarted {
            format: AudioFormat {
                sample_rate: format.sample_rate,
                channels: u16::from(format.channels),
            },
        });
        Box::new(BridgeSession {
            audio_tx: self.audio_tx.clone(),
        })
    }

    fn on_volume(&self, volume: f32) {
        let percent = if volume <= AIRPLAY_VOLUME_MIN_DB {
            0
        } else {
            ((volume + AIRPLAY_VOLUME_MAX_DB) / AIRPLAY_VOLUME_MAX_DB * 100.0).clamp(0.0, 100.0)
                as i32
        };
        tracing::debug!(percent, "AirPlay volume");
        let _ = self.event_tx.try_send(ReceiverEvent::Volume { percent });
    }

    fn on_metadata(&self, metadata: &shairplay::TrackMetadata) {
        let title = metadata.title.clone().unwrap_or_default();
        let artist = metadata.artist.clone().unwrap_or_default();
        let album = metadata.album.clone().unwrap_or_default();
        tracing::debug!(title = %title, artist = %artist, "AirPlay metadata");
        let _ = self.event_tx.try_send(ReceiverEvent::Metadata {
            title,
            artist,
            album,
        });
    }

    fn on_coverart(&self, coverart: &[u8]) {
        tracing::debug!(size = coverart.len() / 1024, "AirPlay cover art (KB)");
        let _ = self.event_tx.try_send(ReceiverEvent::CoverArt {
            bytes: coverart.to_vec(),
        });
    }

    fn on_progress(&self, start: u32, current: u32, end: u32) {
        let sample_rate = u64::from(self.sample_rate.load(Ordering::Relaxed));
        let position_ms = (u64::from(current - start) * 1000) / sample_rate;
        let duration_ms = (u64::from(end - start) * 1000) / sample_rate;
        let _ = self.event_tx.try_send(ReceiverEvent::Progress {
            position_ms,
            duration_ms,
        });
    }

    fn on_remote_control(&self, remote: Arc<dyn shairplay::RemoteControl>) {
        tracing::debug!("AirPlay remote control available");
        let _ = self.event_tx.try_send(ReceiverEvent::RemoteAvailable {
            remote: Arc::new(ShairplayRemoteBridge(remote)),
        });
    }

    fn on_client_disconnected(&self, _addr: &str) {
        tracing::info!("Session ended");
        let _ = self.event_tx.try_send(ReceiverEvent::SessionEnded);
    }
}

// ── AudioSession bridge (hot path — PCM only) ────────────────

struct BridgeSession {
    audio_tx: AudioSender,
}

impl shairplay::AudioSession for BridgeSession {
    fn audio_process(&mut self, samples: &[f32]) {
        let _ = self.audio_tx.try_send(samples.to_vec());
    }
}

// ── RemoteControl bridge ──────────────────────────────────────

struct ShairplayRemoteBridge(Arc<dyn shairplay::RemoteControl>);

impl super::RemoteControl for ShairplayRemoteBridge {
    fn send_command(&self, cmd: super::RemoteCommand) -> Result<()> {
        let sp_cmd = match cmd {
            super::RemoteCommand::Play => shairplay::RemoteCommand::Play,
            super::RemoteCommand::Pause => shairplay::RemoteCommand::Pause,
            super::RemoteCommand::NextTrack => shairplay::RemoteCommand::NextTrack,
            super::RemoteCommand::PreviousTrack => shairplay::RemoteCommand::PreviousTrack,
            super::RemoteCommand::Stop => shairplay::RemoteCommand::Stop,
            super::RemoteCommand::SetVolume(v) => shairplay::RemoteCommand::SetVolume(v),
            super::RemoteCommand::ToggleShuffle => shairplay::RemoteCommand::ToggleShuffle,
            super::RemoteCommand::ToggleRepeat => shairplay::RemoteCommand::ToggleRepeat,
        };
        self.0.send_command(sp_cmd).map_err(Into::into)
    }
}

// ── FilePairingStore (AP2 key persistence) ────────────────────

#[cfg(feature = "ap2")]
struct FilePairingStore {
    path: std::path::PathBuf,
    keys: std::sync::Mutex<std::collections::HashMap<String, [u8; 32]>>,
}

#[cfg(feature = "ap2")]
impl FilePairingStore {
    fn new(path: std::path::PathBuf) -> Self {
        let keys = std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        Self {
            path,
            keys: std::sync::Mutex::new(keys),
        }
    }

    fn save(&self, keys: &std::collections::HashMap<String, [u8; 32]>) {
        if let Ok(json) = serde_json::to_string_pretty(keys) {
            let _ = std::fs::write(&self.path, json);
        }
    }
}

#[cfg(feature = "ap2")]
impl PairingStore for FilePairingStore {
    fn get(&self, device_id: &str) -> Option<[u8; 32]> {
        self.keys.lock().ok()?.get(device_id).copied()
    }
    fn put(&self, device_id: &str, public_key: [u8; 32]) {
        if let Ok(mut keys) = self.keys.lock() {
            keys.insert(device_id.to_string(), public_key);
            self.save(&keys);
        }
    }
    fn remove(&self, device_id: &str) {
        if let Ok(mut keys) = self.keys.lock() {
            keys.remove(device_id);
            self.save(&keys);
        }
    }
}

// ── Helpers ───────────────────────────────────────────────────

/// Detect the MAC address of the primary network interface.
pub(crate) fn detect_hwaddr() -> [u8; 6] {
    mac_address::get_mac_address().ok().flatten().map_or(
        [0x02, 0x42, 0xAA, 0xBB, 0xCC, 0x00],
        mac_address::MacAddress::bytes,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use shairplay::AudioHandler;
    use tokio::sync::mpsc;

    #[test]
    fn test_detect_hwaddr_not_all_zeros() {
        let addr = detect_hwaddr();
        assert_ne!(addr, [0, 0, 0, 0, 0, 0]);
    }

    #[tokio::test]
    async fn airplay_volume_zero_db_is_full_scale() {
        let (audio_tx, _) = mpsc::channel(16);
        let (event_tx, mut event_rx) = mpsc::channel(16);
        let handler = BridgeHandler {
            audio_tx,
            event_tx,
            sample_rate: AtomicU32::new(44100),
        };
        // 0 dB → 100% — completes the corrected volume golden (RFC IT-0003 §8).
        handler.on_volume(0.0);
        match event_rx.recv().await {
            Some(ReceiverEvent::Volume { percent }) => assert_eq!(percent, 100),
            _ => panic!("expected volume event"),
        }
    }

    #[tokio::test]
    async fn test_bridge_handler_volume_mapping() {
        let (audio_tx, _) = mpsc::channel(16);
        let (event_tx, mut event_rx) = mpsc::channel(16);
        let handler = BridgeHandler {
            audio_tx,
            event_tx,
            sample_rate: AtomicU32::new(44100),
        };

        // Minimum (silence) dB -> 0%
        handler.on_volume(-144.0);
        if let Some(ReceiverEvent::Volume { percent }) = event_rx.recv().await {
            assert_eq!(percent, 0);
        } else {
            panic!("Expected volume event");
        }

        // Below minimum dB -> 0%
        handler.on_volume(-200.0);
        if let Some(ReceiverEvent::Volume { percent }) = event_rx.recv().await {
            assert_eq!(percent, 0);
        } else {
            panic!("Expected volume event");
        }

        // Scale -30 dB -> 0% (lower bound of 0-100 linear mapping)
        handler.on_volume(-30.0);
        if let Some(ReceiverEvent::Volume { percent }) = event_rx.recv().await {
            assert_eq!(percent, 0);
        } else {
            panic!("Expected volume event");
        }

        // Mid-point volume: -15 dB should map to ~50%
        handler.on_volume(-15.0);
        if let Some(ReceiverEvent::Volume { percent }) = event_rx.recv().await {
            assert_eq!(percent, 50);
        } else {
            panic!("Expected volume event");
        }

        // Maximum dB (30 dB) -> 100%
        handler.on_volume(30.0);
        if let Some(ReceiverEvent::Volume { percent }) = event_rx.recv().await {
            assert_eq!(percent, 100);
        } else {
            panic!("Expected volume event");
        }

        // Above maximum dB -> 100%
        handler.on_volume(50.0);
        if let Some(ReceiverEvent::Volume { percent }) = event_rx.recv().await {
            assert_eq!(percent, 100);
        } else {
            panic!("Expected volume event");
        }
    }

    // ── IT-T70/T71: handler callback → ReceiverEvent mappers (pure) ────

    fn make_handler() -> (
        BridgeHandler,
        mpsc::Receiver<Vec<f32>>,
        mpsc::Receiver<ReceiverEvent>,
    ) {
        let (audio_tx, audio_rx) = mpsc::channel(16);
        let (event_tx, event_rx) = mpsc::channel(16);
        (
            BridgeHandler {
                audio_tx,
                event_tx,
                sample_rate: AtomicU32::new(44100),
            },
            audio_rx,
            event_rx,
        )
    }

    #[tokio::test]
    async fn volume_mapping_slope_and_infinities() {
        let (handler, _a, mut ev) = make_handler();
        for (db, expect) in [
            (-7.5f32, 75),
            (-22.5, 25),
            (f32::INFINITY, 100),
            (f32::NEG_INFINITY, 0),
        ] {
            handler.on_volume(db);
            match ev.try_recv() {
                Ok(ReceiverEvent::Volume { percent }) => assert_eq!(percent, expect, "{db} dB"),
                _ => panic!("expected volume event for {db} dB"),
            }
        }
    }

    #[tokio::test]
    async fn audio_init_emits_session_started_and_forwards_pcm() {
        let (handler, mut audio, mut ev) = make_handler();
        let mut session = handler.audio_init(shairplay::AudioFormat {
            codec: shairplay::AudioCodec::Pcm,
            bits: 32,
            channels: 2,
            sample_rate: 48000,
        });
        match ev.try_recv() {
            Ok(ReceiverEvent::SessionStarted { format }) => {
                assert_eq!(format.sample_rate, 48000);
                assert_eq!(format.channels, 2);
            }
            _ => panic!("expected SessionStarted"),
        }
        session.audio_process(&[0.1, -0.2]);
        assert_eq!(audio.try_recv().unwrap(), vec![0.1_f32, -0.2]);
    }

    #[tokio::test]
    async fn metadata_coverart_progress_disconnect_map_to_events() {
        let (handler, _a, mut ev) = make_handler();

        handler.on_metadata(&shairplay::TrackMetadata {
            title: Some("T".into()),
            artist: Some("A".into()),
            album: Some("Al".into()),
            ..Default::default()
        });
        match ev.try_recv() {
            Ok(ReceiverEvent::Metadata {
                title,
                artist,
                album,
            }) => assert_eq!(
                (title.as_str(), artist.as_str(), album.as_str()),
                ("T", "A", "Al")
            ),
            _ => panic!("expected Metadata"),
        }

        // All-None metadata → empty strings (unwrap_or_default).
        handler.on_metadata(&shairplay::TrackMetadata::default());
        match ev.try_recv() {
            Ok(ReceiverEvent::Metadata {
                title,
                artist,
                album,
            }) => assert!(title.is_empty() && artist.is_empty() && album.is_empty()),
            _ => panic!("expected Metadata"),
        }

        handler.on_coverart(b"\xff\xd8\xff");
        match ev.try_recv() {
            Ok(ReceiverEvent::CoverArt { bytes }) => assert_eq!(bytes, b"\xff\xd8\xff"),
            _ => panic!("expected CoverArt"),
        }

        // sample_rate 44100: 44100 frames = 1000 ms, 88200 = 2000 ms.
        handler.on_progress(0, 44100, 88200);
        match ev.try_recv() {
            Ok(ReceiverEvent::Progress {
                position_ms,
                duration_ms,
            }) => assert_eq!((position_ms, duration_ms), (1000, 2000)),
            _ => panic!("expected Progress"),
        }

        handler.on_client_disconnected("1.2.3.4");
        assert!(matches!(ev.try_recv(), Ok(ReceiverEvent::SessionEnded)));
    }

    #[test]
    fn remote_command_round_trips_to_shairplay() {
        use crate::receiver::{RemoteCommand, RemoteControl};

        struct FakeRemote(std::sync::Mutex<Vec<shairplay::RemoteCommand>>);
        impl shairplay::RemoteControl for FakeRemote {
            fn send_command(
                &self,
                cmd: shairplay::RemoteCommand,
            ) -> std::result::Result<(), shairplay::ShairplayError> {
                self.0.lock().unwrap().push(cmd);
                Ok(())
            }
            fn available_commands(&self) -> Vec<shairplay::RemoteCommand> {
                vec![]
            }
        }
        let fake = Arc::new(FakeRemote(std::sync::Mutex::new(vec![])));
        let bridge = ShairplayRemoteBridge(fake.clone());
        let cases = [
            (RemoteCommand::Play, shairplay::RemoteCommand::Play),
            (RemoteCommand::Pause, shairplay::RemoteCommand::Pause),
            (
                RemoteCommand::NextTrack,
                shairplay::RemoteCommand::NextTrack,
            ),
            (
                RemoteCommand::PreviousTrack,
                shairplay::RemoteCommand::PreviousTrack,
            ),
            (RemoteCommand::Stop, shairplay::RemoteCommand::Stop),
            (
                RemoteCommand::SetVolume(42),
                shairplay::RemoteCommand::SetVolume(42),
            ),
            (
                RemoteCommand::ToggleShuffle,
                shairplay::RemoteCommand::ToggleShuffle,
            ),
            (
                RemoteCommand::ToggleRepeat,
                shairplay::RemoteCommand::ToggleRepeat,
            ),
        ];
        let expected: Vec<_> = cases.iter().map(|(_, sp)| sp.clone()).collect();
        for (snap, _) in cases {
            bridge.send_command(snap).unwrap();
        }
        assert_eq!(*fake.0.lock().unwrap(), expected);
    }
}
