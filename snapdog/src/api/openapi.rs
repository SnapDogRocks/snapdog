// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! `OpenAPI` documentation registry.

use utoipa::OpenApi;

use crate::api::error::ErrorBody;
use crate::api::routes::system::{self, SystemStatus, VersionInfo};

/// Registry of all endpoints and schemas exposed in the REST API.
#[derive(OpenApi)]
#[openapi(
    paths(
        system::get_status,
        system::get_version,

        // KNX
        crate::api::routes::knx::get_programming_mode,
        crate::api::routes::knx::set_programming_mode,

        // Equalizer
        crate::api::routes::eq::get_eq,
        crate::api::routes::eq::set_eq,
        crate::api::routes::eq::set_band,
        crate::api::routes::eq::apply_preset,

        // Client Equalizer
        crate::api::routes::client_eq::get_eq,
        crate::api::routes::client_eq::set_eq,
        crate::api::routes::client_eq::set_band,
        crate::api::routes::client_eq::apply_preset,

        // Speakers
        crate::api::routes::speakers::list,
        crate::api::routes::speakers::get_profile,
        crate::api::routes::speakers::get_speaker,
        crate::api::routes::speakers::apply_speaker,

        // Clients
        crate::api::routes::clients::get_count,
        crate::api::routes::clients::get_all,
        crate::api::routes::clients::get_client,
        crate::api::routes::clients::get_volume,
        crate::api::routes::clients::set_volume,
        crate::api::routes::clients::get_mute,
        crate::api::routes::clients::set_mute,
        crate::api::routes::clients::toggle_mute,
        crate::api::routes::clients::get_latency,
        crate::api::routes::clients::set_latency,
        crate::api::routes::clients::get_zone,
        crate::api::routes::clients::set_zone,
        crate::api::routes::clients::get_name,
        crate::api::routes::clients::set_name,
        crate::api::routes::clients::get_icon,
        crate::api::routes::clients::get_connected,

        // Media
        crate::api::routes::media::get_playlists,
        crate::api::routes::media::get_playlist,
        crate::api::routes::media::get_playlist_cover,
        crate::api::routes::media::get_playlist_tracks,
        crate::api::routes::media::get_playlist_track,
        crate::api::routes::media::get_track_cover_art,

        // Zones
        crate::api::routes::zones::get_count,
        crate::api::routes::zones::get_all,
        crate::api::routes::zones::get_zone,
        crate::api::routes::zones::get_volume,
        crate::api::routes::zones::set_volume,
        crate::api::routes::zones::get_mute,
        crate::api::routes::zones::set_mute,
        crate::api::routes::zones::toggle_mute,
        crate::api::routes::zones::play,
        crate::api::routes::zones::pause,
        crate::api::routes::zones::stop,
        crate::api::routes::zones::next_track,
        crate::api::routes::zones::previous_track,
        crate::api::routes::zones::get_playlist,
        crate::api::routes::zones::set_playlist,
        crate::api::routes::zones::next_playlist,
        crate::api::routes::zones::previous_playlist,
        crate::api::routes::zones::get_shuffle,
        crate::api::routes::zones::set_shuffle,
        crate::api::routes::zones::toggle_shuffle,
        crate::api::routes::zones::get_repeat,
        crate::api::routes::zones::set_repeat,
        crate::api::routes::zones::toggle_repeat,
        crate::api::routes::zones::get_track,
        crate::api::routes::zones::get_track_metadata,
        crate::api::routes::zones::get_zone_cover,
        crate::api::routes::zones::get_track_title,
        crate::api::routes::zones::get_track_artist,
        crate::api::routes::zones::get_track_album,
        crate::api::routes::zones::get_track_duration,
        crate::api::routes::zones::get_track_position,
        crate::api::routes::zones::seek_position,
        crate::api::routes::zones::get_track_progress,
        crate::api::routes::zones::seek_progress,
        crate::api::routes::zones::get_track_playing,
        crate::api::routes::zones::play_track,
        crate::api::routes::zones::play_url,
        crate::api::routes::zones::play_subsonic_playlist,
        crate::api::routes::zones::play_playlist_track,
        crate::api::routes::zones::play_subsonic_track,
        crate::api::routes::zones::get_name,
        crate::api::routes::zones::get_icon,
        crate::api::routes::zones::get_playback,
        crate::api::routes::zones::get_playlist_name,
        crate::api::routes::zones::get_playlist_info,
        crate::api::routes::zones::get_playlist_count,
        crate::api::routes::zones::get_clients,
        crate::api::routes::zones::get_presence,
        crate::api::routes::zones::set_presence,
        crate::api::routes::zones::get_presence_enabled,
        crate::api::routes::zones::set_presence_enabled,
        crate::api::routes::zones::get_presence_timeout,
        crate::api::routes::zones::set_presence_timeout,
        crate::api::routes::zones::get_presence_timer,
    ),
    components(
        schemas(
            SystemStatus,
            VersionInfo,
            ErrorBody,

            // snapdog-common shared schemas
            snapdog_common::EqConfig,
            snapdog_common::EqBand,
            snapdog_common::FilterType,
            snapdog_common::TrackMetadata,
            snapdog_common::RepeatMode,
            snapdog_common::PlaybackControl,

            // Route-specific request/response schemas
            crate::api::routes::speakers::ApplySpeakerRequest,
            crate::api::routes::clients::ClientInfo,
            crate::api::routes::zones::VolumeValue,
            crate::api::routes::zones::ZoneInfo,
            crate::api::routes::zones::SeekPayload,
            crate::api::routes::zones::PlaylistPayload,
            crate::api::routes::zones::ZoneTrackMetadata,
            crate::api::routes::zones::ZonePlaylistInfo,
            crate::api::routes::media::PlaylistInfo,
            crate::api::routes::media::PlaylistDetails,
            crate::api::routes::media::MediaTrackInfo,
        )
    ),
    tags(
        (name = "system", description = "System and platform administration endpoints"),
        (name = "zones", description = "Multi-zone control, playback, and presence management"),
        (name = "clients", description = "Individual speaker volume, mute, latency, and zone assignment"),
        (name = "equalizer", description = "Parametric EQ curves, bands, and presets for zones and clients"),
        (name = "speakers", description = "Speaker profiles database and correction applications"),
        (name = "knx", description = "KNX home automation integration status and control"),
        (name = "media", description = "Radio station lists, Subsonic playlists, track listings, and covers")
    ),
    info(
        title = "SnapDog REST API",
        version = "1.0.0",
        description = "SnapDog Multi-zone synchronized audio controller API"
    )
)]
pub struct ApiDoc;
