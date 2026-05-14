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
pub fn load(path: &Path) -> Result<AppConfig> {
    let content = std::fs::read_to_string(path) /* blocking OK: called once at startup */
        .with_context(|| format!("Failed to read {}", path.display()))?;
    let raw: FileConfig =
        toml::from_str(&content).with_context(|| format!("Failed to parse {}", path.display()))?;
    load_raw(raw)
}

/// Resolve raw TOML config into fully populated AppConfig with conventions applied.
pub fn load_raw(raw: FileConfig) -> Result<AppConfig> {
    anyhow::ensure!(
        !raw.zone.is_empty(),
        "At least one [[zone]] must be configured"
    );
    anyhow::ensure!(
        !raw.client.is_empty(),
        "At least one [[client]] must be configured"
    );

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

    // ── Presence validation ───────────────────────────────────
    for zone in &zones {
        if let Some(ref presence) = zone.presence {
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
                        *idx < radios.len(),
                        "Zone '{}' presence schedule[{i}]: radio index {idx} out of range (have {} stations)",
                        zone.name,
                        radios.len()
                    );
                }
            }
            if let Some(types::PresenceSource::Radio(idx)) = &presence.default_source {
                anyhow::ensure!(
                    *idx < radios.len(),
                    "Zone '{}' presence default_source: radio index {idx} out of range (have {} stations)",
                    zone.name,
                    radios.len()
                );
            }
        }
    }

    Ok(AppConfig {
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
    })
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
}
