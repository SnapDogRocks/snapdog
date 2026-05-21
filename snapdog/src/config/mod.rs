// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! Configuration loading and validation.
//!
//! Single TOML file → all derived config (KNX addresses, sink paths, snapserver.conf).
//! Convention over configuration: sensible defaults, auto-generated where possible.

mod convention;
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
    let raw: FileConfig =
        toml::from_str(&content).with_context(|| format!("Failed to parse {}", path.display()))?;
    load_raw(raw)
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

fn load_raw_inner(raw: FileConfig, skip_zone_validation: bool) -> Result<AppConfig> {
    if !skip_zone_validation {
        anyhow::ensure!(
            !raw.zone.is_empty(),
            "At least one [[zone]] must be configured"
        );
        anyhow::ensure!(
            !raw.client.is_empty(),
            "At least one [[client]] must be configured"
        );
    }

    // ── Uniqueness & format validation ─────────────────────────
    validate_raw_inputs(&raw)?;

    // ── KNX address validation ───────────────────────────────
    if let Some(ref knx) = raw.knx {
        if let Some(ref addr) = knx.individual_address {
            validate_knx_ia(addr).context("Invalid KNX individual address")?;
        }
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

    let name = if raw.name != "SnapDog" {
        // Explicit top-level name takes priority
        raw.name
    } else if raw.snapcast.mdns_name != "SnapDog" {
        // Deprecated fallback: [snapcast].mdns_name
        tracing::warn!("`[snapcast].mdns_name` is deprecated — use top-level `name` instead");
        raw.snapcast.mdns_name.clone()
    } else {
        raw.name
    };

    let mut config = AppConfig {
        name,
        system: raw.system,
        audio: raw.audio,
        http: raw.http,
        snapcast: raw.snapcast,
        airplay: raw.airplay,
        subsonic: raw.subsonic,
        spotify: raw.spotify,
        mqtt: raw.mqtt,
        knx: raw.knx,
        zones,
        clients,
        radios,
    };

    apply_env_overrides(&mut config);

    Ok(config)
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

    anyhow::ensure!(
        matches!(
            raw.audio.sample_rate,
            44100 | 48000 | 88200 | 96000 | 176_400 | 192_000
        ),
        "Unsupported sample rate: {}. Use 44100, 48000, 88200, 96000, 176400, or 192000",
        raw.audio.sample_rate
    );
    anyhow::ensure!(
        matches!(raw.audio.bit_depth, 16 | 24 | 32),
        "Unsupported bit depth: {}. Use 16, 24, or 32",
        raw.audio.bit_depth
    );
    anyhow::ensure!(
        raw.audio.channels > 0 && raw.audio.channels <= 8,
        "Unsupported number of channels: {}. Use 1 to 8",
        raw.audio.channels
    );
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
    if let Ok(val) = std::env::var("SNAPDOG_HTTP_PORT") {
        if let Ok(port) = val.parse() {
            config.http.port = port;
        }
    }
    if let Ok(val) = std::env::var("SNAPDOG_HTTP_API_KEYS") {
        config.http.api_keys = val
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(SecretString::from)
            .collect();
    }
    if let Some(ref mut subsonic) = config.subsonic {
        if let Ok(val) = std::env::var("SNAPDOG_SUBSONIC_PASSWORD") {
            subsonic.password = SecretString::from(val);
        }
    }
    if let Some(ref mut mqtt) = config.mqtt {
        if let Ok(val) = std::env::var("SNAPDOG_MQTT_PASSWORD") {
            mqtt.password = SecretString::from(val);
        }
    }
    if let Ok(val) = std::env::var("SNAPDOG_SNAPCAST_ENCRYPTION_PSK") {
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
        &knx.presence_timeout,
        &knx.presence_timeout_status,
        &knx.presence_timer_status,
        &knx.presence_source_override,
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
}
