// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! Configuration types: raw TOML structs and resolved application config.

use serde::{Deserialize, Serialize};

// ── Typed enums for config fields ─────────────────────────────

/// A string wrapper that masks its content in `Debug` output to prevent secret leakage.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SecretString(String);

impl SecretString {
    /// Create a new secret string.
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    /// Access the underlying string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Convert into the underlying string.
    #[must_use]
    pub fn into_inner(self) -> String {
        self.0
    }
}

impl std::fmt::Debug for SecretString {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("********")
    }
}

impl std::ops::Deref for SecretString {
    type Target = str;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl From<String> for SecretString {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl From<&str> for SecretString {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

/// KNX operating mode.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum KnxRole {
    /// Connect to a KNX/IP gateway.
    #[default]
    Client,
    /// Run as ETS-programmable KNX/IP device.
    Device,
}

impl KnxRole {
    /// String representation for matching and display.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Client => "client",
            Self::Device => "device",
        }
    }
}

/// Audio codec for Snapcast streaming.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioCodec {
    /// Uncompressed PCM (WAV header passthrough).
    Pcm,
    /// FLAC lossless compression.
    #[default]
    Flac,
    /// Raw f32 with LZ4 compression.
    F32lz4,
    /// Raw f32 with LZ4 compression + encryption.
    F32lz4e,
}

impl AudioCodec {
    /// String representation for Snapcast protocol.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pcm => "pcm",
            Self::Flac => "flac",
            Self::F32lz4 => "f32lz4",
            Self::F32lz4e => "f32lz4e",
        }
    }
}

impl std::fmt::Display for AudioCodec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Tracing log level.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LogLevel {
    /// Most verbose.
    Trace,
    /// Debug messages.
    Debug,
    /// Normal operation.
    #[default]
    Info,
    /// Warnings only.
    Warn,
    /// Errors only.
    Error,
}

impl std::fmt::Display for LogLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Trace => "trace",
            Self::Debug => "debug",
            Self::Info => "info",
            Self::Warn => "warn",
            Self::Error => "error",
        })
    }
}

/// Subsonic stream format.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SubsonicFormat {
    /// Original file (no transcoding).
    #[default]
    Raw,
    /// FLAC lossless.
    Flac,
    /// MP3 lossy.
    Mp3,
    /// Opus lossy.
    Opus,
}

impl SubsonicFormat {
    /// String representation for Subsonic API URL parameter.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Raw => "raw",
            Self::Flac => "flac",
            Self::Mp3 => "mp3",
            Self::Opus => "opus",
        }
    }
}

// ── Raw TOML structs (what the user writes) ───────────────────

/// Root of the TOML config file. Optional fields use defaults.
#[derive(Debug, Default, Deserialize)]
pub struct FileConfig {
    /// Server display name. Used for `mDNS`, MQTT discovery, KNX device name.
    /// Default: "`SnapDog`".
    #[serde(default = "default_server_name")]
    pub name: String,
    /// System settings (log level, log file).
    #[serde(default)]
    pub system: SystemConfig,
    /// Audio output format (sample rate, bit depth, channels, codec).
    #[serde(default)]
    pub audio: AudioConfig,
    /// HTTP server settings (port, API keys).
    #[serde(default)]
    pub http: HttpConfig,
    /// Snapcast connection and management settings.
    #[serde(default)]
    pub snapcast: SnapcastConfig,
    /// mDNS/Bonjour service advertisement.
    #[serde(default)]
    pub mdns: MdnsConfig,
    /// `AirPlay` receiver settings.
    #[serde(default)]
    pub airplay: AirplayConfig,
    /// Subsonic/Navidrome server connection.
    #[serde(default)]
    pub subsonic: Option<SubsonicConfig>,
    /// Spotify Connect receiver settings.
    #[serde(default)]
    pub spotify: Option<SpotifyConfig>,
    /// MQTT bridge settings.
    #[serde(default)]
    pub mqtt: Option<MqttConfig>,
    /// KNX/IP integration settings.
    pub knx: Option<KnxConfig>,
    /// Zone definitions.
    #[serde(default)]
    pub zone: Vec<RawZoneConfig>,
    /// Client (speaker) definitions.
    #[serde(default)]
    pub client: Vec<RawClientConfig>,
    /// Radio station definitions.
    #[serde(default)]
    pub radio: Vec<RawRadioConfig>,
}

/// System-level settings.
#[derive(Debug, Deserialize, Clone)]
pub struct SystemConfig {
    /// Tracing log level: trace, debug, info, warn, error.
    #[serde(default)]
    pub log_level: LogLevel,
    /// Optional log file path (daily rotation).
    pub log_file: Option<String>,
    /// Directory for persistent state files (zones.json, eq.json, snapcast.json).
    /// Defaults to platform-appropriate path.
    #[serde(default = "default_state_dir")]
    pub state_dir: String,
}

impl Default for SystemConfig {
    fn default() -> Self {
        Self {
            log_level: LogLevel::default(),
            log_file: None,
            state_dir: default_state_dir(),
        }
    }
}

fn default_base_url() -> String {
    "http://localhost:5555".into()
}

fn default_state_dir() -> String {
    // Running as systemd service → /var/lib/snapdog
    if std::env::var("INVOCATION_ID").is_ok() {
        return "/var/lib/snapdog".into();
    }

    #[cfg(target_os = "macos")]
    {
        std::env::var("HOME").map_or_else(
            |_| "/tmp/snapdog".into(),
            |h| format!("{h}/Library/Application Support/SnapDog"),
        )
    }
    #[cfg(target_os = "linux")]
    {
        std::env::var("XDG_STATE_HOME").map_or_else(
            |_| {
                std::env::var("HOME").map_or_else(
                    |_| "/tmp/snapdog".into(),
                    |h| format!("{h}/.local/state/snapdog"),
                )
            },
            |p| format!("{p}/snapdog"),
        )
    }
    #[cfg(target_os = "windows")]
    {
        std::env::var("LOCALAPPDATA").map_or_else(
            |_| "C:\\ProgramData\\SnapDog".into(),
            |p| format!("{p}\\SnapDog"),
        )
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        "/tmp/snapdog".into()
    }
}

/// How zone (group) volume changes affect individual client volumes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GroupVolumeMode {
    /// Set all clients to the zone volume, ignoring individual levels.
    Absolute,
    /// Scale each client proportionally: `base_volume * zone_volume / 100`.
    Relative,
    /// Soft scaling with square-root curve — quiet clients don't drop as fast.
    /// `base_volume * sqrt(zone_volume / 100)`.
    #[default]
    Compressed,
}

impl GroupVolumeMode {
    /// Compute effective client volume given the client's base volume, zone volume,
    /// and maximum volume limit.
    #[must_use]
    pub fn effective(self, base_volume: i32, zone_volume: i32, max_volume: i32) -> i32 {
        let max = max_volume.clamp(0, 100);
        let base = base_volume.clamp(0, max);
        let z = zone_volume.clamp(0, 100);
        match self {
            Self::Absolute => z.min(max),
            Self::Relative => (base * z / 100).clamp(0, max),
            Self::Compressed => {
                let factor = (f64::from(z) / 100.0).sqrt();
                (f64::from(base) * factor).round().min(f64::from(max)) as i32
            }
        }
    }
}

// ── Presence ───────────────────────────────────────────────────

/// Presence-triggered playback source.
#[derive(Debug, Clone)]
pub enum PresenceSource {
    /// Play a radio station by index (0-based).
    Radio(usize),
    /// Play a Subsonic playlist by ID.
    Playlist(String),
    /// Do nothing (presence ignored in this time slot).
    None,
}

impl<'de> serde::Deserialize<'de> for PresenceSource {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        match s.as_str() {
            "none" => Ok(Self::None),
            _ if s.starts_with("radio:") => {
                let idx = s[6..].parse::<usize>().map_err(|_| {
                    serde::de::Error::custom(format!(
                        "invalid radio index in '{s}', expected radio:<number>"
                    ))
                })?;
                Ok(Self::Radio(idx))
            }
            _ if s.starts_with("playlist:") => {
                let id = &s[9..];
                if id.is_empty() {
                    return Err(serde::de::Error::custom(
                        "empty playlist ID in presence source",
                    ));
                }
                Ok(Self::Playlist(id.to_string()))
            }
            _ => Err(serde::de::Error::custom(format!(
                "invalid presence source '{s}', expected 'none', 'radio:<index>', or 'playlist:<id>'"
            ))),
        }
    }
}

/// A time-based presence schedule entry.
#[derive(Debug, Clone, Deserialize)]
pub struct PresenceScheduleEntry {
    /// Start time in HH:MM format (24h).
    pub from: String,
    /// End time in HH:MM format (24h). Must be after `from`.
    pub to: String,
    /// Source to play during this time window.
    pub source: PresenceSource,
}

/// Presence-triggered playback configuration for a zone.
#[derive(Debug, Clone, Deserialize)]
pub struct PresenceConfig {
    /// Auto-off delay in seconds after presence lost. 0 = immediate stop.
    #[serde(default = "default_auto_off_delay")]
    pub auto_off_delay: u16,
    /// Default source when no schedule matches. If unset, resumes last source.
    pub default_source: Option<PresenceSource>,
    /// Time-based source schedule. First match wins.
    #[serde(default)]
    pub schedule: Vec<PresenceScheduleEntry>,
}

/// Default auto-off delay in seconds (15 minutes).
pub const DEFAULT_AUTO_OFF_DELAY: u16 = 900;

/// Default auto-off delay in seconds (15 minutes).
#[must_use]
pub const fn default_auto_off_delay() -> u16 {
    DEFAULT_AUTO_OFF_DELAY
}

/// Parse a HH:MM time string into minutes since midnight.
///
/// # Errors
///
/// Returns an error if the string is not in valid HH:MM format.
pub fn parse_time(s: &str) -> anyhow::Result<u16> {
    let parts: Vec<&str> = s.split(':').collect();
    anyhow::ensure!(parts.len() == 2, "expected HH:MM format, got '{s}'");
    let h: u16 = parts[0]
        .parse()
        .map_err(|_| anyhow::anyhow!("invalid hour in '{s}'"))?;
    let m: u16 = parts[1]
        .parse()
        .map_err(|_| anyhow::anyhow!("invalid minute in '{s}'"))?;
    anyhow::ensure!(h <= 23, "hour must be 0-23, got {h} in '{s}'");
    anyhow::ensure!(m <= 59, "minute must be 0-59, got {m} in '{s}'");
    Ok(h * 60 + m)
}

/// Audio output configuration (sample rate, bit depth, codec, encryption).
#[derive(Debug, Clone, Deserialize)]
pub struct AudioConfig {
    /// Output sample rate in Hz (e.g., 44100, 48000, 96000).
    #[serde(default = "default_sample_rate")]
    pub sample_rate: u32,
    /// Output bit depth: 16, 24, or 32.
    #[serde(default = "default_bit_depth")]
    pub bit_depth: u16,
    /// Number of audio channels (typically 2 for stereo).
    #[serde(default = "default_channels")]
    pub channels: u16,
    /// How to resolve conflicts when AirPlay/Spotify is active and local
    /// playback (radio/subsonic) is requested.
    #[serde(default)]
    pub source_conflict: SourceConflict,
    /// Fade duration in milliseconds when a client switches zones.
    /// Set to 0 to disable. Only applies to `SnapDog` clients.
    #[serde(default = "default_zone_switch_fade_ms")]
    pub zone_switch_fade_ms: u16,
    /// Fade duration in milliseconds when switching audio sources within a zone
    /// (e.g., radio → subsonic). Set to 0 to disable.
    #[serde(default = "default_zone_switch_fade_ms")]
    pub source_switch_fade_ms: u16,
}

const fn default_zone_switch_fade_ms() -> u16 {
    snapdog_common::DEFAULT_FADE_MS
}

/// Source conflict resolution policy.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceConflict {
    /// Most recent source always takes over (stops the other).
    #[default]
    LastWins,
    /// AirPlay/Spotify has priority; local playback is rejected until it stops.
    ReceiverWins,
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            sample_rate: default_sample_rate(),
            bit_depth: default_bit_depth(),
            channels: default_channels(),
            source_conflict: SourceConflict::default(),
            zone_switch_fade_ms: default_zone_switch_fade_ms(),
            source_switch_fade_ms: default_zone_switch_fade_ms(),
        }
    }
}

impl AudioConfig {
    /// Snapcast sample format string (e.g., "48000:16:2").
    #[must_use]
    pub fn sample_format(&self) -> String {
        format!("{}:{}:{}", self.sample_rate, self.bit_depth, self.channels)
    }
}

/// HTTP server configuration.
#[derive(Debug, Deserialize, Clone)]
pub struct HttpConfig {
    /// Port for the REST API, WebSocket, and embedded `WebUI`.
    #[serde(default = "default_http_port")]
    pub port: u16,
    /// Bind address. Default: `0.0.0.0`. Use `::` for dual-stack IPv4+IPv6.
    #[serde(default = "default_bind_address")]
    pub bind: String,
    /// External base URL (e.g., `http://192.168.2.20:5555` or `https://music.example.com`).
    /// Used for absolute URLs in API responses. Defaults to `http://localhost:<port>`.
    #[serde(default = "default_base_url")]
    pub base_url: String,
    /// Optional API keys. If set, all API endpoints require `Authorization: Bearer <key>`.
    #[serde(default)]
    pub api_keys: Vec<SecretString>,
}

impl Default for HttpConfig {
    fn default() -> Self {
        Self {
            port: default_http_port(),
            bind: default_bind_address(),
            base_url: default_base_url(),
            api_keys: vec![],
        }
    }
}

/// Policy for handling clients not defined in the configuration.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UnknownClientPolicy {
    /// Accept unknown clients, show in `WebUI`, assign to default zone.
    #[default]
    Accept,
    /// Accept connection but don't show in `WebUI` or assign a zone.
    Ignore,
    /// Reject connection immediately after Hello.
    Reject,
}

/// mDNS/Bonjour service advertisement configuration.
#[derive(Debug, Deserialize, Clone)]
pub struct MdnsConfig {
    /// Enable mDNS advertisement. Default: true.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Additionally advertise `_snapcast._tcp` on the streaming port for
    /// compatibility with standard Snapcast clients. Default: false.
    #[serde(default)]
    pub advertise_snapcast: bool,
}

impl Default for MdnsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            advertise_snapcast: false,
        }
    }
}

/// Snapcast server connection and management.
#[derive(Debug, Deserialize, Clone)]
pub struct SnapcastConfig {
    /// Snapcast server address (hostname or IP).
    #[serde(default = "default_snapcast_address")]
    pub address: String,
    /// TCP JSON-RPC control port (default: 1705).
    #[serde(default = "default_jsonrpc_port")]
    pub jsonrpc_port: u16,
    /// Audio streaming port (default: 1704).
    #[serde(default = "default_streaming_port")]
    pub streaming_port: u16,
    /// Start snapserver as a managed child process.
    #[serde(default = "default_true")]
    pub managed: bool,
    /// Show snapserver console output.
    #[serde(default)]
    pub verbose: bool,
    /// Policy for clients not in the config. Default: `accept`.
    #[serde(default)]
    pub unknown_clients: UnknownClientPolicy,
    /// Default zone for unknown clients (when policy is `accept`).
    /// If not set, uses the first configured zone.
    pub default_zone: Option<String>,
    /// Streaming codec: pcm, flac, f32lz4, f32lz4e.
    #[serde(default)]
    pub codec: AudioCodec,
    /// Pre-shared key for f32lz4e encryption (default: built-in key).
    #[serde(default)]
    pub encryption_psk: Option<SecretString>,
    /// Default group volume mode for all zones.
    #[serde(default)]
    pub group_volume_mode: GroupVolumeMode,
}

impl Default for SnapcastConfig {
    fn default() -> Self {
        Self {
            address: default_snapcast_address(),
            jsonrpc_port: default_jsonrpc_port(),
            streaming_port: default_streaming_port(),
            managed: true,
            verbose: false,
            unknown_clients: UnknownClientPolicy::default(),
            default_zone: None,
            codec: AudioCodec::default(),
            encryption_psk: None,
            group_volume_mode: GroupVolumeMode::default(),
        }
    }
}

/// `AirPlay` protocol mode selection.
#[derive(Debug, Deserialize, Clone, Copy, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AirplayMode {
    /// Classic `AirPlay` 1: ALAC/AAC over RTP.
    Airplay1,
    /// `AirPlay` 2: buffered audio, `ChaCha20`, SRP pairing.
    #[default]
    Airplay2,
}

/// `AirPlay` receiver settings (shared across all zones).
#[derive(Debug, Deserialize, Clone, Default)]
pub struct AirplayConfig {
    /// Optional password for `AirPlay` connections.
    pub password: Option<String>,
    /// `AirPlay` protocol mode: `"airplay1"` or `"airplay2"` (default).
    #[serde(default)]
    pub mode: AirplayMode,
    /// Bind to specific addresses (default: all interfaces).
    pub bind: Option<Vec<std::net::IpAddr>>,
}

/// Spotify Connect receiver settings.
#[derive(Debug, Deserialize, Clone)]
pub struct SpotifyConfig {
    /// Device name shown in Spotify app (e.g., "`SnapDog` Ground Floor").
    pub name: String,
    /// Audio bitrate: 96, 160, or 320 kbps. Default: 320.
    #[serde(default = "default_spotify_bitrate")]
    pub bitrate: u32,
}

impl SpotifyConfig {
    /// Stable device ID derived from the name (for Zeroconf).
    #[must_use]
    pub fn device_id(&self) -> String {
        format!("{:x}", md5::compute(self.name.as_bytes()))
    }

    /// Convert the bitrate u32 to librespot's Bitrate enum.
    #[cfg(feature = "spotify")]
    #[must_use]
    pub const fn bitrate_enum(&self) -> librespot_playback::config::Bitrate {
        match self.bitrate {
            96 => librespot_playback::config::Bitrate::Bitrate96,
            160 => librespot_playback::config::Bitrate::Bitrate160,
            _ => librespot_playback::config::Bitrate::Bitrate320,
        }
    }
}

const fn default_spotify_bitrate() -> u32 {
    320
}

/// Subsonic/Navidrome server connection.
#[derive(Debug, Deserialize, Clone)]
pub struct SubsonicConfig {
    /// Server base URL (e.g., `https://music.example.com`).
    pub url: String,
    /// Authentication username.
    pub username: String,
    /// Authentication password.
    pub password: SecretString,
    /// Audio output format — SSOT with Snapcast stream configuration.
    /// Stream format: raw (original file), flac, mp3, opus.
    #[serde(default)]
    pub format: SubsonicFormat,
    /// Skip TLS certificate verification (for self-signed certs).
    #[serde(default)]
    pub tls_skip_verify: bool,
    /// Track cache configuration.
    #[serde(default)]
    pub cache: SubsonicCacheConfig,
}

/// Subsonic track cache configuration.
#[derive(Debug, Deserialize, Clone)]
pub struct SubsonicCacheConfig {
    /// Cache directory path. Defaults to `$XDG_CACHE_HOME/snapdog/tracks` or `/tmp/snapdog/tracks`.
    #[serde(default = "default_cache_path")]
    pub path: String,
    /// Maximum cache size in megabytes. Oldest entries evicted when exceeded.
    #[serde(default = "default_cache_max_size_mb")]
    pub max_size_mb: u64,
}

/// Number of upcoming playlist tracks to pre-fetch (not configurable).
pub const CACHE_LOOKAHEAD: usize = 2;

fn default_cache_path() -> String {
    #[cfg(target_os = "macos")]
    {
        std::env::var("HOME").map_or_else(
            |_| "/tmp/snapdog/tracks".into(),
            |h| format!("{h}/Library/Caches/SnapDog/tracks"),
        )
    }
    #[cfg(target_os = "linux")]
    {
        std::env::var("XDG_CACHE_HOME").map_or_else(
            |_| {
                std::env::var("HOME").map_or_else(
                    |_| "/tmp/snapdog/tracks".into(),
                    |h| format!("{h}/.cache/snapdog/tracks"),
                )
            },
            |p| format!("{p}/snapdog/tracks"),
        )
    }
    #[cfg(target_os = "windows")]
    {
        std::env::var("LOCALAPPDATA").map_or_else(
            |_| "C:\\ProgramData\\snapdog\\cache\\tracks".into(),
            |p| format!("{p}\\SnapDog\\cache\\tracks"),
        )
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        "/tmp/snapdog/tracks".into()
    }
}

const fn default_cache_max_size_mb() -> u64 {
    2048
}

impl Default for SubsonicCacheConfig {
    fn default() -> Self {
        Self {
            path: default_cache_path(),
            max_size_mb: default_cache_max_size_mb(),
        }
    }
}

/// MQTT bridge configuration.
#[derive(Debug, Deserialize, Clone)]
pub struct MqttConfig {
    /// Broker address (host:port).
    pub broker: String,
    /// MQTT client ID. Defaults to "snapdog".
    #[serde(default = "default_mqtt_client_id")]
    pub client_id: String,
    /// MQTT username (empty for anonymous).
    #[serde(default)]
    pub username: String,
    /// MQTT password.
    #[serde(default)]
    pub password: SecretString,
    /// Base topic prefix (e.g., "snapdog/").
    #[serde(default = "default_mqtt_base_topic")]
    pub base_topic: String,
}

fn default_mqtt_client_id() -> String {
    "snapdog".into()
}

/// KNX/IP integration settings.
#[derive(Debug, Deserialize, Clone, Default)]
pub struct KnxConfig {
    /// Device role: `client` (default) or `device` (ETS-programmable).
    #[serde(default, alias = "mode")]
    pub role: KnxRole,
    /// KNX/IP gateway URL for tunnel/routing.
    /// Required for client role. Optional for device role (enables GA reception via tunnel).
    /// Examples: `udp://192.168.1.50:3671` (tunnel), `udp://224.0.23.12:3671` (routing)
    pub url: Option<String>,
    /// KNX individual address (device role). Example: `"1.1.100"`
    pub individual_address: Option<String>,
    /// Persist ETS-programmed configuration across restarts (device role).
    pub persist_ets_config: Option<bool>,
    /// Start with programming mode active (set via --knx-prog-mode CLI flag).
    #[serde(default)]
    pub start_prog_mode: bool,
}

// ── Raw zone/client/radio (user-facing, optional fields) ──────

/// Zone definition as written in TOML.
#[derive(Debug, Deserialize)]
pub struct RawZoneConfig {
    /// Human-readable zone name (also used as `AirPlay` name).
    pub name: String,
    /// Emoji icon for the zone.
    #[serde(default = "default_zone_icon")]
    pub icon: String,
    /// Override Snapcast sink path (default: auto-generated).
    pub sink: Option<String>,
    /// Override `AirPlay` receiver name (default: zone name).
    pub airplay_name: Option<String>,
    /// KNX group addresses for this zone.
    #[serde(default)]
    pub knx: RawZoneKnxConfig,
    /// Override group volume mode for this zone.
    pub group_volume_mode: Option<GroupVolumeMode>,
    /// Presence-triggered playback configuration.
    pub presence: Option<PresenceConfig>,
}

/// KNX group addresses for zone control (all optional, explicit config only).
///
/// Each field is a KNX group address string (e.g. "1/2/3") mapped to a zone function.
#[derive(Debug, Default, Deserialize)]
pub struct RawZoneKnxConfig {
    /// Play command.
    pub play: Option<String>,
    /// Pause command.
    pub pause: Option<String>,
    /// Stop command.
    pub stop: Option<String>,
    /// Next track command.
    pub track_next: Option<String>,
    /// Previous track command.
    pub track_previous: Option<String>,
    /// Playback status feedback.
    pub control_status: Option<String>,
    /// Volume set (DPT 5.001 scaling).
    pub volume: Option<String>,
    /// Volume status feedback.
    pub volume_status: Option<String>,
    /// Relative volume dimming (DPT 3.007).
    pub volume_dim: Option<String>,
    /// Mute command.
    pub mute: Option<String>,
    /// Mute status feedback.
    pub mute_status: Option<String>,
    /// Mute toggle command.
    pub mute_toggle: Option<String>,
    /// Track title status feedback (DPT 16.001).
    pub track_title_status: Option<String>,
    /// Track artist status feedback (DPT 16.001).
    pub track_artist_status: Option<String>,
    /// Track album status feedback (DPT 16.001).
    pub track_album_status: Option<String>,
    /// Track progress status feedback (percentage).
    pub track_progress_status: Option<String>,
    /// Track playing status feedback (boolean).
    pub track_playing_status: Option<String>,
    /// Single-track repeat command.
    pub track_repeat: Option<String>,
    /// Single-track repeat status feedback.
    pub track_repeat_status: Option<String>,
    /// Single-track repeat toggle command.
    pub track_repeat_toggle: Option<String>,
    /// Playlist selection command (index).
    pub playlist: Option<String>,
    /// Playlist selection status feedback.
    pub playlist_status: Option<String>,
    /// Next playlist command.
    pub playlist_next: Option<String>,
    /// Previous playlist command.
    pub playlist_previous: Option<String>,
    /// Shuffle command.
    pub shuffle: Option<String>,
    /// Shuffle status feedback.
    pub shuffle_status: Option<String>,
    /// Shuffle toggle command.
    pub shuffle_toggle: Option<String>,
    /// Playlist repeat command.
    pub repeat: Option<String>,
    /// Playlist repeat status feedback.
    pub repeat_status: Option<String>,
    /// Playlist repeat toggle command.
    pub repeat_toggle: Option<String>,
    /// Presence command (DPT 1.018 Occupancy).
    pub presence: Option<String>,
    /// Presence enable/disable (DPT 1.001).
    pub presence_enable: Option<String>,
    /// Presence enable status feedback.
    pub presence_enable_status: Option<String>,
    /// Presence auto-off timeout (DPT 7.005, seconds).
    pub presence_timeout: Option<String>,
    /// Presence auto-off timeout status feedback.
    pub presence_timeout_status: Option<String>,
    /// Presence auto-off timer active status (DPT 1.001).
    pub presence_timer_status: Option<String>,
    /// Presence source override (DPT 5.010, 0=schedule).
    pub presence_source_override: Option<String>,
}

/// Client (speaker) definition as written in TOML.
#[derive(Debug, Deserialize)]
pub struct RawClientConfig {
    /// Human-readable client name.
    pub name: String,
    /// Snapcast client MAC address (used to match `host.mac`).
    pub mac: String,
    /// Zone name this client belongs to.
    pub zone: String,
    /// Emoji icon for the client.
    #[serde(default = "default_client_icon")]
    pub icon: String,
    /// Maximum volume (0–100). Limits how loud this client can go.
    #[serde(default = "default_max_volume")]
    pub max_volume: i32,
    /// KNX group addresses for this client.
    #[serde(default)]
    pub knx: RawClientKnxConfig,
}

/// KNX group addresses for client control (all optional, explicit config only).
///
/// Each field is a KNX group address string (e.g. "1/2/3") mapped to a client function.
#[derive(Debug, Default, Deserialize)]
pub struct RawClientKnxConfig {
    /// Volume set (DPT 5.001 scaling).
    pub volume: Option<String>,
    /// Volume status feedback.
    pub volume_status: Option<String>,
    /// Relative volume dimming (DPT 3.007).
    pub volume_dim: Option<String>,
    /// Mute command.
    pub mute: Option<String>,
    /// Mute status feedback.
    pub mute_status: Option<String>,
    /// Mute toggle command.
    pub mute_toggle: Option<String>,
    /// Latency set (milliseconds).
    pub latency: Option<String>,
    /// Latency status feedback.
    pub latency_status: Option<String>,
    /// Zone assignment command.
    pub zone: Option<String>,
    /// Zone assignment status feedback.
    pub zone_status: Option<String>,
    /// Connection status feedback.
    pub connected_status: Option<String>,
}

/// Radio station definition.
#[derive(Debug, Deserialize)]
pub struct RawRadioConfig {
    /// Station name.
    pub name: String,
    /// Stream URL (direct, M3U, PLS, or HLS).
    pub url: String,
    /// Cover art URL.
    pub cover: Option<String>,
}

// ── Resolved config (fully populated, no Option) ──────────────

/// Fully resolved application configuration. All conventions applied.
#[derive(Debug, Clone)]
pub struct AppConfig {
    /// Server display name.
    pub name: String,
    /// System settings.
    pub system: SystemConfig,
    /// Audio output format.
    pub audio: AudioConfig,
    /// HTTP server settings.
    pub http: HttpConfig,
    /// Snapcast connection settings.
    pub snapcast: SnapcastConfig,
    /// mDNS/Bonjour service advertisement.
    pub mdns: MdnsConfig,
    /// `AirPlay` receiver settings.
    pub airplay: AirplayConfig,
    /// Subsonic connection (if configured).
    pub subsonic: Option<SubsonicConfig>,
    /// Spotify Connect receiver settings (if configured).
    pub spotify: Option<SpotifyConfig>,
    /// MQTT bridge (if configured).
    pub mqtt: Option<MqttConfig>,
    /// KNX settings.
    pub knx: Option<KnxConfig>,
    /// Resolved zone configurations (1-indexed).
    pub zones: Vec<ZoneConfig>,
    /// Resolved client configurations (1-indexed).
    pub clients: Vec<ClientConfig>,
    /// Radio station list.
    pub radios: Vec<RadioConfig>,
}

/// Result of resolving a unified playlist index.
#[derive(Debug, Clone)]
pub enum ResolvedPlaylist {
    /// Index 0 when radios are configured.
    Radio,
    /// A Subsonic playlist, with its position in the Subsonic list.
    Subsonic(usize),
}

impl AppConfig {
    /// Whether the radio "playlist" occupies index 0.
    #[must_use]
    pub fn has_radio_playlist(&self) -> bool {
        !self.radios.is_empty()
    }

    /// Total number of unified playlists (radio + subsonic).
    #[must_use]
    pub fn unified_playlist_count(&self, subsonic_count: usize) -> usize {
        usize::from(self.has_radio_playlist()) + subsonic_count
    }

    /// Resolve a unified playlist index to radio or a Subsonic playlist offset.
    #[must_use]
    pub fn resolve_playlist_index(
        &self,
        index: usize,
        subsonic_count: usize,
    ) -> Option<ResolvedPlaylist> {
        let has_radio = self.has_radio_playlist();
        let total = self.unified_playlist_count(subsonic_count);
        if index >= total {
            return None;
        }
        if has_radio && index == 0 {
            Some(ResolvedPlaylist::Radio)
        } else {
            let sub_idx = if has_radio { index - 1 } else { index };
            Some(ResolvedPlaylist::Subsonic(sub_idx))
        }
    }
}

/// Resolved zone configuration with all conventions applied.
#[derive(Debug, Clone)]
pub struct ZoneConfig {
    /// Zone index (1-based).
    pub index: usize,
    /// Zone name.
    pub name: String,
    /// Emoji icon.
    pub icon: String,
    /// Snapcast sink path (e.g., `/snapsinks/zone1`).
    pub sink: String,
    /// Snapcast stream name (e.g., `Zone1`).
    pub stream_name: String,
    /// TCP source port for audio data to snapserver.
    pub tcp_source_port: u16,
    /// `AirPlay` receiver name (shown in `AirPlay` picker).
    pub airplay_name: String,
    /// KNX group addresses.
    pub knx: ZoneKnxAddresses,
    /// Group volume mode (resolved from zone override or global default).
    pub group_volume_mode: GroupVolumeMode,
    /// Presence-triggered playback configuration.
    pub presence: Option<PresenceConfig>,
}

/// Resolved KNX group addresses for a zone (all optional, explicit config only).
///
/// Same fields as [`RawZoneKnxConfig`] after address validation.
#[derive(Debug, Clone)]
pub struct ZoneKnxAddresses {
    /// Play command.
    pub play: Option<String>,
    /// Pause command.
    pub pause: Option<String>,
    /// Stop command.
    pub stop: Option<String>,
    /// Next track command.
    pub track_next: Option<String>,
    /// Previous track command.
    pub track_previous: Option<String>,
    /// Playback status feedback.
    pub control_status: Option<String>,
    /// Volume set (DPT 5.001 scaling).
    pub volume: Option<String>,
    /// Volume status feedback.
    pub volume_status: Option<String>,
    /// Relative volume dimming (DPT 3.007).
    pub volume_dim: Option<String>,
    /// Mute command.
    pub mute: Option<String>,
    /// Mute status feedback.
    pub mute_status: Option<String>,
    /// Mute toggle command.
    pub mute_toggle: Option<String>,
    /// Track title status feedback (DPT 16.001).
    pub track_title_status: Option<String>,
    /// Track artist status feedback (DPT 16.001).
    pub track_artist_status: Option<String>,
    /// Track album status feedback (DPT 16.001).
    pub track_album_status: Option<String>,
    /// Track progress status feedback (percentage).
    pub track_progress_status: Option<String>,
    /// Track playing status feedback (boolean).
    pub track_playing_status: Option<String>,
    /// Single-track repeat command.
    pub track_repeat: Option<String>,
    /// Single-track repeat status feedback.
    pub track_repeat_status: Option<String>,
    /// Single-track repeat toggle command.
    pub track_repeat_toggle: Option<String>,
    /// Playlist selection command (index).
    pub playlist: Option<String>,
    /// Playlist selection status feedback.
    pub playlist_status: Option<String>,
    /// Next playlist command.
    pub playlist_next: Option<String>,
    /// Previous playlist command.
    pub playlist_previous: Option<String>,
    /// Shuffle command.
    pub shuffle: Option<String>,
    /// Shuffle status feedback.
    pub shuffle_status: Option<String>,
    /// Shuffle toggle command.
    pub shuffle_toggle: Option<String>,
    /// Playlist repeat command.
    pub repeat: Option<String>,
    /// Playlist repeat status feedback.
    pub repeat_status: Option<String>,
    /// Playlist repeat toggle command.
    pub repeat_toggle: Option<String>,
    /// Presence command.
    pub presence: Option<String>,
    /// Presence enable/disable.
    pub presence_enable: Option<String>,
    /// Presence enable status feedback.
    pub presence_enable_status: Option<String>,
    /// Presence auto-off timeout.
    pub presence_timeout: Option<String>,
    /// Presence auto-off timeout status feedback.
    pub presence_timeout_status: Option<String>,
    /// Presence auto-off timer active status.
    pub presence_timer_status: Option<String>,
    /// Presence source override.
    pub presence_source_override: Option<String>,
}

/// Resolved client configuration.
#[derive(Debug, Clone)]
pub struct ClientConfig {
    /// Client index (1-based).
    pub index: usize,
    /// Client name.
    pub name: String,
    /// Snapcast client MAC address.
    pub mac: String,
    /// Zone index this client belongs to.
    pub zone_index: usize,
    /// Emoji icon.
    pub icon: String,
    /// Maximum volume (0–100). Limits how loud this client can go.
    pub max_volume: i32,
    /// KNX group addresses.
    pub knx: ClientKnxAddresses,
}

/// Resolved KNX group addresses for a client (all optional, explicit config only).
///
/// Same fields as [`RawClientKnxConfig`] after address validation.
#[derive(Debug, Clone)]
pub struct ClientKnxAddresses {
    /// Volume set (DPT 5.001 scaling).
    pub volume: Option<String>,
    /// Volume status feedback.
    pub volume_status: Option<String>,
    /// Relative volume dimming (DPT 3.007).
    pub volume_dim: Option<String>,
    /// Mute command.
    pub mute: Option<String>,
    /// Mute status feedback.
    pub mute_status: Option<String>,
    /// Mute toggle command.
    pub mute_toggle: Option<String>,
    /// Latency set (milliseconds).
    pub latency: Option<String>,
    /// Latency status feedback.
    pub latency_status: Option<String>,
    /// Zone assignment command.
    pub zone: Option<String>,
    /// Zone assignment status feedback.
    pub zone_status: Option<String>,
    /// Connection status feedback.
    pub connected_status: Option<String>,
}

/// Resolved radio station configuration.
#[derive(Debug, Clone)]
pub struct RadioConfig {
    /// Station name.
    pub name: String,
    /// Stream URL.
    pub url: String,
    /// Cover art URL.
    pub cover: Option<String>,
}

impl From<RawRadioConfig> for RadioConfig {
    fn from(raw: RawRadioConfig) -> Self {
        Self {
            name: raw.name,
            url: raw.url,
            cover: raw.cover,
        }
    }
}

// ── Defaults ──────────────────────────────────────────────────

const fn default_sample_rate() -> u32 {
    snapdog_common::DEFAULT_SAMPLE_RATE
}
const fn default_bit_depth() -> u16 {
    16
}
const fn default_channels() -> u16 {
    2
}
const fn default_http_port() -> u16 {
    5555
}
fn default_bind_address() -> String {
    "::".into()
}
fn default_snapcast_address() -> String {
    "127.0.0.1".into()
}
const fn default_jsonrpc_port() -> u16 {
    1705
}
const fn default_streaming_port() -> u16 {
    1704
}
fn default_server_name() -> String {
    "SnapDog".into()
}
fn default_mqtt_base_topic() -> String {
    "snapdog/".into()
}
fn default_zone_icon() -> String {
    "🎵".into()
}
fn default_client_icon() -> String {
    "🎵".into()
}
/// Default maximum volume (100%).
#[must_use]
pub const fn default_max_volume() -> i32 {
    100
}
/// Serde default returning `true`.
#[must_use]
pub const fn default_true() -> bool {
    true
}
