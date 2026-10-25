// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! Configuration loading and validation.
//!
//! Single TOML file → all derived config (KNX addresses, sink paths, snapserver.conf).
//! Convention over configuration: sensible defaults, auto-generated where possible.

mod convention;
mod notices;
mod types;

pub use types::*;

use std::path::Path;

use anyhow::{Context, Result};

/// Load, validate, and resolve configuration from a TOML file.
///
/// # Errors
///
/// Returns an error if the file cannot be read or contains invalid TOML/config.
pub fn load(path: &Path) -> Result<AppConfig> {
    let content = std::fs::read_to_string(path) /* blocking OK: called once at startup */
        .with_context(|| format!("Failed to read {}", path.display()))?;
    load_str(&content).with_context(|| format!("Failed to parse {}", path.display()))
}

/// Like [`load`], but takes the TOML text. Keys that no config struct knows
/// (typos, removed keys) are accepted, so existing files keep loading, but are
/// reported through [`AppConfig::notices`].
///
/// # Errors
///
/// Returns an error if the text is not valid TOML or the configuration is invalid.
pub fn load_str(content: &str) -> Result<AppConfig> {
    let mut unknown = Vec::new();
    let raw: FileConfig =
        serde_ignored::deserialize(toml::Deserializer::parse(content)?, |path| {
            unknown.push(path.to_string());
        })?;
    let mut config = load_raw(raw)?;
    for key in unknown {
        config.warn(format!(
            "Unknown configuration key `{}` is ignored. Check the spelling against snapdog.example.toml.",
            display_key_path(&key)
        ));
    }
    Ok(config)
}

/// Unknown-key list for a TOML text, in the form used in log messages.
/// Mainly for tests that need to assert a file contains no stale keys.
///
/// # Errors
///
/// Returns an error if the text cannot be parsed into the config structure.
pub fn unknown_keys(content: &str) -> Result<Vec<String>> {
    let mut unknown = Vec::new();
    let _: FileConfig = serde_ignored::deserialize(toml::Deserializer::parse(content)?, |path| {
        unknown.push(display_key_path(&path.to_string()));
    })?;
    Ok(unknown)
}

/// Make a `serde_ignored` path readable: `zone.0.knx.foo` becomes `zone[0].knx.foo`
/// and the `?` that marks an `Option` is dropped (`spotify.?.name` becomes `spotify.name`).
#[must_use]
pub fn display_key_path(path: &str) -> String {
    let mut out = String::new();
    for segment in path.split('.').filter(|s| !s.is_empty() && *s != "?") {
        if segment.bytes().all(|b| b.is_ascii_digit()) {
            out.push('[');
            out.push_str(segment);
            out.push(']');
        } else {
            if !out.is_empty() {
                out.push('.');
            }
            out.push_str(segment);
        }
    }
    out
}

/// Expand a leading `~` or `~/` to the user's home directory. Other forms
/// (`~user`, a `~` elsewhere) and a missing home directory leave the text unchanged.
#[must_use]
pub fn expand_tilde(path: &str) -> String {
    let home = || {
        std::env::var("HOME")
            .ok()
            .or_else(|| std::env::var("USERPROFILE").ok())
            .filter(|h| !h.is_empty())
    };
    if path == "~" {
        return home().unwrap_or_else(|| path.to_string());
    }
    match (path.strip_prefix("~/"), home()) {
        (Some(rest), Some(home)) => format!("{}/{rest}", home.trim_end_matches('/')),
        _ => path.to_string(),
    }
}

/// Apply `~` expansion to every path-valued key a user writes by hand.
fn expand_paths(raw: &mut FileConfig) {
    raw.system.state_dir = expand_tilde(&raw.system.state_dir);
    raw.system.log_file = raw.system.log_file.take().map(|p| expand_tilde(&p));
    if let Some(subsonic) = raw.subsonic.as_mut() {
        subsonic.cache.path = expand_tilde(&subsonic.cache.path);
    }
    let expand = |p: &mut Option<std::path::PathBuf>| {
        if let Some(path) = p.as_ref().and_then(|p| p.to_str()) {
            *p = Some(std::path::PathBuf::from(expand_tilde(path)));
        }
    };
    expand(&mut raw.http.tls_cert);
    expand(&mut raw.http.tls_key);
}

/// Resolve raw TOML config into fully populated `AppConfig` with conventions applied.
///
/// # Errors
///
/// Returns an error if the configuration is invalid.
pub fn load_raw(raw: FileConfig) -> Result<AppConfig> {
    load_raw_inner(raw, false)
}

/// Like `load_raw` but skips zone/client validation (for KNX device mode).
///
/// # Errors
///
/// Returns an error if the configuration is invalid.
pub fn load_raw_no_validate(raw: FileConfig) -> Result<AppConfig> {
    load_raw_inner(raw, true)
}

fn load_raw_inner(mut raw: FileConfig, skip_zone_validation: bool) -> Result<AppConfig> {
    expand_paths(&mut raw);
    if !skip_zone_validation {
        anyhow::ensure!(
            !raw.zone.is_empty(),
            "At least one [[zone]] must be configured"
        );
        // Clients auto-register when snapcast.unknown_clients = "accept", so a
        // static [[client]] is only required for the ignore/reject policies.
        anyhow::ensure!(
            !raw.client.is_empty() || raw.snapcast.unknown_clients == UnknownClientPolicy::Accept,
            "At least one [[client]] must be configured (unless snapcast.unknown_clients = \"accept\")"
        );
    }

    // ── Uniqueness & format validation ─────────────────────────
    validate_raw_inputs(&raw)?;

    // ── KNX address validation ───────────────────────────────
    if let Some(ref knx) = raw.knx
        && let Some(ref addr) = knx.individual_address
    {
        validate_knx_ia(addr).context("Invalid KNX individual address")?;
    }
    for zone in &raw.zone {
        validate_zone_knx(&zone.knx).with_context(|| format!("Zone '{}' KNX error", zone.name))?;
    }
    for client in &raw.client {
        validate_client_knx(&client.knx)
            .with_context(|| format!("Client '{}' KNX error", client.name))?;
    }

    let zones: Vec<ZoneConfig> = raw
        .zone
        .into_iter()
        .enumerate()
        .map(|(i, z)| convention::resolve_zone(i + 1, z, &raw.audio, &raw.snapcast))
        .collect::<Vec<_>>();

    let zone_names: Vec<&str> = zones.iter().map(|z| z.name.as_str()).collect();

    let clients: Vec<ClientConfig> = raw
        .client
        .into_iter()
        .enumerate()
        .map(|(i, c)| convention::resolve_client(i + 1, c, &zone_names))
        .collect::<Result<_>>()?;

    let radios: Vec<RadioConfig> = raw.radio.into_iter().map(Into::into).collect();

    // ── KNX mode validation ───────────────────────────────────
    if let Some(ref knx) = raw.knx {
        match knx.role {
            KnxRole::Client => {
                anyhow::ensure!(
                    knx.url.is_some(),
                    "KNX client mode requires 'url' (e.g. udp://192.168.1.50:3671)"
                );
            }
            KnxRole::Device => {
                anyhow::ensure!(
                    knx.individual_address.is_some(),
                    "KNX device mode requires 'individual_address' (e.g. 1.1.100)"
                );
            }
        }
    }

    if let Some(ref subsonic) = raw.subsonic {
        let url = url::Url::parse(&subsonic.url).context("Invalid Subsonic URL")?;
        anyhow::ensure!(
            matches!(url.scheme(), "http" | "https"),
            "Subsonic URL must use http or https"
        );
    }

    // ── Presence validation ───────────────────────────────────
    for zone in &zones {
        validate_presence(zone, radios.len())?;
    }

    let name = raw.name;

    let mut config = AppConfig {
        name,
        system: raw.system,
        audio: raw.audio,
        http: raw.http,
        snapcast: raw.snapcast,
        mdns: raw.mdns,
        dbus: raw.dbus,
        airplay: raw.airplay,
        subsonic: raw.subsonic,
        spotify: raw.spotify,
        mqtt: raw.mqtt,
        knx: raw.knx,
        zones,
        clients,
        radios,
        notices: Vec::new(),
    };

    apply_env_overrides(&mut config);
    validate(&mut config)?;
    notices::collect(&mut config);

    Ok(config)
}

/// Cross-field validation and normalization. Runs after all overrides are applied,
/// and again after command-line overrides (see `main`), so it must be idempotent.
///
/// # Errors
///
/// Returns an error if a value is out of range or two settings contradict each other.
pub fn validate(config: &mut AppConfig) -> Result<()> {
    // ── Audio format constraints ──────────────────────────────
    validate_audio_format(&config.audio)?;

    // FLAC supports max 24-bit
    if config.snapcast.codec == AudioCodec::Flac && config.audio.bit_depth > 24 {
        config.warn(format!(
            "bit_depth {} is not supported by FLAC (maximum 24), using 24",
            config.audio.bit_depth
        ));
        config.audio.bit_depth = 24;
    }

    // f32lz4 always uses 32-bit float internally — bit_depth is ignored
    if matches!(
        config.snapcast.codec,
        AudioCodec::F32lz4 | AudioCodec::F32lz4e
    ) && config.audio.bit_depth != 32
    {
        config.info(format!(
            "codec {} uses native 32-bit float, the configured bit_depth {} is ignored",
            config.snapcast.codec, config.audio.bit_depth
        ));
    }

    // ── Port collision ────────────────────────────────────────
    if config.http.port == config.snapcast.streaming_port {
        anyhow::bail!(
            "http.port ({}) and snapcast.streaming_port ({}) must not be the same",
            config.http.port,
            config.snapcast.streaming_port
        );
    }

    // ── Client max_volume ─────────────────────────────────────
    let mut clamped = Vec::new();
    for client in &mut config.clients {
        if client.max_volume > 100 {
            clamped.push(format!(
                "Client '{}': max_volume {} exceeds 100, using 100",
                client.name, client.max_volume
            ));
            client.max_volume = 100;
        } else if client.max_volume < 0 {
            clamped.push(format!(
                "Client '{}': max_volume {} is below 0, using 0",
                client.name, client.max_volume
            ));
            client.max_volume = 0;
        }
    }
    for message in clamped {
        config.warn(message);
    }

    Ok(())
}

/// Check sample rate, bit depth and channel count. Shared by file loading and the
/// re-validation after command-line overrides, so `--sample-rate 12345` fails like
/// `sample_rate = 12345` does.
fn validate_audio_format(audio: &AudioConfig) -> Result<()> {
    anyhow::ensure!(
        matches!(
            audio.sample_rate,
            44100 | 48000 | 88200 | 96000 | 176_400 | 192_000
        ),
        "Unsupported sample rate: {}. Use 44100, 48000, 88200, 96000, 176400, or 192000",
        audio.sample_rate
    );
    anyhow::ensure!(
        matches!(audio.bit_depth, 16 | 24 | 32),
        "Unsupported bit depth: {}. Use 16, 24, or 32",
        audio.bit_depth
    );
    anyhow::ensure!(
        audio.channels > 0 && audio.channels <= 8,
        "Unsupported number of channels: {}. Use 1 to 8",
        audio.channels
    );
    Ok(())
}

/// Validate uniqueness of zone/client names and audio format parameters.
fn validate_raw_inputs(raw: &FileConfig) -> Result<()> {
    let mut zone_names_set = std::collections::HashSet::new();
    for zone in &raw.zone {
        if !zone_names_set.insert(&zone.name) {
            anyhow::bail!("Duplicate zone name: '{}'", zone.name);
        }
    }

    let mut client_names_set = std::collections::HashSet::new();
    let mut client_macs_set = std::collections::HashSet::new();
    for client in &raw.client {
        if !client_names_set.insert(&client.name) {
            anyhow::bail!("Duplicate client name: '{}'", client.name);
        }
        let mac = client.mac.to_lowercase();
        if !client_macs_set.insert(mac.clone()) {
            anyhow::bail!("Duplicate client MAC address: '{}'", client.mac);
        }
        mac.parse::<mac_address::MacAddress>()
            .with_context(|| format!("Invalid MAC address format: '{}'", client.mac))?;
    }

    validate_audio_format(&raw.audio)?;
    Ok(())
}

/// Validate presence schedule for a zone (time ranges, overlaps, radio indices).
fn validate_presence(zone: &ZoneConfig, num_radios: usize) -> Result<()> {
    let Some(ref presence) = zone.presence else {
        return Ok(());
    };
    let mut intervals: Vec<(u16, u16, usize)> = Vec::new();
    for (i, entry) in presence.schedule.iter().enumerate() {
        let from = types::parse_time(&entry.from).with_context(|| {
            format!(
                "Zone '{}' presence schedule[{i}]: invalid 'from' time '{}'",
                zone.name, entry.from
            )
        })?;
        let to = types::parse_time(&entry.to).with_context(|| {
            format!(
                "Zone '{}' presence schedule[{i}]: invalid 'to' time '{}'",
                zone.name, entry.to
            )
        })?;
        anyhow::ensure!(
            from < to,
            "Zone '{}' presence schedule[{i}]: 'from' ({}) must be before 'to' ({}). For overnight, use two entries.",
            zone.name,
            entry.from,
            entry.to
        );
        for &(pf, pt, j) in &intervals {
            anyhow::ensure!(
                to <= pf || from >= pt,
                "Zone '{}' presence schedule[{i}] ({}-{}) overlaps with schedule[{j}] ({:02}:{:02}-{:02}:{:02})",
                zone.name,
                entry.from,
                entry.to,
                pf / 60,
                pf % 60,
                pt / 60,
                pt % 60
            );
        }
        intervals.push((from, to, i));

        if let types::PresenceSource::Radio(idx) = &entry.source {
            anyhow::ensure!(
                *idx < num_radios,
                "Zone '{}' presence schedule[{i}]: radio index {idx} out of range (have {num_radios} stations)",
                zone.name,
            );
        }
    }
    if let Some(types::PresenceSource::Radio(idx)) = &presence.default_source {
        anyhow::ensure!(
            *idx < num_radios,
            "Zone '{}' presence default_source: radio index {idx} out of range (have {num_radios} stations)",
            zone.name,
        );
    }
    Ok(())
}

/// Apply environment variable overrides to the resolved configuration.
fn apply_env_overrides(config: &mut AppConfig) {
    override_http_port(config, std::env::var("SNAPDOG_HTTP_PORT").ok());
    if let Ok(val) = std::env::var("SNAPDOG_HTTP_API_KEYS") {
        config.http.api_keys = val
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(SecretString::from)
            .collect();
    }
    if let Some(ref mut subsonic) = config.subsonic
        && let Ok(val) = std::env::var("SNAPDOG_SUBSONIC_PASSWORD")
    {
        subsonic.password = SecretString::from(val);
    }
    if let Some(ref mut mqtt) = config.mqtt
        && let Ok(val) = std::env::var("SNAPDOG_MQTT_PASSWORD")
    {
        mqtt.password = SecretString::from(val);
    }
    override_psk(
        config,
        std::env::var("SNAPDOG_SNAPCAST_ENCRYPTION_PSK").ok(),
    );
}

fn override_http_port(config: &mut AppConfig, value: Option<String>) {
    let Some(val) = value else { return };
    match val.trim().parse() {
        Ok(port) => config.http.port = port,
        Err(_) => config.warn(format!(
            "SNAPDOG_HTTP_PORT={val:?} is not a port number and is ignored, using http.port {}",
            config.http.port
        )),
    }
}

/// An empty variable (for example `${PSK:-}` in a compose file) means "not set".
/// Treating it as a key would encrypt with an empty, publicly known secret.
fn override_psk(config: &mut AppConfig, value: Option<String>) {
    if let Some(val) = value.filter(|v| !v.is_empty()) {
        config.snapcast.encryption_psk = Some(SecretString::from(val));
    }
}

// ── KNX address validation helpers ──────────────────────────

fn validate_knx_ga(ga: &str) -> Result<()> {
    let parts: Vec<&str> = ga.split('/').collect();
    match parts.len() {
        2 => {
            let main: u8 = parts[0].parse().context("Invalid main group")?;
            let sub: u16 = parts[1].parse().context("Invalid sub group")?;
            anyhow::ensure!(main <= 31, "Main group must be 0-31");
            anyhow::ensure!(sub <= 2047, "Sub group (2-level) must be 0-2047");
        }
        3 => {
            let main: u8 = parts[0].parse().context("Invalid main group")?;
            let middle: u8 = parts[1].parse().context("Invalid middle group")?;
            let _sub: u8 = parts[2].parse().context("Invalid sub group")?;
            anyhow::ensure!(main <= 31, "Main group must be 0-31");
            anyhow::ensure!(middle <= 7, "Middle group must be 0-7");
        }
        _ => anyhow::bail!("Expected main/sub or main/middle/sub format"),
    }
    Ok(())
}

fn validate_knx_ia(ia: &str) -> Result<()> {
    let parts: Vec<&str> = ia.split('.').collect();
    anyhow::ensure!(parts.len() == 3, "Expected area.line.device format");
    let area: u8 = parts[0].parse().context("Invalid area")?;
    let line: u8 = parts[1].parse().context("Invalid line")?;
    let _device: u8 = parts[2].parse().context("Invalid device")?;
    anyhow::ensure!(area <= 15, "Area must be 0-15");
    anyhow::ensure!(line <= 15, "Line must be 0-15");
    Ok(())
}

fn validate_zone_knx(knx: &RawZoneKnxConfig) -> Result<()> {
    let gas = [
        &knx.play,
        &knx.pause,
        &knx.stop,
        &knx.track_next,
        &knx.track_previous,
        &knx.control_status,
        &knx.volume,
        &knx.volume_status,
        &knx.volume_dim,
        &knx.mute,
        &knx.mute_status,
        &knx.mute_toggle,
        &knx.track_title_status,
        &knx.track_artist_status,
        &knx.track_album_status,
        &knx.track_progress_status,
        &knx.track_playing_status,
        &knx.track_repeat,
        &knx.track_repeat_status,
        &knx.track_repeat_toggle,
        &knx.playlist,
        &knx.playlist_status,
        &knx.playlist_next,
        &knx.playlist_previous,
        &knx.shuffle,
        &knx.shuffle_status,
        &knx.shuffle_toggle,
        &knx.repeat,
        &knx.repeat_status,
        &knx.repeat_toggle,
        &knx.presence,
        &knx.presence_enable,
        &knx.presence_enable_status,
        &knx.presence_timer_status,
    ];
    for ga in gas.into_iter().flatten() {
        validate_knx_ga(ga).with_context(|| format!("Invalid Group Address: '{ga}'"))?;
    }
    Ok(())
}

fn validate_client_knx(knx: &RawClientKnxConfig) -> Result<()> {
    let gas = [
        &knx.volume,
        &knx.volume_status,
        &knx.volume_dim,
        &knx.mute,
        &knx.mute_status,
        &knx.mute_toggle,
        &knx.latency,
        &knx.latency_status,
        &knx.zone,
        &knx.zone_status,
        &knx.connected_status,
    ];
    for ga in gas.into_iter().flatten() {
        validate_knx_ga(ga).with_context(|| format!("Invalid Group Address: '{ga}'"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal_toml() -> &'static str {
        r#"
            [[zone]]
            name = "Ground Floor"

            [[client]]
            name = "Living Room"
            mac = "02:42:ac:11:00:10"
            zone = "Ground Floor"
        "#
    }

    #[test]
    fn parses_minimal_config() {
        let raw: FileConfig = toml::from_str(minimal_toml()).unwrap();
        let config = load_raw(raw).unwrap();
        assert_eq!(config.zones.len(), 1);
        assert_eq!(config.clients.len(), 1);
    }

    #[test]
    fn zone_conventions_applied() {
        let raw: FileConfig = toml::from_str(minimal_toml()).unwrap();
        let config = load_raw(raw).unwrap();
        let zone = &config.zones[0];
        assert_eq!(zone.index, 1);
        assert_eq!(zone.sink, "/snapsinks/zone1");
        assert_eq!(zone.stream_name, "Zone1");
        assert_eq!(zone.tcp_source_port, 4953);
    }

    #[test]
    fn knx_zone_no_defaults() {
        let raw: FileConfig = toml::from_str(minimal_toml()).unwrap();
        let config = load_raw(raw).unwrap();
        let knx = &config.zones[0].knx;
        // No convention defaults — all None unless explicitly configured
        assert!(knx.play.is_none());
        assert!(knx.volume.is_none());
        assert!(knx.mute.is_none());
    }

    #[test]
    fn knx_client_no_defaults() {
        let raw: FileConfig = toml::from_str(minimal_toml()).unwrap();
        let config = load_raw(raw).unwrap();
        let knx = &config.clients[0].knx;
        assert!(knx.volume.is_none());
        assert!(knx.mute.is_none());
    }

    #[test]
    fn client_zone_resolved_by_name() {
        let raw: FileConfig = toml::from_str(minimal_toml()).unwrap();
        let config = load_raw(raw).unwrap();
        assert_eq!(config.clients[0].zone_index, 1);
    }

    #[test]
    fn rejects_empty_zones() {
        let raw: FileConfig = toml::from_str(
            r#"
            [[client]]
            name = "X"
            mac = "00:00:00:00:00:00"
            zone = "X"
        "#,
        )
        .unwrap();
        assert!(load_raw(raw).is_err());
    }

    #[test]
    fn accept_policy_allows_no_clients() {
        // Fresh servers with unknown_clients = "accept" have no static clients
        // (they auto-register), so a zone-only config must be accepted.
        let raw: FileConfig = toml::from_str(
            r#"
            [snapcast]
            unknown_clients = "accept"

            [[zone]]
            name = "Ground Floor"
        "#,
        )
        .unwrap();
        assert!(load_raw(raw).is_ok());
    }

    #[test]
    fn reject_policy_still_requires_clients() {
        // With unknown_clients = "reject", only known clients work, so a static
        // [[client]] is still required.
        let raw: FileConfig = toml::from_str(
            r#"
            [snapcast]
            unknown_clients = "reject"

            [[zone]]
            name = "Ground Floor"
        "#,
        )
        .unwrap();
        assert!(load_raw(raw).is_err());
    }

    #[test]
    fn rejects_invalid_zone_reference() {
        let raw: FileConfig = toml::from_str(
            r#"
            [[zone]]
            name = "Ground Floor"

            [[client]]
            name = "X"
            mac = "00:00:00:00:00:00"
            zone = "Nonexistent"
        "#,
        )
        .unwrap();
        assert!(load_raw(raw).is_err());
    }

    #[test]
    fn zone_sink_override() {
        let raw: FileConfig = toml::from_str(
            r#"
            [[zone]]
            name = "Custom"
            sink = "/custom/path"

            [[client]]
            name = "X"
            mac = "00:00:00:00:00:00"
            zone = "Custom"
        "#,
        )
        .unwrap();
        let config = load_raw(raw).unwrap();
        assert_eq!(config.zones[0].sink, "/custom/path");
    }

    #[test]
    fn second_zone_gets_correct_indices() {
        let raw: FileConfig = toml::from_str(
            r#"
            [[zone]]
            name = "A"
            [[zone]]
            name = "B"

            [[client]]
            name = "X"
            mac = "00:00:00:00:00:00"
            zone = "B"
        "#,
        )
        .unwrap();
        let config = load_raw(raw).unwrap();
        assert_eq!(config.zones[1].index, 2);
        assert_eq!(config.zones[1].sink, "/snapsinks/zone2");
        assert_eq!(config.zones[1].tcp_source_port, 4954);
        assert_eq!(config.zones[1].knx.play, None);
        assert_eq!(config.clients[0].zone_index, 2);
    }

    #[test]
    fn knx_client_mode_requires_url() {
        let raw: FileConfig = toml::from_str(
            r#"
            [knx]
            enabled = true
            mode = "client"

            [[zone]]
            name = "A"
            [[client]]
            name = "X"
            mac = "00:00:00:00:00:00"
            zone = "A"
        "#,
        )
        .unwrap();
        assert!(load_raw(raw).unwrap_err().to_string().contains("url"));
    }

    #[test]
    fn knx_device_mode_requires_individual_address() {
        let raw: FileConfig = toml::from_str(
            r#"
            [knx]
            enabled = true
            mode = "device"

            [[zone]]
            name = "A"
            [[client]]
            name = "X"
            mac = "00:00:00:00:00:00"
            zone = "A"
        "#,
        )
        .unwrap();
        assert!(
            load_raw(raw)
                .unwrap_err()
                .to_string()
                .contains("individual_address")
        );
    }

    #[test]
    fn knx_rejects_unknown_mode() {
        let result: Result<FileConfig, _> = toml::from_str(
            r#"
            [knx]
            enabled = true
            mode = "bogus"

            [[zone]]
            name = "A"
            [[client]]
            name = "X"
            mac = "00:00:00:00:00:00"
            zone = "A"
        "#,
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("bogus"));
    }

    #[test]
    fn knx_disabled_skips_validation() {
        let raw: FileConfig = toml::from_str(
            r#"
            [[zone]]
            name = "A"
            [[client]]
            name = "X"
            mac = "00:00:00:00:00:00"
            zone = "A"
        "#,
        )
        .unwrap();
        assert!(load_raw(raw).is_ok());
    }

    #[test]
    fn presence_valid_schedule() {
        let raw: FileConfig = toml::from_str(
            r#"
            [[zone]]
            name = "A"
            [zone.presence]
            auto_off_delay = 900
            default_source = "radio:0"
            [[zone.presence.schedule]]
            from = "06:00"
            to = "09:00"
            source = "radio:0"
            [[zone.presence.schedule]]
            from = "09:00"
            to = "22:00"
            source = "playlist:abc123"

            [[radio]]
            name = "Test"
            url = "http://example.com/stream"

            [[client]]
            name = "X"
            mac = "00:00:00:00:00:00"
            zone = "A"
        "#,
        )
        .unwrap();
        assert!(load_raw(raw).is_ok());
    }

    #[test]
    fn presence_rejects_invalid_time() {
        let raw: FileConfig = toml::from_str(
            r#"
            [[zone]]
            name = "A"
            [zone.presence]
            [[zone.presence.schedule]]
            from = "25:00"
            to = "23:00"
            source = "none"
            [[client]]
            name = "X"
            mac = "00:00:00:00:00:00"
            zone = "A"
        "#,
        )
        .unwrap();
        let err = load_raw(raw).unwrap_err().to_string();
        assert!(
            err.contains("hour") || err.contains("25"),
            "expected hour error, got: {err}"
        );
    }

    #[test]
    fn presence_rejects_from_after_to() {
        let raw: FileConfig = toml::from_str(
            r#"
            [[zone]]
            name = "A"
            [zone.presence]
            [[zone.presence.schedule]]
            from = "18:00"
            to = "06:00"
            source = "none"
            [[client]]
            name = "X"
            mac = "00:00:00:00:00:00"
            zone = "A"
        "#,
        )
        .unwrap();
        assert!(
            load_raw(raw)
                .unwrap_err()
                .to_string()
                .contains("must be before")
        );
    }

    #[test]
    fn presence_rejects_overlapping_schedule() {
        let raw: FileConfig = toml::from_str(
            r#"
            [[zone]]
            name = "A"
            [zone.presence]
            [[zone.presence.schedule]]
            from = "06:00"
            to = "12:00"
            source = "none"
            [[zone.presence.schedule]]
            from = "10:00"
            to = "18:00"
            source = "none"
            [[client]]
            name = "X"
            mac = "00:00:00:00:00:00"
            zone = "A"
        "#,
        )
        .unwrap();
        assert!(load_raw(raw).unwrap_err().to_string().contains("overlaps"));
    }

    #[test]
    fn presence_rejects_invalid_radio_index() {
        let raw: FileConfig = toml::from_str(
            r#"
            [[zone]]
            name = "A"
            [zone.presence]
            default_source = "radio:5"
            [[client]]
            name = "X"
            mac = "00:00:00:00:00:00"
            zone = "A"
        "#,
        )
        .unwrap();
        assert!(
            load_raw(raw)
                .unwrap_err()
                .to_string()
                .contains("out of range")
        );
    }

    #[test]
    fn presence_rejects_invalid_source_format() {
        let result: Result<FileConfig, _> = toml::from_str(
            r#"
            [[zone]]
            name = "A"
            [zone.presence]
            default_source = "spotify:abc"
            [[client]]
            name = "X"
            mac = "00:00:00:00:00:00"
            zone = "A"
        "#,
        );
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("invalid presence source")
        );
    }

    #[test]
    fn rejects_duplicate_zone_names() {
        let raw: FileConfig = toml::from_str(
            r#"
            [[zone]]
            name = "A"
            [[zone]]
            name = "A"
            [[client]]
            name = "X"
            mac = "00:00:00:00:00:00"
            zone = "A"
        "#,
        )
        .unwrap();
        assert!(
            load_raw(raw)
                .unwrap_err()
                .to_string()
                .contains("Duplicate zone name")
        );
    }

    #[test]
    fn rejects_duplicate_client_macs() {
        let raw: FileConfig = toml::from_str(
            r#"
            [[zone]]
            name = "A"
            [[client]]
            name = "X"
            mac = "00:00:00:00:00:01"
            zone = "A"
            [[client]]
            name = "Y"
            mac = "00:00:00:00:00:01"
            zone = "A"
        "#,
        )
        .unwrap();
        assert!(
            load_raw(raw)
                .unwrap_err()
                .to_string()
                .contains("Duplicate client MAC address")
        );
    }

    #[test]
    fn rejects_invalid_mac_format() {
        let raw: FileConfig = toml::from_str(
            r#"
            [[zone]]
            name = "A"
            [[client]]
            name = "X"
            mac = "not-a-mac"
            zone = "A"
        "#,
        )
        .unwrap();
        assert!(
            load_raw(raw)
                .unwrap_err()
                .to_string()
                .contains("Invalid MAC address format")
        );
    }

    #[test]
    fn rejects_invalid_knx_ga() {
        let raw: FileConfig = toml::from_str(
            r#"
            [[zone]]
            name = "A"
            knx = { play = "32/0/0" }

            [[client]]
            name = "X"
            mac = "00:00:00:00:00:00"
            zone = "A"
        "#,
        )
        .unwrap();
        let err = format!("{:?}", load_raw(raw).unwrap_err());
        assert!(
            err.contains("Main group must be 0-31"),
            "Expected 'Main group must be 0-31' in error, got: {err}",
        );
    }

    #[test]
    fn accepts_valid_knx_ga_2level() {
        let raw: FileConfig = toml::from_str(
            r#"
            [[zone]]
            name = "A"
            [zone.knx]
            play = "1/2047"
            [[client]]
            name = "X"
            mac = "00:00:00:00:00:00"
            zone = "A"
        "#,
        )
        .unwrap();
        assert!(load_raw(raw).is_ok());
    }

    // ── `~` expansion, environment overrides, re-validation ──────

    fn home() -> Option<String> {
        std::env::var("HOME").ok().filter(|h| !h.is_empty())
    }

    #[test]
    fn tilde_expands_only_at_the_start() {
        let Some(home) = home() else { return };
        assert_eq!(expand_tilde("~"), home);
        assert_eq!(
            expand_tilde("~/.cache/snapdog/tracks"),
            format!("{}/.cache/snapdog/tracks", home.trim_end_matches('/'))
        );
        assert_eq!(expand_tilde("/var/~/x"), "/var/~/x");
        assert_eq!(expand_tilde("~other/x"), "~other/x");
        assert_eq!(expand_tilde("relative/path"), "relative/path");
    }

    #[test]
    fn tilde_is_expanded_in_every_user_written_path() {
        let Some(home) = home() else { return };
        let config = load_str(
            r#"
            [system]
            state_dir = "~/state"
            log_file = "~/snapdog.log"
            [http]
            tls_cert = "~/cert.pem"
            tls_key = "~/key.pem"
            [subsonic]
            url = "http://navidrome:4533"
            username = "u"
            password = "p"
            [subsonic.cache]
            path = "~/.cache/snapdog/tracks"
            [[zone]]
            name = "A"
        "#,
        )
        .unwrap();
        let prefix = home.trim_end_matches('/');
        assert_eq!(config.system.state_dir, format!("{prefix}/state"));
        assert_eq!(
            config.system.log_file.as_deref(),
            Some(format!("{prefix}/snapdog.log").as_str())
        );
        assert_eq!(
            config.http.tls_cert.as_deref().and_then(|p| p.to_str()),
            Some(format!("{prefix}/cert.pem").as_str())
        );
        assert_eq!(
            config.subsonic.unwrap().cache.path,
            format!("{prefix}/.cache/snapdog/tracks")
        );
    }

    #[test]
    fn unparsable_http_port_variable_is_reported() {
        let mut config = load_str("[[zone]]\nname = \"A\"").unwrap();
        let before = config.http.port;
        override_http_port(&mut config, Some("not-a-port".into()));
        assert_eq!(config.http.port, before);
        assert!(
            config
                .notices
                .iter()
                .any(|n| n.message.contains("SNAPDOG_HTTP_PORT"))
        );
        override_http_port(&mut config, Some(" 8080 ".into()));
        assert_eq!(config.http.port, 8080);
        override_http_port(&mut config, None);
        assert_eq!(config.http.port, 8080);
    }

    #[test]
    fn revalidation_catches_values_set_after_loading() {
        let mut config = load_str("[[zone]]\nname = \"A\"").unwrap();
        assert!(validate(&mut config).is_ok());
        config.audio.sample_rate = 12345;
        let error = validate(&mut config).unwrap_err().to_string();
        assert!(error.contains("Unsupported sample rate"), "{error}");

        let mut config = load_str("[[zone]]\nname = \"A\"").unwrap();
        config.http.port = config.snapcast.streaming_port;
        assert!(validate(&mut config).is_err());
    }

    #[test]
    fn revalidation_does_not_repeat_notices() {
        let mut config = load_str(
            "[audio]\nbit_depth = 32\n[snapcast]\ncodec = \"flac\"\n[[zone]]\nname = \"A\"",
        )
        .unwrap();
        let before = config.notices.clone();
        validate(&mut config).unwrap();
        assert_eq!(config.notices, before);
    }

    #[test]
    fn flac_clamp_is_reported_as_a_notice() {
        let config = load_str(
            "[audio]\nbit_depth = 32\n[snapcast]\ncodec = \"flac\"\n[[zone]]\nname = \"A\"",
        )
        .unwrap();
        assert_eq!(config.audio.bit_depth, 24);
        assert!(config.notices.iter().any(|n| n.message.contains("FLAC")));
    }

    #[test]
    fn unknown_keys_helper_lists_stale_keys() {
        let keys =
            unknown_keys("[snapcast]\nbogus = 1\n[[zone]]\nname = \"A\"\nnope = true").unwrap();
        assert_eq!(keys, ["snapcast.bogus", "zone[0].nope"]);
    }
}
