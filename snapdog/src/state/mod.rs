// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! Application state management.
//!
//! In-memory state for zones and clients, persisted to JSON file.
//! Thread-safe via `Arc<RwLock<_>>` for concurrent access from API, MQTT, audio pipeline.

pub mod cover;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use crate::config::AppConfig;

/// Thread-safe shared state handle.
pub type SharedState = Arc<RwLock<Store>>;

/// Create a new state store initialized from config, optionally loading persisted state.
///
/// # Errors
///
/// Returns an error if the persisted state file cannot be read or parsed.
pub fn init(config: &AppConfig, persist_path: Option<&Path>) -> Result<SharedState> {
    let mut store = Store::from_config(config);

    if let Some(path) = persist_path {
        if path.exists() {
            store.load(path)?;
            tracing::info!(path = %path.display(), "Restored persisted state");
        }
    }

    Ok(Arc::new(RwLock::new(store)))
}

/// Central application state.
///
/// Holds all zone and client state in memory, with optional JSON persistence.
/// Initialized from [`AppConfig`] at startup, then mutated by the audio pipeline,
/// Snapcast sync, and control interfaces (API, MQTT, KNX).
#[derive(Debug, Serialize, Deserialize)]
pub struct Store {
    /// Zone states keyed by 1-based zone index.
    pub zones: HashMap<usize, ZoneState>,
    /// Client states keyed by 1-based client index.
    pub clients: HashMap<usize, ClientState>,
    /// Path for JSON persistence. `None` disables auto-save.
    #[serde(skip)]
    persist_path: Option<PathBuf>,
    /// Whether the state has changed since the last persistence.
    #[serde(skip)]
    pub dirty: bool,
}

/// Runtime state of a single audio zone.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZoneState {
    /// Human-readable zone name from config.
    pub name: String,
    /// Emoji icon for UI display.
    pub icon: String,
    /// Group volume (0–100), applied via Snapcast.
    pub volume: i32,
    /// Whether the zone is muted.
    pub muted: bool,
    /// Current playback state (stopped, playing, paused).
    pub playback: PlaybackState,
    /// Shuffle mode for playlist playback.
    pub shuffle: bool,
    /// Repeat mode (off, track, playlist).
    pub repeat: snapdog_common::RepeatMode,
    /// Currently playing track metadata, if any.
    pub track: Option<TrackInfo>,
    /// Active unified playlist index (0 = radio, 1+ = Subsonic playlists).
    pub playlist_index: Option<usize>,
    /// Display name of the active playlist.
    pub playlist_name: Option<String>,
    /// Total number of playlists available.
    pub playlist_total: Option<usize>,
    /// Current track position within the active playlist.
    pub playlist_track_index: Option<usize>,
    /// Total number of tracks in the active playlist.
    pub playlist_track_count: Option<usize>,
    /// Active audio source type.
    pub source: SourceType,
    /// URL for the current cover art image.
    pub cover_url: Option<String>,
    /// Buffered position in milliseconds (stream-and-cache progress). Transient, not persisted.
    #[serde(skip)]
    pub buffered_ms: Option<i64>,
    /// Snapcast group ID for this zone (set after Snapcast sync).
    pub snapcast_group_id: Option<String>,
    /// Whether presence is currently detected.
    #[serde(default)]
    pub presence: bool,
    /// Whether presence-triggered playback is enabled.
    #[serde(default = "default_true")]
    pub presence_enabled: bool,
    /// Whether the current playback was started by presence (not manually).
    #[serde(default)]
    pub presence_source: bool,
    /// Auto-off delay in seconds (runtime, may differ from config if changed via KNX/MQTT).
    #[serde(default = "default_auto_off_delay")]
    pub auto_off_delay: u16,
    /// Whether the auto-off timer is currently running.
    #[serde(default)]
    pub auto_off_active: bool,
}

/// Runtime state of a single Snapcast client (speaker/output device).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientState {
    /// Human-readable client name from config.
    pub name: String,
    /// Emoji icon for UI display.
    pub icon: String,
    /// MAC address used to identify the Snapcast client.
    pub mac: String,
    /// 1-based index of the zone this client belongs to.
    pub zone_index: usize,
    /// Client volume (0–100), applied via Snapcast.
    pub volume: i32,
    /// Base volume (0–100) before group scaling. Used by relative/compressed modes.
    /// In absolute mode, this equals `volume`.
    #[serde(default = "default_volume")]
    pub base_volume: i32,
    /// Maximum volume (0–100). Limits how loud this client can go.
    #[serde(default = "default_max_volume")]
    pub max_volume: i32,
    /// Whether the client is muted.
    pub muted: bool,
    /// Audio output latency in milliseconds.
    pub latency_ms: i32,
    /// Whether the Snapcast client is currently connected.
    pub connected: bool,
    /// Snapcast client ID (set after Snapcast sync).
    pub snapcast_id: Option<String>,
    /// Whether this is a `SnapDog` client (supports custom protocol features like EQ).
    #[serde(default)]
    pub is_snapdog: bool,
}

/// Default initial volume for zones and clients.
pub const DEFAULT_VOLUME: i32 = 50;

const fn default_volume() -> i32 {
    DEFAULT_VOLUME
}
use crate::config::{default_auto_off_delay, default_max_volume, default_true};

/// Zone playback state.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PlaybackState {
    /// No audio source active.
    #[default]
    Stopped,
    /// Audio is actively being decoded and streamed.
    Playing,
    /// Playback is paused (source retained, can resume).
    Paused,
}

impl std::fmt::Display for PlaybackState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Stopped => write!(f, "stopped"),
            Self::Playing => write!(f, "playing"),
            Self::Paused => write!(f, "paused"),
        }
    }
}

/// Active audio source type for a zone.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SourceType {
    /// No source active.
    #[default]
    Idle,
    /// Internet radio stream.
    Radio,
    /// Subsonic playlist playback.
    SubsonicPlaylist,
    /// Single Subsonic track playback.
    SubsonicTrack,
    /// Arbitrary URL stream.
    Url,
    /// `AirPlay` receiver (passive, preempts other sources).
    AirPlay,
    /// Spotify Connect receiver (passive, preempts other sources).
    Spotify,
}

impl std::fmt::Display for SourceType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Idle => write!(f, "idle"),
            Self::Radio => write!(f, "radio"),
            Self::SubsonicPlaylist => write!(f, "subsonic_playlist"),
            Self::SubsonicTrack => write!(f, "subsonic_track"),
            Self::Url => write!(f, "url"),
            Self::AirPlay => write!(f, "airplay"),
            Self::Spotify => write!(f, "spotify"),
        }
    }
}

/// Metadata and playback info for the currently playing track.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackInfo {
    /// Track title.
    pub title: String,
    /// Track artist.
    pub artist: String,
    /// Album name.
    pub album: String,
    /// Album artist (may differ from track artist on compilations).
    pub album_artist: Option<String>,
    /// Genre tag.
    pub genre: Option<String>,
    /// Release year.
    pub year: Option<u32>,
    /// Track number within the album.
    pub track_number: Option<u32>,
    /// Disc number for multi-disc albums.
    pub disc_number: Option<u32>,

    /// Total track duration in milliseconds.
    pub duration_ms: i64,
    /// Current playback position in milliseconds.
    pub position_ms: i64,
    /// Whether seeking is supported (false for radio streams).
    pub seekable: bool,
    /// Source type that produced this track info.
    pub source: SourceType,

    /// Audio bitrate in kbps, if known.
    pub bitrate_kbps: Option<u32>,
    /// MIME content type of the audio stream.
    pub content_type: Option<String>,
    /// Decoded sample rate in Hz.
    pub sample_rate: Option<u32>,
}

impl Store {
    /// Initialize state from config with default values.
    fn from_config(config: &AppConfig) -> Self {
        let zones = config
            .zones
            .iter()
            .map(|z| {
                (
                    z.index,
                    ZoneState {
                        name: z.name.clone(),
                        icon: z.icon.clone(),
                        volume: DEFAULT_VOLUME,
                        muted: false,
                        playback: PlaybackState::Stopped,
                        shuffle: false,
                        repeat: snapdog_common::RepeatMode::Off,
                        track: None,
                        playlist_index: None,
                        playlist_name: None,
                        playlist_total: None,
                        playlist_track_index: None,
                        playlist_track_count: None,
                        source: SourceType::Idle,
                        cover_url: None,
                        buffered_ms: None,
                        snapcast_group_id: None,
                        presence: false,
                        presence_enabled: true,
                        presence_source: false,
                        auto_off_delay: z
                            .presence
                            .as_ref()
                            .map_or(crate::config::DEFAULT_AUTO_OFF_DELAY, |p| p.auto_off_delay),
                        auto_off_active: false,
                    },
                )
            })
            .collect();

        let clients = config
            .clients
            .iter()
            .map(|c| {
                (
                    c.index,
                    ClientState {
                        name: c.name.clone(),
                        icon: c.icon.clone(),
                        mac: c.mac.clone(),
                        zone_index: c.zone_index,
                        volume: c.default_volume,
                        base_volume: c.default_volume,
                        max_volume: c.max_volume,
                        muted: false,
                        latency_ms: c.default_latency,
                        connected: false,
                        snapcast_id: None,
                        is_snapdog: false,
                    },
                )
            })
            .collect();

        Self {
            zones,
            clients,
            persist_path: None,
            dirty: false,
        }
    }

    /// Set the persistence path and enable auto-save.
    pub fn set_persist_path(&mut self, path: PathBuf) {
        self.persist_path = Some(path);
    }

    /// Persist current state to JSON file via atomic write (write to `.tmp`, then rename).
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be written or renamed.
    pub fn persist(&mut self) -> Result<()> {
        let Some(path) = &self.persist_path else {
            return Ok(());
        };
        let tmp = path.with_extension("tmp");
        let json = serde_json::to_string_pretty(self).context("Failed to serialize state")?;
        std::fs::write(&tmp, &json)
            .with_context(|| format!("Failed to write {}", tmp.display()))?;
        std::fs::rename(&tmp, path)
            .with_context(|| format!("Failed to rename to {}", path.display()))?;
        self.dirty = false;
        tracing::debug!(path = %path.display(), "State persisted");
        Ok(())
    }

    /// Load state from JSON file, merging with current config-derived state.
    fn load(&mut self, path: &Path) -> Result<()> {
        let json = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read {}", path.display()))?;
        let saved: Self = serde_json::from_str(&json)
            .with_context(|| format!("Failed to parse {}", path.display()))?;

        // Merge: only restore runtime state for zones/clients that still exist in config
        for (idx, saved_zone) in saved.zones {
            if let Some(zone) = self.zones.get_mut(&idx) {
                zone.volume = saved_zone.volume;
                zone.muted = saved_zone.muted;
                zone.shuffle = saved_zone.shuffle;
                zone.repeat = saved_zone.repeat;
                zone.playlist_index = saved_zone.playlist_index;
                zone.playlist_name = saved_zone.playlist_name;
                // Don't restore playback/track — those are transient
            }
        }

        for (idx, saved_client) in saved.clients {
            let valid_zone = self.zones.contains_key(&saved_client.zone_index);
            if let Some(client) = self.clients.get_mut(&idx) {
                client.volume = saved_client.volume;
                client.muted = saved_client.muted;
                client.latency_ms = saved_client.latency_ms;
                if valid_zone {
                    client.zone_index = saved_client.zone_index;
                } else {
                    tracing::warn!(
                        client = idx,
                        zone = saved_client.zone_index,
                        "Ignoring persisted client zone outside current config"
                    );
                }
                // Don't restore connected/snapcast_id — those are transient
            }
        }

        self.persist_path = Some(path.to_path_buf());
        Ok(())
    }
}

/// Periodic auto-save loop for the state store.
/// Persists state every 5 seconds if it has been marked as dirty.
pub fn spawn_auto_save(store: SharedState) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(5));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            let dirty = store.read().await.dirty;
            if dirty {
                let mut s = store.write().await;
                if s.dirty {
                    if let Err(e) = s.persist() {
                        tracing::error!(error = %e, "Auto-save failed");
                    }
                }
            }
        }
    });
}

/// Update a client's state via closure and broadcast a [`Notification::ClientStateChanged`] event.
///
/// No-op if `client_index` does not exist in the store.
pub async fn update_client_and_notify(
    store: &SharedState,
    client_index: usize,
    notify: &crate::api::ws::NotifySender,
    f: impl FnOnce(&mut ClientState),
) {
    let notification = {
        let mut s = store.write().await;
        let Some(client) = s.clients.get_mut(&client_index) else {
            return;
        };
        f(client);
        let volume = client.base_volume;
        let muted = client.muted;
        let connected = client.connected;
        let zone = client.zone_index;
        let is_snapdog = client.is_snapdog;
        s.dirty = true;
        drop(s);
        crate::api::ws::Notification::ClientStateChanged {
            client: client_index,
            volume,
            muted,
            connected,
            zone,
            is_snapdog,
        }
    };
    crate::api::ws::broadcast_notification(notify, &notification);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> AppConfig {
        let raw: crate::config::FileConfig = toml::from_str(
            r#"
            [[zone]]
            name = "Ground Floor"
            [[client]]
            name = "Living Room"
            mac = "00:00:00:00:00:00"
            zone = "Ground Floor"
        "#,
        )
        .unwrap();
        crate::config::load_raw(raw).unwrap()
    }

    #[test]
    fn source_type_wire_format_is_lowercase_without_underscores() {
        // serde `rename_all = "lowercase"` only lowercases — it does NOT insert
        // underscores (the `Display` impl differs: "subsonic_playlist"). Guards the
        // persisted-state / WS / MQTT wire format against a variant-rename regression.
        assert_eq!(serde_json::to_value(SourceType::Idle).unwrap(), "idle");
        assert_eq!(serde_json::to_value(SourceType::Radio).unwrap(), "radio");
        assert_eq!(
            serde_json::to_value(SourceType::SubsonicPlaylist).unwrap(),
            "subsonicplaylist"
        );
        assert_eq!(
            serde_json::to_value(SourceType::SubsonicTrack).unwrap(),
            "subsonictrack"
        );
        assert_eq!(serde_json::to_value(SourceType::Url).unwrap(), "url");
        assert_eq!(
            serde_json::to_value(SourceType::AirPlay).unwrap(),
            "airplay"
        );
        assert_eq!(
            serde_json::to_value(SourceType::Spotify).unwrap(),
            "spotify"
        );
        let back: SourceType =
            serde_json::from_value(serde_json::json!("subsonicplaylist")).unwrap();
        assert!(matches!(back, SourceType::SubsonicPlaylist));
    }

    #[test]
    fn playback_state_wire_format_is_lowercase() {
        assert_eq!(
            serde_json::to_value(PlaybackState::Stopped).unwrap(),
            "stopped"
        );
        assert_eq!(
            serde_json::to_value(PlaybackState::Playing).unwrap(),
            "playing"
        );
        assert_eq!(
            serde_json::to_value(PlaybackState::Paused).unwrap(),
            "paused"
        );
    }

    #[test]
    fn initializes_from_config() {
        let config = test_config();
        let store = Store::from_config(&config);
        assert_eq!(store.zones.len(), 1);
        assert_eq!(store.clients.len(), 1);
        assert_eq!(store.zones[&1].name, "Ground Floor");
        assert_eq!(store.zones[&1].volume, DEFAULT_VOLUME);
        assert_eq!(store.clients[&1].zone_index, 1);
    }

    #[test]
    fn persist_and_load_roundtrip() {
        let config = test_config();
        let mut store = Store::from_config(&config);

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        store.set_persist_path(path.clone());

        // Modify state
        store.zones.get_mut(&1).unwrap().volume = 80;
        store.clients.get_mut(&1).unwrap().muted = true;
        store.persist().unwrap();

        // Load into fresh store
        let mut store2 = Store::from_config(&config);
        store2.load(&path).unwrap();
        assert_eq!(store2.zones[&1].volume, 80);
        assert!(store2.clients[&1].muted);
    }

    #[test]
    fn load_ignores_removed_zones() {
        let config = test_config();
        let mut store = Store::from_config(&config);

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        store.set_persist_path(path.clone());

        // Add a fake zone that doesn't exist in config
        store.zones.insert(
            99,
            ZoneState {
                name: "Ghost".into(),
                icon: "👻".into(),
                volume: 100,
                muted: false,
                playback: PlaybackState::Stopped,
                shuffle: false,
                repeat: snapdog_common::RepeatMode::Off,
                track: None,
                playlist_index: None,
                playlist_name: None,
                playlist_total: None,
                playlist_track_index: None,
                playlist_track_count: None,
                source: SourceType::Idle,
                cover_url: None,
                buffered_ms: None,
                snapcast_group_id: None,
                presence: false,
                presence_enabled: true,
                presence_source: false,
                auto_off_delay: crate::config::DEFAULT_AUTO_OFF_DELAY,
                auto_off_active: false,
            },
        );
        store.persist().unwrap();

        // Load into fresh config-derived store — zone 99 should not appear
        let mut store2 = Store::from_config(&config);
        store2.load(&path).unwrap();
        assert!(!store2.zones.contains_key(&99));
    }

    #[test]
    fn load_ignores_invalid_client_zone() {
        let config = test_config();
        let mut store = Store::from_config(&config);

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        store.set_persist_path(path.clone());
        store.clients.get_mut(&1).unwrap().zone_index = 99;
        store.persist().unwrap();

        let mut store2 = Store::from_config(&config);
        store2.load(&path).unwrap();
        assert_eq!(store2.clients[&1].zone_index, 1);
    }

    #[test]
    fn does_not_restore_transient_state() {
        let config = test_config();
        let mut store = Store::from_config(&config);

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        store.set_persist_path(path.clone());

        store.zones.get_mut(&1).unwrap().playback = PlaybackState::Playing;
        store.clients.get_mut(&1).unwrap().connected = true;
        store.persist().unwrap();

        let mut store2 = Store::from_config(&config);
        store2.load(&path).unwrap();
        // Transient state should NOT be restored
        assert_eq!(store2.zones[&1].playback, PlaybackState::Stopped);
        assert!(!store2.clients[&1].connected);
    }
}
