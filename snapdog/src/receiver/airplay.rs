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

use crate::config::{AirplayConfig, AirplayMode};
use crate::net::listener::AIRPLAY_BASE_PORT;

/// The protocol mode a receiver really runs.
///
/// `[airplay].mode` only takes effect in builds with the `ap2` feature. Without it
/// shairplay is `AirPlay` 1 only, whatever the configuration says. Released binaries
/// carry `ap2` (the `full` feature), so there the default `airplay2` is live. The
/// start-up log line prints this value, so an operator can see which one they got.
#[must_use]
pub(crate) const fn effective_mode(configured: AirplayMode) -> AirplayMode {
    if cfg!(feature = "ap2") {
        configured
    } else {
        AirplayMode::Airplay1
    }
}

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
            builder = builder.pairing_store(pairing_store(&self.state_dir));
        }

        let mut server = builder.build(handler)?;
        server.start().await?;

        let port = server.service_info().port;
        tracing::info!(
            zone = %self.airplay_name,
            port,
            mode = ?effective_mode(self.config.mode),
            "AirPlay started"
        );

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
        // saturating_sub: shairplay can deliver current < start (seek-before-start)
        // or end < start; plain `-` would panic in debug / wrap in release.
        let position_ms = (u64::from(current.saturating_sub(start)) * 1000) / sample_rate;
        let duration_ms = (u64::from(end.saturating_sub(start)) * 1000) / sample_rate;
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

/// Pairing keys of `AirPlay` 2 senders, kept in one JSON file.
///
/// Every zone has its own receiver, and all of them use the same file. Each zone used to
/// load a private copy at start and rewrite the whole file on every change, so a key
/// stored through one zone was overwritten by the next change through another zone.
/// [`FilePairingStore::shared`] hands every zone the same store per file, so there is one
/// map and one writer. Writes are serialised by the map's lock and go through a temporary
/// file and a rename, so a crash never leaves a half-written file.
#[cfg(feature = "ap2")]
struct FilePairingStore {
    path: std::path::PathBuf,
    keys: std::sync::Mutex<std::collections::HashMap<String, [u8; 32]>>,
}

/// The pairing store every zone of this process uses (`<state_dir>/airplay-pairing`).
#[cfg(feature = "ap2")]
fn pairing_store(state_dir: &str) -> Arc<FilePairingStore> {
    FilePairingStore::shared(std::path::PathBuf::from(state_dir).join("airplay-pairing"))
}

#[cfg(feature = "ap2")]
type PairingKeys = std::collections::HashMap<String, [u8; 32]>;

#[cfg(feature = "ap2")]
impl FilePairingStore {
    /// The store for `path`, shared by every caller in this process.
    fn shared(path: std::path::PathBuf) -> Arc<Self> {
        use std::sync::{Mutex, OnceLock, PoisonError};
        static STORES: OnceLock<
            Mutex<std::collections::HashMap<std::path::PathBuf, Arc<FilePairingStore>>>,
        > = OnceLock::new();
        let mut stores = STORES
            .get_or_init(Mutex::default)
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        Arc::clone(
            stores
                .entry(path.clone())
                .or_insert_with(|| Arc::new(Self::load(path))),
        )
    }

    /// Read the file. A missing file is an empty store. A file that cannot be read or
    /// parsed is reported and also starts empty, which the next change overwrites.
    fn load(path: std::path::PathBuf) -> Self {
        let keys = match std::fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str::<PairingKeys>(&text).unwrap_or_else(|e| {
                tracing::warn!(path = %path.display(), error = %e, "AirPlay pairing file is not valid, starting without pairings");
                PairingKeys::default()
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => PairingKeys::default(),
            Err(e) => {
                tracing::warn!(path = %path.display(), error = %e, "could not read the AirPlay pairing file, starting without pairings");
                PairingKeys::default()
            }
        };
        Self {
            path,
            keys: std::sync::Mutex::new(keys),
        }
    }

    fn save(&self, keys: &PairingKeys) {
        if let Err(e) = self.try_save(keys) {
            tracing::warn!(path = %self.path.display(), error = %e, "could not save the AirPlay pairing keys");
        }
    }

    fn try_save(&self, keys: &PairingKeys) -> std::io::Result<()> {
        let json = serde_json::to_string_pretty(keys).map_err(std::io::Error::other)?;
        let tmp = self.path.with_extension("tmp");
        std::fs::write(&tmp, json)?;
        std::fs::rename(&tmp, &self.path)
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

    // ── AP2 pairing store shared by all zones ─────────────────────────

    #[cfg(feature = "ap2")]
    fn key(byte: u8) -> [u8; 32] {
        [byte; 32]
    }

    #[cfg(feature = "ap2")]
    fn on_disk(path: &std::path::Path) -> PairingKeys {
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    /// The bug: zone A and zone B each held a private copy, so the later write of zone A
    /// dropped the key zone B had stored. All zones must see one store.
    #[cfg(feature = "ap2")]
    #[test]
    fn keys_paired_through_two_zones_both_reach_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("airplay-pairing");
        let state_dir = dir.path().to_str().unwrap();
        let zone_a = pairing_store(state_dir);
        let zone_b = pairing_store(state_dir);
        zone_b.put("phone-b", key(2));
        zone_a.put("laptop-a", key(1));
        let stored = on_disk(&path);
        assert_eq!(
            stored.get("phone-b"),
            Some(&key(2)),
            "zone B's key was lost"
        );
        assert_eq!(stored.get("laptop-a"), Some(&key(1)));
        // Both zones answer for both senders, also after a restart.
        assert_eq!(zone_a.get("phone-b"), Some(key(2)));
        let restarted = FilePairingStore::load(path);
        assert_eq!(restarted.get("phone-b"), Some(key(2)));
        assert_eq!(restarted.get("laptop-a"), Some(key(1)));
    }

    #[cfg(feature = "ap2")]
    #[test]
    fn shared_store_is_one_per_file() {
        let dir = tempfile::tempdir().unwrap();
        let first = FilePairingStore::shared(dir.path().join("a"));
        let again = FilePairingStore::shared(dir.path().join("a"));
        let other = FilePairingStore::shared(dir.path().join("b"));
        assert!(Arc::ptr_eq(&first, &again));
        assert!(!Arc::ptr_eq(&first, &other));
    }

    #[cfg(feature = "ap2")]
    #[test]
    fn concurrent_pairings_are_all_kept() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("airplay-pairing");
        let handles: Vec<_> = (0..16u8)
            .map(|n| {
                let path = path.clone();
                std::thread::spawn(move || {
                    FilePairingStore::shared(path).put(&format!("sender-{n}"), key(n));
                })
            })
            .collect();
        for handle in handles {
            handle.join().unwrap();
        }
        assert_eq!(on_disk(&path).len(), 16);
    }

    #[cfg(feature = "ap2")]
    #[test]
    fn writes_are_atomic_and_remove_is_persisted() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("airplay-pairing");
        let store = FilePairingStore::shared(path.clone());
        store.put("one", key(1));
        store.put("two", key(2));
        store.remove("one");
        assert_eq!(on_disk(&path).keys().collect::<Vec<_>>(), ["two"]);
        let leftovers: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(
            leftovers,
            ["airplay-pairing"],
            "no temporary file may stay behind"
        );
    }

    #[cfg(feature = "ap2")]
    #[test]
    fn unreadable_pairing_file_starts_empty_and_is_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("airplay-pairing");
        std::fs::write(&path, "{ not json").unwrap();
        let store = FilePairingStore::load(path.clone());
        assert_eq!(store.get("x"), None);
        store.put("x", key(9));
        assert_eq!(on_disk(&path).get("x"), Some(&key(9)));
    }

    /// A write error must not lose the key for this run or panic the receiver.
    #[cfg(feature = "ap2")]
    #[test]
    fn failing_write_keeps_the_key_in_memory() {
        let dir = tempfile::tempdir().unwrap();
        let store = FilePairingStore::load(dir.path().join("missing-dir").join("airplay-pairing"));
        store.put("x", key(4));
        assert_eq!(store.get("x"), Some(key(4)));
    }

    /// `[airplay].mode` is compiled out without `ap2`: the default `airplay2` must
    /// then resolve to `AirPlay` 1, and with `ap2` both modes are honoured. This is
    /// the pair of facts that made the release-build switch a behaviour change.
    #[test]
    fn effective_mode_follows_the_ap2_feature() {
        if cfg!(feature = "ap2") {
            assert_eq!(effective_mode(AirplayMode::Airplay2), AirplayMode::Airplay2);
            assert_eq!(effective_mode(AirplayMode::Airplay1), AirplayMode::Airplay1);
        } else {
            assert_eq!(effective_mode(AirplayMode::Airplay2), AirplayMode::Airplay1);
            assert_eq!(effective_mode(AirplayMode::Airplay1), AirplayMode::Airplay1);
        }
    }

    /// `AirPlay` 2 is the documented default and `airplay1` is the documented way back
    /// (docs/upgrading.md). Changing either is a user-visible change of the released
    /// receiver: update the upgrade notes and mark the commit as breaking.
    #[test]
    fn airplay_mode_defaults_to_airplay2_and_airplay1_stays_selectable() {
        assert_eq!(AirplayConfig::default().mode, AirplayMode::Airplay2);
        let configured: AirplayConfig = toml::from_str(r#"mode = "airplay1""#).unwrap();
        assert_eq!(configured.mode, AirplayMode::Airplay1);
        let unset: AirplayConfig = toml::from_str("").unwrap();
        assert_eq!(unset.mode, AirplayMode::Airplay2);
    }

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
