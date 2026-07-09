// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! REST API and WebSocket server via axum.

mod auth;
pub mod error;
mod health;
pub mod openapi;
mod routes;
mod webui;
pub mod ws;

/// Cache-Control header value for immutable assets cached for 1 day.
pub(crate) const CACHE_CONTROL_1DAY: &str = "public, max-age=86400";

/// Install the process-wide rustls crypto provider (aws-lc-rs, matching reqwest's
/// client default).
///
/// Both the `ring` and `aws-lc-rs` crypto providers are present in the dependency
/// tree (via axum-server/bollard and reqwest respectively). With more than one
/// provider compiled in, rustls cannot auto-select a default, so `ServerConfig::builder()`
/// — reached through `axum_server::tls_rustls::RustlsConfig::from_pem_file` in [`serve`]
/// when `http.tls_cert`/`tls_key` are set — panics at runtime unless a default has been
/// installed first. Call this once at startup, before any TLS is set up.
///
/// Idempotent: the underlying `install_default` returns an error if a provider was
/// already installed (e.g. a second call from a test), which is intentionally ignored.
pub fn install_crypto_provider() {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
}

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
    shutdown: impl std::future::Future<Output = ()> + Send + 'static,
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

    let app = build_router(&state);

    let local_addr = listener.local_addr()?;
    let port = local_addr.port();
    let tls_enabled = state.config.http.tls_cert.is_some();
    let scheme = if tls_enabled { "https" } else { "http" };

    if local_addr.ip().is_unspecified() {
        tracing::info!("REST API listening on port {port} (all interfaces, {scheme})");
        tracing::info!("  → {scheme}://localhost:{port}");
    } else {
        tracing::info!("REST API listening on {scheme}://{local_addr}");
    }

    if let (Some(cert_path), Some(key_path)) =
        (&state.config.http.tls_cert, &state.config.http.tls_key)
    {
        let rustls_config =
            axum_server::tls_rustls::RustlsConfig::from_pem_file(cert_path, key_path)
                .await
                .map_err(|e| anyhow::anyhow!("TLS configuration failed: {e}"))?;
        // TLS graceful shutdown via axum_server's Handle: when the cooperative
        // `shutdown` future resolves, stop accepting + drain in-flight, then
        // force-close after a grace period (parity with the plain-HTTP path).
        let handle = axum_server::Handle::new();
        let shutdown_handle = handle.clone();
        tokio::spawn(async move {
            shutdown.await;
            shutdown_handle.graceful_shutdown(Some(std::time::Duration::from_secs(10)));
        });
        axum_server::bind_rustls(local_addr, rustls_config)
            .handle(handle)
            .serve(app.into_make_service())
            .await?;
    } else {
        axum::serve(listener, app)
            .with_graceful_shutdown(shutdown)
            .await?;
    }
    Ok(())
}

/// Build the application `Router` from shared state, without binding a listener.
///
/// Extracted from [`serve`] (which calls this) so integration tests can drive the
/// full router in-process via `tower::ServiceExt::oneshot` — no TCP socket, no
/// background tasks. The route assembly here is the single source of truth for
/// both production serving and tests (`IT-T10`).
pub fn build_router(state: &SharedState) -> Router {
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
        // Layer order is load-bearing: the LAST `.layer` is the OUTERMOST, so the
        // Extension (which inserts ApiKeys into request extensions) must come AFTER
        // the auth middleware here — otherwise `require_api_key` runs first, sees no
        // ApiKeys extension, and falls through (auth.rs), silently bypassing auth.
        protected = protected
            .layer(axum::middleware::from_fn(auth::require_api_key))
            .layer(axum::Extension(auth::ApiKeys(api_keys)));
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
        .fallback(webui::fallback);

    #[cfg(feature = "api-docs")]
    let app = if state.config.http.api_docs {
        use utoipa::OpenApi;
        use utoipa_scalar::{Scalar, Servable};
        app.merge(Scalar::with_url("/docs", openapi::ApiDoc::openapi()))
    } else {
        app
    };

    app.layer(CompressionLayer::new())
        .layer(TraceLayer::new_for_http())
}

#[cfg(test)]
mod crypto_provider_tests {
    /// Regression: with both `ring` and `aws-lc-rs` in the tree, `ServerConfig::builder()`
    /// (used by the axum-server TLS path in `serve`) panics unless a default provider was
    /// installed. `install_crypto_provider` must make the builder succeed.
    #[test]
    fn builder_ok_after_provider_installed() {
        super::install_crypto_provider();
        // Panics with "Could not automatically determine the process-level CryptoProvider"
        // if no default is installed.
        let _ = rustls::ServerConfig::builder();
    }
}
