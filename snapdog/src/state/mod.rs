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

/// The stable identity of this server: a random id that is written to
/// `state_dir/server_id` once and read back at every start.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerIdentity {
    /// The id.
    pub id: String,
    /// Whether the id is stored: `false` when it could not be written, so that the next
    /// start makes up another one. Anything that outlives a start (Home Assistant
    /// entities, whose ids contain it) must not be built on an id that is not stored.
    pub persisted: bool,
}

/// Read `state_dir/server_id`, or make an id up and try to store it.
#[must_use]
pub fn server_identity(state_dir: &Path) -> ServerIdentity {
    let path = state_dir.join("server_id");
    if let Ok(id) = std::fs::read_to_string(&path)
        && !id.trim().is_empty()
    {
        return ServerIdentity {
            id: id.trim().to_string(),
            persisted: true,
        };
    }
    let id = uuid::Uuid::new_v4().to_string();
    let persisted = std::fs::write(&path, &id).is_ok();
    ServerIdentity { id, persisted }
}

/// Thread-safe shared state handle.
pub type SharedState = Arc<RwLock<Store>>;

/// Create a new state store initialized from config, optionally loading persisted state.
///
/// A state file that cannot be read or parsed does not stop the server: it is moved aside as
/// `<name>.unreadable`, logged, and the server starts from the configuration.
///
/// # Errors
///
/// None at present; the `Result` is kept for the callers.
#[allow(clippy::unnecessary_wraps)] // many callers and tests use `?`; a future check may fail again
pub fn init(config: &AppConfig, persist_path: Option<&Path>) -> Result<SharedState> {
    let mut store = Store::from_config(config);

    if let Some(path) = persist_path {
        if path.exists() {
            match store.load(path) {
                Ok(()) => tracing::info!(path = %path.display(), "Restored persisted state"),
                Err(e) => {
                    let aside = path.with_extension("json.unreadable");
                    let moved = std::fs::rename(path, &aside).is_ok();
                    tracing::warn!(
                        error = %format!("{e:#}"),
                        path = %path.display(),
                        kept_as = %aside.display(),
                        moved,
                        "State file unreadable, starting from the configuration"
                    );
                    store = Store::from_config(config);
                }
            }
        }
        // Also on a first start without a file, so the state is saved from now on.
        store.set_persist_path(path.to_path_buf());
    }
    store.apply_zone_volume_limits(config);

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
    /// Stable reference of the active playlist (`snapdog:radio`, `subsonic:<id>`).
    #[serde(default)]
    pub playlist_id: Option<String>,
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
                        playlist_id: None,
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
        if let Some((path, json)) = self.snapshot()? {
            write_state_file(&path, &json)?;
        }
        Ok(())
    }

    /// Serialize the state for saving and clear `dirty`. `None` without a persist path.
    ///
    /// # Errors
    ///
    /// Returns an error if the state cannot be serialized.
    fn snapshot(&mut self) -> Result<Option<(std::path::PathBuf, String)>> {
        let Some(path) = self.persist_path.clone() else {
            return Ok(None);
        };
        let json = serde_json::to_string_pretty(self).context("Failed to serialize state")?;
        self.dirty = false;
        Ok(Some((path, json)))
    }

    /// Apply the configured zone volumes at server start: `default_volume` where it is set,
    /// and `max_volume` as the upper bound of whatever volume was restored.
    fn apply_zone_volume_limits(&mut self, config: &AppConfig) {
        for zone_config in &config.zones {
            if let Some(zone) = self.zones.get_mut(&zone_config.index) {
                if let Some(volume) = zone_config.default_volume {
                    zone.volume = volume;
                }
                zone.volume = zone.volume.clamp(0, zone_config.max_volume);
            }
        }
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
                zone.playlist_id = saved_zone.playlist_id;
                zone.playlist_name = saved_zone.playlist_name;
                // Don't restore playback/track — those are transient
            }
        }

        for (idx, saved_client) in saved.clients {
            let valid_zone = self.zones.contains_key(&saved_client.zone_index);
            if let Some(client) = self.clients.get_mut(&idx) {
                client.volume = saved_client.volume;
                // `base_volume` is what the reconnect path pushes to the client, so
                // leaving it at the configured default would undo the restored volume.
                client.base_volume = saved_client.base_volume.clamp(0, client.max_volume);
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

/// Write the state file: to a temporary file, synced to disk, then renamed over the old one, so a
/// power loss leaves either the old or the new state and never an empty file.
fn write_state_file(path: &Path, json: &str) -> Result<()> {
    use std::io::Write;
    let tmp = path.with_extension("tmp");
    let mut file = std::fs::File::create(&tmp)
        .with_context(|| format!("Failed to create {}", tmp.display()))?;
    file.write_all(json.as_bytes())
        .and_then(|()| file.sync_all())
        .with_context(|| format!("Failed to write {}", tmp.display()))?;
    drop(file);
    std::fs::rename(&tmp, path)
        .with_context(|| format!("Failed to rename to {}", path.display()))?;
    tracing::debug!(path = %path.display(), "State persisted");
    Ok(())
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
                // Serialize under the lock, write without it: readers and writers of the state
                // do not wait for the disk.
                let snapshot = {
                    let mut s = store.write().await;
                    if s.dirty { s.snapshot() } else { Ok(None) }
                };
                let result = match snapshot {
                    Ok(Some((path, json))) => {
                        tokio::task::spawn_blocking(move || write_state_file(&path, &json))
                            .await
                            .unwrap_or_else(|e| Err(anyhow::anyhow!("save task failed: {e}")))
                    }
                    Ok(None) => Ok(()),
                    Err(e) => Err(e),
                };
                if let Err(e) = result {
                    tracing::error!(error = %e, "Auto-save failed");
                    store.write().await.dirty = true; // try again on the next tick
                }
            }
        }
    });
}

/// Assign a client to a zone and announce it with `ClientStateChanged`.
///
/// The one place where REST, KNX and MQTT move a client, so all three accept the same targets
/// and every move reaches the WebSocket, MQTT state and KNX status. The caller queues the
/// Snapcast reconcile. Returns `false`, and changes nothing, for an unknown client or zone.
pub async fn assign_client_zone(
    store: &SharedState,
    notify: &crate::api::ws::NotifySender,
    client_index: usize,
    zone_index: usize,
) -> bool {
    {
        let s = store.read().await;
        if zone_index == 0
            || !s.zones.contains_key(&zone_index)
            || !s.clients.contains_key(&client_index)
        {
            return false;
        }
    }
    update_client_and_notify(store, client_index, notify, |c| c.zone_index = zone_index).await;
    true
}

/// Update a client's state via closure and broadcast a [`Notification::ClientStateChanged`] event.
///
/// No-op if `client_index` does not exist in the store. Returns whether the client exists.
pub async fn update_client_and_notify(
    store: &SharedState,
    client_index: usize,
    notify: &crate::api::ws::NotifySender,
    f: impl FnOnce(&mut ClientState),
) -> bool {
    let notification = {
        let mut s = store.write().await;
        let Some(client) = s.clients.get_mut(&client_index) else {
            return false;
        };
        f(client);
        let volume = client.base_volume;
        let muted = client.muted;
        let connected = client.connected;
        let zone = client.zone_index;
        let is_snapdog = client.is_snapdog;
        let latency_ms = client.latency_ms;
        let name = client.name.clone();
        s.dirty = true;
        drop(s);
        crate::api::ws::Notification::ClientStateChanged {
            client: client_index,
            volume,
            muted,
            connected,
            zone,
            is_snapdog,
            latency_ms,
            name,
        }
    };
    crate::api::ws::broadcast_notification(notify, &notification);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_server_id_is_stored_once_and_read_back() {
        let dir = tempfile::tempdir().unwrap();
        let first = server_identity(dir.path());
        assert!(first.persisted);
        assert_eq!(
            server_identity(dir.path()),
            first,
            "the same id at the next start"
        );
        // A blank file is not an id.
        std::fs::write(dir.path().join("server_id"), " \n").unwrap();
        let again = server_identity(dir.path());
        assert!(again.persisted);
        assert_ne!(again.id, first.id);
    }

    /// Review finding m6: when the id cannot be stored (a read-only or missing state
    /// directory) every start makes up a new one; that must be known.
    #[test]
    fn an_id_that_cannot_be_stored_is_marked_as_not_persisted() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("does-not-exist");
        let identity = server_identity(&missing);
        assert!(!identity.persisted);
        assert_ne!(
            identity.id,
            server_identity(&missing).id,
            "a new id at every start"
        );
    }

    fn config_with_zone(extra: &str) -> AppConfig {
        let raw: crate::config::FileConfig = toml::from_str(&format!(
            r#"
            [[zone]]
            name = "Ground Floor"
            {extra}
            [[client]]
            name = "Living Room"
            mac = "00:00:00:00:00:00"
            zone = "Ground Floor"
        "#
        ))
        .unwrap();
        crate::config::load_raw(raw).unwrap()
    }

    /// #509: `default_volume` is applied at every start, over a restored volume.
    #[test]
    fn default_volume_is_set_at_start_over_the_restored_volume() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        let plain = config_with_zone("");
        let mut store = Store::from_config(&plain);
        store.set_persist_path(path.clone());
        store.zones.get_mut(&1).unwrap().volume = 80;
        store.persist().unwrap();

        let restored = init(&plain, Some(&path)).unwrap();
        assert_eq!(
            restored.try_read().unwrap().zones[&1].volume,
            80,
            "no default_volume: kept"
        );

        let with_default = init(&config_with_zone("default_volume = 30"), Some(&path)).unwrap();
        assert_eq!(with_default.try_read().unwrap().zones[&1].volume, 30);
    }

    /// #509: `max_volume` bounds a restored volume and `default_volume`.
    #[test]
    fn max_volume_bounds_the_start_volume() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        let plain = config_with_zone("");
        let mut store = Store::from_config(&plain);
        store.set_persist_path(path.clone());
        store.zones.get_mut(&1).unwrap().volume = 90;
        store.persist().unwrap();

        let capped = init(&config_with_zone("max_volume = 60"), Some(&path)).unwrap();
        assert_eq!(capped.try_read().unwrap().zones[&1].volume, 60);
        let both = config_with_zone("max_volume = 60\n            default_volume = 75");
        assert_eq!(
            both.zones[0].default_volume,
            Some(60),
            "default_volume is clamped to max"
        );
    }

    #[test]
    fn zone_receivers_default_to_on_and_can_be_switched_off() {
        let on = config_with_zone("");
        assert!(on.zones[0].airplay && on.zones[0].spotify);
        assert_eq!(
            (on.zones[0].max_volume, on.zones[0].default_volume),
            (100, None)
        );
        let off = config_with_zone("airplay = false\n            spotify = false");
        assert!(!off.zones[0].airplay && !off.zones[0].spotify);
    }

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

    /// #502: an unreadable state file does not stop the start; it is kept aside.
    #[test]
    fn an_unreadable_state_file_is_moved_aside_and_the_server_starts() {
        let config = test_config();
        for content in ["", "{ not json", r#"{"zones": 5}"#] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("zones.json");
            std::fs::write(&path, content).unwrap();
            let store = init(&config, Some(&path)).expect("start does not fail");
            assert_eq!(store.try_read().unwrap().zones[&1].volume, DEFAULT_VOLUME);
            assert!(!path.exists(), "the bad file is moved away: {content:?}");
            assert_eq!(
                std::fs::read_to_string(path.with_extension("json.unreadable")).unwrap(),
                content
            );
        }
    }

    /// #502: a first start without a state file saves the state from then on.
    #[test]
    fn a_first_start_without_a_file_persists_afterwards() {
        let config = test_config();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("zones.json");
        let store = init(&config, Some(&path)).unwrap();
        store.try_write().unwrap().zones.get_mut(&1).unwrap().volume = 33;
        store.try_write().unwrap().persist().unwrap();
        let again = init(&config, Some(&path)).unwrap();
        assert_eq!(again.try_read().unwrap().zones[&1].volume, 33);
        assert!(
            !path.with_extension("tmp").exists(),
            "no temporary file is left"
        );
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
        store.clients.get_mut(&1).unwrap().base_volume = 30;
        store.persist().unwrap();

        // Load into fresh store
        let mut store2 = Store::from_config(&config);
        store2.load(&path).unwrap();
        assert_eq!(store2.zones[&1].volume, 80);
        assert!(store2.clients[&1].muted);
        assert_eq!(store2.clients[&1].base_volume, 30);
    }

    #[test]
    fn load_clamps_restored_base_volume_to_max_volume() {
        let config = test_config();
        let mut store = Store::from_config(&config);

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        store.set_persist_path(path.clone());
        store.clients.get_mut(&1).unwrap().base_volume = 90;
        store.persist().unwrap();

        let mut store2 = Store::from_config(&config);
        store2.clients.get_mut(&1).unwrap().max_volume = 60;
        store2.load(&path).unwrap();
        assert_eq!(store2.clients[&1].base_volume, 60);
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
                playlist_id: None,
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
