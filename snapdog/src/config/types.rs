// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! Configuration types: raw TOML structs and resolved application config.

use serde::{Deserialize, Serialize};

use super::mqtt::MqttConfig;

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
    /// Server display name. Used as the `mDNS` service instance name, the Home
    /// Assistant device name and the `name` in `GET /api/v1/system/version`.
    /// Default: "`SnapDog`".
    #[serde(default = "default_server_name")]
    pub name: String,
    /// System settings (log level, log file).
    #[serde(default)]
    pub system: SystemConfig,
    /// Audio output format (sample rate, bit depth, channels, codec).
    #[serde(default)]
    pub audio: AudioConfig,
    /// Network settings shared by every listener.
    #[serde(default)]
    pub network: NetworkConfig,
    /// HTTP server settings (port, API keys).
    #[serde(default)]
    pub http: HttpConfig,
    /// Snapcast connection and management settings.
    #[serde(default)]
    pub snapcast: SnapcastConfig,
    /// mDNS/Bonjour service advertisement.
    #[serde(default)]
    pub mdns: MdnsConfig,
    /// D-Bus MPRIS2 interface.
    #[serde(default)]
    pub dbus: DbusConfig,
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
    /// Optional log file, written in addition to stdout with daily rotation.
    /// The date is inserted before the extension (`snapdog.log` becomes
    /// `snapdog.2026-10-25.log`). `SnapDog` keeps 14 daily files and deletes older
    /// ones at start and then hourly. Only files named exactly like that are deleted,
    /// the oldest date first. Days change at midnight UTC. If the file cannot be
    /// opened, `SnapDog` logs a warning and continues with stdout only.
    pub log_file: Option<String>,
    /// Directory for persistent state files (`zones.json`, `eq.json`,
    /// `server_id`, and `airplay-pairing` with the `ap2` feature).
    /// Defaults to a platform-appropriate path.
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

    /// The base volume at which a client plays at `effective`: the inverse of
    /// [`effective`](Self::effective), for volumes a client reports itself.
    ///
    /// Clamped to `max_volume`, so the result may play quieter than asked.
    /// `None` when no base volume can change the result: in absolute mode, or
    /// when the zone is at 0.
    #[must_use]
    pub fn base_for(self, effective: i32, zone_volume: i32, max_volume: i32) -> Option<i32> {
        let max = max_volume.clamp(0, 100);
        let e = effective.clamp(0, 100);
        let z = zone_volume.clamp(0, 100);
        if z == 0 {
            return None;
        }
        match self {
            Self::Absolute => None,
            // Round up: `effective` truncates, so this is the smallest base that reaches `e`.
            Self::Relative => Some(((e * 100 + z - 1) / z).min(max)),
            Self::Compressed => {
                let factor = (f64::from(z) / 100.0).sqrt();
                Some((f64::from(e) / factor).round().min(f64::from(max)) as i32)
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
    /// Set to 0 to disable. Only applies to `SnapDog` clients. Max: 1000ms.
    #[serde(
        default = "default_zone_switch_fade_ms",
        deserialize_with = "deserialize_fade_ms"
    )]
    pub zone_switch_fade_ms: u16,
    /// Fade duration in milliseconds when switching audio sources within a zone
    /// (e.g., radio → subsonic). Set to 0 to disable. Max: 1000ms.
    #[serde(
        default = "default_zone_switch_fade_ms",
        deserialize_with = "deserialize_fade_ms"
    )]
    pub source_switch_fade_ms: u16,
}

/// Maximum fade duration in milliseconds.
const MAX_FADE_MS: u16 = 1000;

const fn default_zone_switch_fade_ms() -> u16 {
    snapdog_common::DEFAULT_FADE_MS
}

fn deserialize_fade_ms<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<u16, D::Error> {
    let v = u16::deserialize(deserializer)?;
    if v > MAX_FADE_MS {
        tracing::warn!(
            value = v,
            max = MAX_FADE_MS,
            "Fade duration exceeds maximum, clamping"
        );
    }
    Ok(v.min(MAX_FADE_MS))
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

/// Network settings shared by every listener (`[network]`).
///
/// `bind` is the default for every service that has no `bind` of its own, see RFC
/// LBD-0008. Unset, each service keeps its built-in default (HTTP `::`, the others
/// the IPv4 wildcard).
#[derive(Debug, Deserialize, Clone, Default)]
pub struct NetworkConfig {
    /// Where the services listen unless they set their own `bind`: one entry or a list of
    /// IP addresses, interface names (`"eth0"`) or `"loopback"`, for example
    /// `["eth0"]` or `["192.168.1.10"]`. A name stands for the IPv4 and non-link-local IPv6
    /// addresses of that interface when the file is read, see RFC LBD-0009. A service whose
    /// backend takes one address (Snapcast streaming, KNX) uses this only when the result is
    /// exactly one address.
    #[serde(default, deserialize_with = "address_list")]
    pub bind: Option<Vec<std::net::IpAddr>>,
}

/// One value or a list of values: `bind = "10.0.0.1"` and `bind = ["10.0.0.1", "::1"]`.
#[derive(Deserialize)]
#[serde(untagged)]
enum OneOrMany<T> {
    One(T),
    Many(Vec<T>),
}

/// The `[http].bind` setting: one entry or a list. An entry that is an address, `loopback`
/// or the name of an interface is replaced by the addresses it stands for (RFC LBD-0009).
/// Anything else is kept as written, so a host name still reaches the bind and fails there,
/// with its name.
fn http_bind_list<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let entries = match OneOrMany::<String>::deserialize(deserializer)? {
        OneOrMany::One(one) => vec![one],
        OneOrMany::Many(many) => many,
    };
    let mut bind = Vec::with_capacity(entries.len());
    for entry in entries {
        if super::interfaces::is_address_or_known_name(&entry) {
            let addrs =
                super::interfaces::resolve_here(&entry).map_err(serde::de::Error::custom)?;
            bind.extend(addrs.iter().map(ToString::to_string));
        } else {
            bind.push(entry);
        }
    }
    Ok(bind)
}

/// A bind setting written as one entry or a list of entries: addresses, `loopback` or
/// interface names, all turned into addresses.
fn address_list<'de, D>(deserializer: D) -> Result<Option<Vec<std::net::IpAddr>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error;
    let Some(value) = Option::<OneOrMany<String>>::deserialize(deserializer)? else {
        return Ok(None);
    };
    let entries = match value {
        OneOrMany::One(one) => vec![one],
        OneOrMany::Many(many) => many,
    };
    let mut addrs = Vec::new();
    for entry in &entries {
        for ip in super::interfaces::resolve_here(entry).map_err(D::Error::custom)? {
            if !addrs.contains(&ip) {
                addrs.push(ip);
            }
        }
    }
    Ok(Some(addrs))
}

/// A bind setting of a service that listens on exactly one address. A list with one entry
/// is accepted, a longer one is an error that says so, instead of taking the first entry.
/// The entry is an address, `loopback` or an interface with exactly one IPv4 address.
fn single_address<'de, D>(deserializer: D) -> Result<Option<std::net::IpAddr>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error;
    let entry = match Option::<OneOrMany<String>>::deserialize(deserializer)? {
        None => return Ok(None),
        Some(OneOrMany::One(one)) => one,
        Some(OneOrMany::Many(many)) => match many.as_slice() {
            [one] => one.clone(),
            _ => {
                return Err(D::Error::custom(format!(
                    "this service listens on exactly one address, but the list has {}",
                    many.len()
                )));
            }
        },
    };
    super::interfaces::resolve_single_here(&entry)
        .map(Some)
        .map_err(D::Error::custom)
}

/// HTTP server configuration.
#[derive(Debug, Deserialize, Clone)]
pub struct HttpConfig {
    /// Port for the REST API, WebSocket, and embedded `WebUI`.
    #[serde(default = "default_http_port")]
    pub port: u16,
    /// Where to listen: one string or a list of IP addresses, interface names or
    /// `"loopback"`. Unset: `[network].bind`, then `::` (dual-stack IPv4 and IPv6 where the
    /// OS allows it). Use `0.0.0.0` for IPv4 only.
    /// After loading this is never empty, see [`HttpConfig::bind_addresses`].
    #[serde(default, deserialize_with = "http_bind_list")]
    pub bind: Vec<String>,
    /// External base URL (e.g., `http://192.168.2.20:5555` or `https://music.example.com`).
    /// Used for absolute URLs in API responses. Defaults to `http://localhost:<port>`.
    #[serde(default = "default_base_url")]
    pub base_url: String,
    /// TLS certificate path (PEM fullchain). If set with `tls_key`, enables HTTPS.
    #[serde(default)]
    pub tls_cert: Option<std::path::PathBuf>,
    /// TLS private key path (PEM, unencrypted). Required when `tls_cert` is set.
    #[serde(default)]
    pub tls_key: Option<std::path::PathBuf>,
    /// Optional API keys. If set, all API endpoints require `Authorization: Bearer <key>`.
    #[serde(default)]
    pub api_keys: Vec<SecretString>,
    /// Serve the `OpenAPI` document at `GET /api/v1/openapi.json`, behind the API key
    /// like the rest of `/api/v1` (only if feature `api-docs` is enabled).
    /// Default: true.
    #[serde(default = "default_true")]
    pub api_docs: bool,
    /// Serve the interactive API console (Scalar) at `/docs` (only if feature
    /// `api-docs` is enabled and `api_docs` is true). The page embeds the whole
    /// `OpenAPI` document and is served WITHOUT the API key, because a browser
    /// cannot send the `Authorization` header when opening a page. The page and its
    /// pinned Scalar script come from `SnapDog` itself and contact no other host, on
    /// the origin of the web UI. Default: false.
    #[serde(default)]
    pub api_console: bool,
}

impl HttpConfig {
    /// The addresses to listen on: what is configured, or the built-in default `::`.
    #[must_use]
    pub fn bind_addresses(&self) -> Vec<String> {
        if self.bind.is_empty() {
            vec![default_bind_address()]
        } else {
            self.bind.clone()
        }
    }

    /// The one address the server listens on, when it is a specific one: what the mDNS
    /// advertisement and the D-Bus cover URLs name. `None` for a wildcard, a list or no setting.
    #[must_use]
    pub fn single_specific_bind(&self) -> Option<String> {
        match self.bind.as_slice() {
            [one]
                if one
                    .parse::<std::net::IpAddr>()
                    .map_or(true, |ip| !ip.is_unspecified()) =>
            {
                Some(one.clone())
            }
            _ => None,
        }
    }
}

impl Default for HttpConfig {
    fn default() -> Self {
        Self {
            port: default_http_port(),
            bind: Vec::new(),
            base_url: default_base_url(),
            tls_cert: None,
            tls_key: None,
            api_keys: vec![],
            api_docs: true,
            api_console: false,
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

/// D-Bus MPRIS2 configuration.
#[derive(Debug, Deserialize, Clone)]
pub struct DbusConfig {
    /// Enable MPRIS2 D-Bus interface. Default: true.
    #[serde(default = "default_true")]
    pub enabled: bool,
}

impl Default for DbusConfig {
    fn default() -> Self {
        Self { enabled: true }
    }
}

/// Snapcast server connection and management.
#[derive(Debug, Deserialize, Clone)]
pub struct SnapcastConfig {
    /// Address of an external `snapserver`. Only used by builds with the
    /// `snapcast-process` feature. The default embedded server always listens
    /// on all interfaces, so any other value is ignored there (with a warning).
    #[serde(default = "default_snapcast_address")]
    pub address: String,
    /// JSON-RPC control port of an external `snapserver` (default: 1705, alias
    /// `jsonrpc_port`). Only used by builds with the `snapcast-process`
    /// feature. The embedded server has no JSON-RPC listener, so any other
    /// value is ignored there (with a warning).
    #[serde(default = "default_jsonrpc_port", alias = "jsonrpc_port")]
    pub jsonrpc_tcp_port: u16,
    /// Audio streaming port (default: 1704). Snapcast clients connect here.
    ///
    /// This port carries the audio stream and has no authentication: whoever can reach
    /// it can connect as a client and receive the audio. Restrict it with
    /// `streaming_bind` and the firewall, and use codec `f32lz4e` with your own
    /// `encryption_psk` if the stream must not be readable on the network.
    #[serde(default = "default_streaming_port")]
    pub streaming_port: u16,
    /// Local address the streaming port listens on, for example `192.168.10.5` for one
    /// interface or `127.0.0.1` for this host only. Unset: all interfaces (`0.0.0.0` in
    /// the embedded server, what an external `snapserver` does by default otherwise).
    /// A single address, not a list.
    #[serde(default, deserialize_with = "single_address")]
    pub streaming_bind: Option<std::net::IpAddr>,
    /// Start an external `snapserver` binary as a child process (default:
    /// true). Only used by builds with the `snapcast-process` feature. The
    /// embedded Rust server in the default build always runs, so `false` is
    /// ignored there (with a warning).
    #[serde(default = "default_true")]
    pub managed: bool,
    /// Show the console output of the external `snapserver` child process.
    /// Only used by builds with the `snapcast-process` feature; ignored (with a
    /// warning) by the embedded server.
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
    /// Pre-shared key for `f32lz4e` encryption. Without a key the built-in
    /// default key is used, which is public and protects nothing, so set one
    /// when you use `f32lz4e` (`SnapDog` logs a warning otherwise). Can also be
    /// set with `SNAPDOG_SNAPCAST_ENCRYPTION_PSK`. An empty key, from the file or
    /// the variable, is never used: with `f32lz4e` it stops the start, with
    /// another codec it is ignored (the variable with a warning).
    #[serde(default)]
    pub encryption_psk: Option<SecretString>,
    /// Default group volume mode for all zones.
    #[serde(default)]
    pub group_volume_mode: GroupVolumeMode,
}

impl SnapcastConfig {
    /// Socket address the embedded server listens on for the audio stream.
    #[must_use]
    pub fn streaming_socket_addr(&self) -> std::net::SocketAddr {
        let ip = self
            .streaming_bind
            .unwrap_or(std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED));
        std::net::SocketAddr::new(ip, self.streaming_port)
    }
}

impl Default for SnapcastConfig {
    fn default() -> Self {
        Self {
            address: default_snapcast_address(),
            jsonrpc_tcp_port: default_jsonrpc_port(),
            streaming_port: default_streaming_port(),
            streaming_bind: None,
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
    /// `AirPlay` protocol mode: `"airplay1"` or `"airplay2"` (default). Only
    /// takes effect in builds with the `ap2` feature; other builds always speak
    /// `AirPlay` 1.
    #[serde(default)]
    pub mode: AirplayMode,
    /// Where to listen: one entry or a list of IP addresses, interface names or `"loopback"`,
    /// for example `["192.168.1.10"]` or `["eth0"]`. Default: `[network].bind`, then all
    /// interfaces. Only takes effect in builds with the `ap2` feature (ignored with a warning
    /// otherwise).
    #[serde(default, deserialize_with = "address_list")]
    pub bind: Option<Vec<std::net::IpAddr>>,
}

/// Spotify Connect receiver settings.
#[derive(Debug, Deserialize, Clone)]
pub struct SpotifyConfig {
    /// The name of the Spotify Connect device, for every zone that does not set
    /// `[[zone]].spotify_name`. With one zone the device is named exactly this; with
    /// several zones each gets `"<name> <zone name>"`, because every zone runs its own
    /// device and Spotify lists devices by name. Unset or blank: the zone name.
    /// A zone's own `spotify_name` always wins. Changing a name makes the device appear
    /// as a new one in the Spotify app (its Zeroconf id is derived from the name).
    #[serde(default)]
    pub name: String,
    /// Audio bitrate: 96, 160, or 320 kbps. Default: 320.
    #[serde(default = "default_spotify_bitrate")]
    pub bitrate: u32,
    /// TCP port of the Zeroconf (Spotify Connect) HTTP server. Default: 7100, above the `AirPlay` ports (7001 and up, one per zone).
    ///
    /// A fixed port keeps firewall rules and monitoring stable. Zone `n` listens on
    /// `port + n`. `0` picks a random port at every start (not firewall-friendly).
    #[serde(default = "default_spotify_port")]
    pub port: u16,
    /// Zeroconf announcement backend: `"libmdns"` (default), `"avahi"` or `"dns-sd"`.
    ///
    /// `libmdns` announces on the default route's interface only; `avahi` hands the
    /// announcement to the host's avahi-daemon (system D-Bus), which covers all interfaces.
    #[serde(default)]
    pub zeroconf_backend: Option<String>,
    /// Not supported: Spotify Connect's discovery server listens on all interfaces and
    /// cannot be bound to an address. Accepted only so that setting it is an error that says
    /// why, instead of a key that silently does nothing.
    #[serde(default, deserialize_with = "address_list")]
    pub bind: Option<Vec<std::net::IpAddr>>,
}

impl SpotifyConfig {
    /// Stable device ID derived from the name (for Zeroconf). Each zone has its own
    /// receiver with its own name, so the ids differ as long as the names do.
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

const fn default_spotify_port() -> u16 {
    7100
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

/// KNX/IP integration settings.
#[derive(Debug, Deserialize, Clone)]
pub struct KnxConfig {
    /// Device role: `client` (default) or `device` (ETS-programmable).
    #[serde(default, alias = "mode")]
    pub role: KnxRole,
    /// KNX/IP gateway URL for tunnel/routing.
    /// Required for client role, ignored in device role (the device is its own KNXnet/IP server
    /// on UDP 3671). `udp://`, `tunnel://` or `router://` followed by `host:port`; `udp://` picks
    /// routing for a multicast address and tunnelling otherwise.
    /// Examples: `udp://192.168.1.50:3671` (tunnel), `udp://224.0.23.12:3671` (routing)
    pub url: Option<String>,
    /// Local IPv4 address of the interface the KNX device server (device role) uses for the
    /// KNX multicast group, for example `"192.168.1.10"`. Unset: `[network].bind` when it
    /// has exactly one address, else the system default. The UDP socket on port 3671
    /// itself stays on all interfaces, the library binds it that way.
    #[serde(default, deserialize_with = "single_address")]
    pub bind: Option<std::net::IpAddr>,
    /// KNX individual address (device role). Example: `"1.1.100"`
    pub individual_address: Option<String>,
    /// Persist ETS-programmed configuration across restarts (device role).
    pub persist_ets_config: Option<bool>,
    /// Restart the snapdog process when ETS finishes a programming session
    /// (device role), so the new parameters are applied at boot. Default: true.
    pub restart_after_ets: Option<bool>,
    /// Start with programming mode active (set via --knx-prog-mode CLI flag).
    #[serde(default)]
    pub start_prog_mode: bool,

    // ── Global (device-level) group addresses ────────────────────
    /// Cyclic "server online" heartbeat (send).
    pub server_online: Option<String>,
    /// "All stop" trigger — stops every zone (receive).
    pub all_stop: Option<String>,
    /// "All mute" switch — mutes/unmutes every zone (receive).
    pub all_mute: Option<String>,
    /// "All mute" status (send).
    pub all_mute_status: Option<String>,
    /// "System fault" alarm (send) — set while the server is unhealthy.
    pub system_fault: Option<String>,
    /// KNX time-of-day clock input (receive) — drives presence schedules.
    pub knx_time: Option<String>,
    /// Heartbeat interval in minutes for the "server online" cyclic send.
    #[serde(default = "default_heartbeat_minutes")]
    pub heartbeat_minutes: u16,
    /// Also set the OS system clock from received KNX time
    /// (set via the `--knx-sync-system-clock` CLI flag).
    #[serde(default)]
    pub sync_system_clock: bool,
}

/// Default heartbeat interval (minutes) for the global "server online" send.
const fn default_heartbeat_minutes() -> u16 {
    5
}

impl Default for KnxConfig {
    /// The same values a `[knx]` table without keys deserializes to, so a table created from
    /// the command line (`--knx-device`, `--knx-sync-system-clock`) behaves like one from
    /// TOML. The derived default would have left the heartbeat at 0, which the publisher
    /// clamps to one minute.
    fn default() -> Self {
        Self {
            role: KnxRole::default(),
            url: None,
            bind: None,
            individual_address: None,
            persist_ets_config: None,
            restart_after_ets: None,
            start_prog_mode: false,
            server_online: None,
            all_stop: None,
            all_mute: None,
            all_mute_status: None,
            system_fault: None,
            knx_time: None,
            heartbeat_minutes: default_heartbeat_minutes(),
            sync_system_clock: false,
        }
    }
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
    /// Override Spotify Connect receiver name (default: zone name).
    pub spotify_name: Option<String>,
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
    /// Presence auto-off timer active status (DPT 1.001).
    pub presence_timer_status: Option<String>,
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
    /// Initial volume (0–100) applied when the client first connects.
    #[serde(default = "default_volume")]
    pub default_volume: i32,
    /// Initial audio latency in ms (Snapcast latency compensation).
    #[serde(default)]
    pub default_latency: i32,
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
    /// Network settings shared by every listener.
    pub network: NetworkConfig,
    /// HTTP server settings.
    pub http: HttpConfig,
    /// Snapcast connection settings.
    pub snapcast: SnapcastConfig,
    /// mDNS/Bonjour service advertisement.
    pub mdns: MdnsConfig,
    /// D-Bus MPRIS2 interface.
    pub dbus: DbusConfig,
    /// `AirPlay` receiver settings.
    pub airplay: AirplayConfig,
    /// Subsonic connection (if configured).
    pub subsonic: Option<SubsonicConfig>,
    /// Spotify Connect receiver settings (if configured).
    pub spotify: Option<SpotifyConfig>,
    /// MQTT bridge (if configured and valid).
    pub mqtt: Option<MqttConfig>,
    /// Why MQTT is off although a `[mqtt]` section exists: the section is invalid.
    /// The server runs without MQTT, logs this at start-up as an error, and
    /// `--check-config` reports it as a failure.
    pub mqtt_problem: Option<String>,
    /// KNX settings.
    pub knx: Option<KnxConfig>,
    /// Resolved zone configurations (1-indexed).
    pub zones: Vec<ZoneConfig>,
    /// Resolved client configurations (1-indexed).
    pub clients: Vec<ClientConfig>,
    /// Radio station list.
    pub radios: Vec<RadioConfig>,
    /// Messages collected while loading and validating the configuration.
    /// Logging is not set up yet at that point, so the caller emits them with
    /// [`AppConfig::log_notices`] once it is.
    pub notices: Vec<ConfigNotice>,
}

/// Severity of a [`ConfigNotice`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoticeLevel {
    /// Informational, logged at `info`.
    Info,
    /// Something in the configuration is ignored, deprecated or unsafe.
    Warn,
}

/// A message about the loaded configuration, produced before logging exists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigNotice {
    /// How serious the message is.
    pub level: NoticeLevel,
    /// Human-readable text. Names the key and says what to do instead.
    pub message: String,
    /// Computed from the resolved settings (see `notices::collect`) rather than
    /// recorded while loading. Such notices are recomputed whenever the
    /// settings change, so a command-line override replaces a stale one.
    pub derived: bool,
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
    /// Record a warning to be logged once logging is set up. Duplicates are dropped.
    pub fn warn(&mut self, message: impl Into<String>) {
        self.push_notice(NoticeLevel::Warn, message.into());
    }

    /// Record an informational message to be logged once logging is set up.
    pub fn info(&mut self, message: impl Into<String>) {
        self.push_notice(NoticeLevel::Info, message.into());
    }

    fn push_notice(&mut self, level: NoticeLevel, message: String) {
        let notice = ConfigNotice {
            level,
            message,
            derived: false,
        };
        if !self
            .notices
            .iter()
            .any(|n| n.level == notice.level && n.message == notice.message)
        {
            self.notices.push(notice);
        }
    }

    /// Log every collected [`ConfigNotice`]. Call after the tracing subscriber is installed.
    pub fn log_notices(&self) {
        for notice in &self.notices {
            match notice.level {
                NoticeLevel::Info => {
                    tracing::info!(target: "snapdog::config", "{}", notice.message);
                }
                NoticeLevel::Warn => {
                    tracing::warn!(target: "snapdog::config", "{}", notice.message);
                }
            }
        }
    }

    /// Whether the radio "playlist" occupies index 0.
    #[must_use]
    pub const fn has_radio_playlist(&self) -> bool {
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
    /// Spotify Connect receiver name (shown in the Spotify device picker).
    pub spotify_name: String,
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
    /// Presence auto-off timer active status.
    pub presence_timer_status: Option<String>,
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
    /// Initial volume (0–100) applied when the client first connects.
    pub default_volume: i32,
    /// Initial audio latency in ms (Snapcast latency compensation).
    pub default_latency: i32,
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
/// Default initial client volume (50%).
#[must_use]
pub const fn default_volume() -> i32 {
    50
}
/// Serde default returning `true`.
#[must_use]
pub const fn default_true() -> bool {
    true
}

#[cfg(test)]
mod spotify_config_tests {
    use super::SpotifyConfig;

    #[test]
    fn spotify_defaults_to_fixed_port_and_default_backend() {
        let cfg: SpotifyConfig = toml::from_str(r#"name = "Docks""#).unwrap();
        assert_eq!(cfg.port, 7100);
        assert_eq!(cfg.bitrate, 320);
        assert!(cfg.zeroconf_backend.is_none());
    }

    #[test]
    fn spotify_port_and_backend_are_configurable() {
        let cfg: SpotifyConfig =
            toml::from_str("name = \"Docks\"\nport = 0\nzeroconf_backend = \"avahi\"").unwrap();
        assert_eq!(cfg.port, 0);
        assert_eq!(cfg.zeroconf_backend.as_deref(), Some("avahi"));
    }
}
