// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! REST API and WebSocket server via axum.

mod auth;
pub mod error;
mod health;
mod routes;
mod webui;
pub mod ws;

/// Cache-Control header value for immutable assets cached for 1 day.
pub(crate) const CACHE_CONTROL_1DAY: &str = "public, max-age=86400";

use std::collections::HashMap;
use std::hash::BuildHasher;
use std::sync::Arc;

use anyhow::Result;
use axum::Router;
use tokio::net::TcpListener;
use tower_http::compression::CompressionLayer;
use tower_http::trace::TraceLayer;

use crate::config::AppConfig;
use crate::player;
use crate::player::ZoneCommandSender;
use crate::state;

/// Shared application state accessible from all handlers.
pub struct AppState {
    /// Resolved application configuration.
    pub config: AppConfig,
    /// In-memory zone/client state store.
    pub store: state::SharedState,
    /// Command senders keyed by zone index (1-based).
    pub zone_commands: HashMap<usize, ZoneCommandSender>,
    /// Sender for Snapcast JSON-RPC commands.
    pub snap_tx: player::SnapcastCmdSender,
    /// Content-addressed cover art cache.
    pub covers: state::cover::SharedCoverCache,
    /// Broadcast sender for WebSocket notifications.
    pub notifications: ws::NotifySender,
    /// Shared parametric EQ store.
    pub eq_store: std::sync::Arc<std::sync::Mutex<crate::audio::eq::EqStore>>,
    /// KNX device control (programming mode). `None` in client mode.
    pub knx_device_control: Option<crate::knx::DeviceControlHandle>,
    /// Cached Subsonic playlist list with expiry timestamp.
    pub playlist_cache:
        tokio::sync::RwLock<Option<(std::time::Instant, Vec<crate::subsonic::PlaylistEntry>)>>,
    /// Spinorama speaker profile database.
    pub speaker_db: crate::spinorama::SpeakerDb,
}

/// Thread-safe shared reference to [`AppState`].
pub type SharedState = Arc<AppState>;

/// Start the HTTP server.
///
/// # Errors
///
/// Returns an error if the server fails. The caller is responsible for
/// pre-binding the `listener` before starting any subsystems so that a
/// port conflict is detected early and can be handled as a fatal error.
#[expect(clippy::too_many_arguments)]
pub async fn serve<S: BuildHasher>(
    listener: TcpListener,
    config: AppConfig,
    store: state::SharedState,
    zone_commands: HashMap<usize, ZoneCommandSender, S>,
    snap_tx: player::SnapcastCmdSender,
    covers: state::cover::SharedCoverCache,
    notifications: ws::NotifySender,
    eq_store: std::sync::Arc<std::sync::Mutex<crate::audio::eq::EqStore>>,
    knx_device_control: Option<crate::knx::DeviceControlHandle>,
) -> Result<()> {
    let state = Arc::new(AppState {
        config,
        store,
        zone_commands: zone_commands.into_iter().collect(),
        snap_tx,
        covers,
        notifications,
        eq_store,
        knx_device_control,
        playlist_cache: tokio::sync::RwLock::new(None),
        speaker_db: crate::spinorama::SpeakerDb::new(),
    });

    let api_keys: Vec<String> = state
        .config
        .http
        .api_keys
        .iter()
        .map(|k| k.as_str().to_string())
        .collect();

    // Protected routes (API + WebSocket)
    let mut protected = Router::new()
        .merge(ws::router(state.clone()))
        .nest(
            "/api/v1/zones",
            routes::zones::router(state.clone()).merge(routes::eq::router(state.clone())),
        )
        .nest(
            "/api/v1/clients",
            routes::clients::router(state.clone())
                .merge(routes::client_eq::router(state.clone()))
                .merge(routes::speakers::client_speaker_router(state.clone())),
        )
        .nest("/api/v1/media", routes::media::router(state.clone()))
        .nest("/api/v1/system", routes::system::router(state.clone()))
        .nest("/api/v1/knx", routes::knx::router(state.clone()))
        .nest(
            "/api/v1/speakers",
            routes::speakers::speakers_router(state.clone()),
        );

    if !api_keys.is_empty() {
        tracing::info!(keys = api_keys.len(), "API authentication enabled");
        protected = protected
            .layer(axum::Extension(auth::ApiKeys(api_keys)))
            .layer(axum::middleware::from_fn(auth::require_api_key));
    } else if state
        .config
        .http
        .bind
        .parse::<std::net::IpAddr>()
        .is_ok_and(|ip| ip.is_unspecified())
    {
        tracing::warn!(
            bind = %state.config.http.bind,
            "API authentication is disabled while listening on all interfaces"
        );
    }

    let app = Router::new()
        .merge(health::router(state.clone()))
        .merge(protected)
        .fallback(webui::fallback)
        .layer(CompressionLayer::new())
        .layer(TraceLayer::new_for_http());

    let local_addr = listener.local_addr()?;
    let port = local_addr.port();
    if local_addr.ip().is_unspecified() {
        tracing::info!("REST API listening on port {port} (all interfaces)");
        tracing::info!("  → http://localhost:{port}");
    } else {
        tracing::info!("REST API listening on http://{local_addr}");
    }
    axum::serve(listener, app).await?;
    Ok(())
}
