// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! `OpenAPI` documentation registry.

use utoipa::openapi::path::Operation;
use utoipa::openapi::response::ResponseBuilder;
use utoipa::openapi::schema::Schema;
use utoipa::openapi::security::{HttpAuthScheme, HttpBuilder, SecurityRequirement, SecurityScheme};
use utoipa::openapi::{ContentBuilder, RefOr};
use utoipa::{Modify, OpenApi};

use crate::api::error::ErrorBody;
use crate::api::routes::system::{self, SystemStatus, VersionInfo};

/// Registry of all endpoints and schemas exposed in the REST API.
#[derive(OpenApi)]
#[openapi(
    modifiers(&Contract),
    paths(
        system::get_status,
        system::get_version,
        system::get_listeners,
        system::update_status,
        system::check_updates,
        system::apply_update,
        system::configure_updates,

        // KNX
        crate::api::routes::knx::get_programming_mode,
        crate::api::routes::knx::set_programming_mode,
        crate::api::routes::knx::get_knxprod,
        crate::api::routes::knx::get_product_info,

        // Equalizer
        crate::api::routes::eq::get_eq,
        crate::api::routes::eq::set_eq,
        crate::api::routes::eq::set_band,
        crate::api::routes::eq::apply_preset,
        crate::api::routes::eq::list_presets,

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
            crate::net::listener::Row,
            crate::net::listener::Protocol,
            crate::net::listener::BindSupport,
            system::UpdateStatus,
            system::UpdateProgress,
            system::UpdateConfig,
            ErrorBody,

            // snapdog-common shared schemas
            snapdog_common::EqConfig,
            snapdog_common::EqBand,
            snapdog_common::FilterType,
            snapdog_common::TrackMetadata,
            snapdog_common::RepeatMode,
            snapdog_common::PlaybackControl,

            // Route-specific request/response schemas
            crate::api::routes::eq::EqPreset,
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

/// Corrections that the path attributes cannot express: the `Authorization: Bearer` scheme and
/// the 401 every protected operation can answer, and the 1-based lower bound of the index
/// parameters.
struct Contract;

impl Modify for Contract {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        let components = openapi.components.get_or_insert_with(Default::default);
        components.add_security_scheme(
            "bearer",
            SecurityScheme::Http(
                HttpBuilder::new()
                    .scheme(HttpAuthScheme::Bearer)
                    .description(Some(
                        "One of the configured `api_keys`. Not needed when none is configured.",
                    ))
                    .build(),
            ),
        );
        // The empty requirement makes the key optional, as it is when `api_keys` is empty.
        openapi.security = Some(vec![
            SecurityRequirement::default(),
            SecurityRequirement::new("bearer", Vec::<String>::new()),
        ]);

        for item in openapi.paths.paths.values_mut() {
            for op in [
                &mut item.get,
                &mut item.put,
                &mut item.post,
                &mut item.delete,
                &mut item.patch,
            ]
            .into_iter()
            .flatten()
            {
                document_operation(op);
            }
        }
    }
}

/// utoipa documents a bare `bool`, number or `String` body as `text/plain`. The handlers read
/// and write them with `Json<T>`, so on the wire they are JSON (`true`, `40`, `"Kitchen"`).
macro_rules! retype_json {
    ($content:expr, $remove:ident) => {
        if let Some(body) = $content.$remove("text/plain") {
            $content.entry("application/json".into()).or_insert(body);
        }
    };
}

fn document_operation(op: &mut Operation) {
    if let Some(RefOr::T(body)) = &mut op.request_body {
        retype_json!(body.content, remove);
    }
    for response in op.responses.responses.values_mut() {
        if let RefOr::T(response) = response {
            retype_json!(response.content, shift_remove);
        }
    }
    // The middleware answers with a plain-text body, not an `ErrorBody`.
    op.responses
        .responses
        .entry("401".into())
        .or_insert_with(|| {
            ResponseBuilder::new()
                .description("Missing or invalid API key (plain-text body)")
                .content("text/plain", ContentBuilder::new().build())
                .build()
                .into()
        });
    for param in op.parameters.iter_mut().flatten() {
        let RefOr::T(param) = param else { continue };
        if !matches!(param.name.as_str(), "zone_index" | "client_index") {
            continue;
        }
        if let Some(RefOr::T(Schema::Object(schema))) = &mut param.schema {
            // `usize` is documented as 0-based; these indexes start at 1.
            schema.minimum = Some(1.into());
        }
    }
}

/// `GET /api/v1/openapi.json`: the generated `OpenAPI` document. Mounted with the
/// protected routes in `api/mod.rs`, so it sits behind the API key like they do.
#[cfg(feature = "api-docs")]
pub(crate) async fn openapi_json() -> impl axum::response::IntoResponse {
    use std::sync::OnceLock;
    static SPEC: OnceLock<String> = OnceLock::new();
    // Serialising the generated document cannot fail: openapi_contract.rs does it too.
    let spec = SPEC.get_or_init(|| {
        ApiDoc::openapi()
            .to_json()
            .expect("the OpenAPI document serialises")
    });
    (
        [(axum::http::header::CONTENT_TYPE, "application/json")],
        spec.clone(),
    )
}
