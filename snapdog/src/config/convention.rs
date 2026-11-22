// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! Convention-over-configuration: auto-generate KNX addresses, sink paths, stream names.

use anyhow::{Context, Result};

use super::types::{AudioConfig, ClientConfig, RawClientConfig, RawZoneConfig, ZoneConfig};

const TCP_SOURCE_BASE_PORT: u16 = 4952;

/// How `[spotify].name` and the number of zones name a zone's Spotify Connect device.
#[derive(Debug, Clone, Copy, Default)]
pub struct SpotifyNaming<'a> {
    /// `[spotify].name`, trimmed. `None` when the key is missing or blank.
    pub base: Option<&'a str>,
    /// Number of zones. With one zone the device is named exactly `base`.
    pub zone_count: usize,
}

impl<'a> SpotifyNaming<'a> {
    /// The naming rule for `zone_count` zones and the `[spotify]` section, if there is one.
    /// A blank name counts as none, and the name is trimmed.
    #[must_use]
    pub fn new(spotify: Option<&'a super::types::SpotifyConfig>, zone_count: usize) -> Self {
        Self {
            base: spotify
                .map(|s| s.name.trim())
                .filter(|name| !name.is_empty()),
            zone_count,
        }
    }
}

impl SpotifyNaming<'_> {
    /// Name of the Spotify device of the zone called `zone_name` when the zone sets no
    /// `spotify_name` of its own: `base` for a single zone, `"{base} {zone}"` for several
    /// (Spotify lists devices by name and every zone runs its own device, so the names
    /// have to differ), the zone name without a base.
    #[must_use]
    pub fn device_name(&self, zone_name: &str) -> String {
        match self.base {
            Some(base) if self.zone_count <= 1 => base.to_string(),
            Some(base) => format!("{base} {zone_name}"),
            None => zone_name.to_string(),
        }
    }
}

/// Resolve a raw zone config into a fully populated `ZoneConfig`.
pub fn resolve_zone(
    index: usize,
    raw: RawZoneConfig,
    audio: &AudioConfig,
    snapcast: &super::types::SnapcastConfig,
    spotify: SpotifyNaming<'_>,
) -> ZoneConfig {
    let n = index;
    let _ = audio; // Reserved for future sample format in stream name

    let airplay_name = raw.airplay_name.unwrap_or_else(|| raw.name.clone());
    let spotify_name = raw
        .spotify_name
        .unwrap_or_else(|| spotify.device_name(&raw.name));

    ZoneConfig {
        index,
        name: raw.name,
        icon: raw.icon,
        sink: raw.sink.unwrap_or_else(|| format!("/snapsinks/zone{n}")),
        stream_name: format!("Zone{n}"),
        tcp_source_port: TCP_SOURCE_BASE_PORT + n as u16,
        airplay_name,
        spotify_name,
        // Group addresses are used as written; validation has checked them.
        knx: raw.knx,
        group_volume_mode: raw.group_volume_mode.unwrap_or(snapcast.group_volume_mode),
        seek_interval_secs: raw
            .seek_interval_secs
            .unwrap_or(super::types::DEFAULT_SEEK_INTERVAL_SECS),
        presence: raw.presence,
        // Out-of-range values are clamped here; `validate` reports them.
        max_volume: raw.max_volume.clamp(0, 100),
        default_volume: raw
            .default_volume
            .map(|v| v.clamp(0, raw.max_volume.clamp(0, 100))),
        airplay: raw.airplay,
        spotify: raw.spotify,
    }
}

/// Resolve a raw client config into a fully populated `ClientConfig`.
pub fn resolve_client(
    index: usize,
    raw: RawClientConfig,
    zone_names: &[&str],
) -> Result<ClientConfig> {
    let zone_index = zone_names
        .iter()
        .position(|&name| name == raw.zone)
        .map(|i| i + 1)
        .with_context(|| {
            format!(
                "Client '{}' references unknown zone '{}'. Available: {:?}",
                raw.name, raw.zone, zone_names
            )
        })?;

    Ok(ClientConfig {
        index,
        name: raw.name,
        mac: raw.mac,
        zone_index,
        icon: raw.icon,
        max_volume: raw.max_volume.clamp(0, 100),
        default_volume: raw.default_volume.clamp(0, 100),
        default_latency: raw.default_latency,
        knx: raw.knx,
    })
}
