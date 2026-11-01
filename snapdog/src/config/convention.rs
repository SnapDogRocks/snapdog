// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! Convention-over-configuration: auto-generate KNX addresses, sink paths, stream names.

use anyhow::{Context, Result};

use super::types::{
    AudioConfig, ClientConfig, ClientKnxAddresses, RawClientConfig, RawClientKnxConfig,
    RawZoneConfig, RawZoneKnxConfig, ZoneConfig, ZoneKnxAddresses,
};

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
        knx: resolve_zone_knx(n, raw.knx),
        group_volume_mode: raw.group_volume_mode.unwrap_or(snapcast.group_volume_mode),
        presence: raw.presence,
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
        knx: resolve_client_knx(index, raw.knx),
    })
}

/// Zone KNX addresses — explicit only, no convention defaults.
/// Only configured GAs are active. Unconfigured GAs are ignored.
fn resolve_zone_knx(_n: usize, raw: RawZoneKnxConfig) -> ZoneKnxAddresses {
    ZoneKnxAddresses {
        play: raw.play,
        pause: raw.pause,
        stop: raw.stop,
        volume: raw.volume,
        volume_status: raw.volume_status,
        volume_dim: raw.volume_dim,
        mute: raw.mute,
        mute_status: raw.mute_status,
        mute_toggle: raw.mute_toggle,
        track_next: raw.track_next,
        track_previous: raw.track_previous,
        control_status: raw.control_status,
        track_title_status: raw.track_title_status,
        track_artist_status: raw.track_artist_status,
        track_album_status: raw.track_album_status,
        track_progress_status: raw.track_progress_status,
        track_playing_status: raw.track_playing_status,
        track_repeat: raw.track_repeat,
        track_repeat_status: raw.track_repeat_status,
        track_repeat_toggle: raw.track_repeat_toggle,
        playlist: raw.playlist,
        playlist_status: raw.playlist_status,
        playlist_next: raw.playlist_next,
        playlist_previous: raw.playlist_previous,
        playlist_step: raw.playlist_step,
        playlist_name_status: raw.playlist_name_status,
        track_number: raw.track_number,
        track_number_status: raw.track_number_status,
        play_pause: raw.play_pause,
        track_step: raw.track_step,
        shuffle: raw.shuffle,
        shuffle_status: raw.shuffle_status,
        shuffle_toggle: raw.shuffle_toggle,
        repeat: raw.repeat,
        repeat_status: raw.repeat_status,
        repeat_toggle: raw.repeat_toggle,
        presence: raw.presence,
        presence_enable: raw.presence_enable,
        presence_enable_status: raw.presence_enable_status,
        presence_timer_status: raw.presence_timer_status,
    }
}

/// Client KNX addresses — explicit only, no convention defaults.
fn resolve_client_knx(_n: usize, raw: RawClientKnxConfig) -> ClientKnxAddresses {
    ClientKnxAddresses {
        volume: raw.volume,
        volume_status: raw.volume_status,
        volume_dim: raw.volume_dim,
        mute: raw.mute,
        mute_status: raw.mute_status,
        mute_toggle: raw.mute_toggle,
        latency: raw.latency,
        latency_status: raw.latency_status,
        zone: raw.zone,
        zone_status: raw.zone_status,
        connected_status: raw.connected_status,
    }
}
