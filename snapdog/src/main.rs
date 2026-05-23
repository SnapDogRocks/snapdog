// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

// Pedantic lints allowed crate-wide: same rationale as lib.rs — audio/protocol code
// uses intentional numeric casts, long functions are unavoidable in the main event loop.
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_sign_loss)]
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::float_cmp)]
#![allow(clippy::future_not_send)]
#![allow(clippy::struct_excessive_bools)]
#![allow(clippy::implicit_hasher)]

#[cfg(all(feature = "snapcast-embedded", feature = "snapcast-process"))]
compile_error!(
    "Features `snapcast-embedded` and `snapcast-process` are mutually exclusive. \
     Use `--no-default-features --features snapcast-process` for process mode."
);

#[cfg(not(any(feature = "snapcast-embedded", feature = "snapcast-process")))]
compile_error!("Either `snapcast-embedded` or `snapcast-process` feature must be enabled.");

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use clap::Parser;
use tracing_subscriber::EnvFilter;

#[cfg(feature = "snapcast-process")]
use snapdog::process;
use snapdog::{api, audio, config, knx, mqtt, player, snapcast, state};

/// Multi-zone audio controller with `AirPlay`, Snapcast, MQTT, and KNX integration.
#[derive(Parser)]
#[command(version, about)]
struct Cli {
    /// Path to configuration file (TOML)
    #[arg(short, long)]
    config: Option<PathBuf>,

    /// KNX device mode — ETS programs everything, no TOML required
    #[arg(long)]
    knx_device: bool,

    /// KNX individual address (device mode). Example: 1.1.100
    #[arg(long, requires = "knx_device")]
    knx_address: Option<String>,

    /// Start with KNX programming mode active (requires --knx-device)
    #[arg(long, requires = "knx_device")]
    knx_prog_mode: bool,

    /// HTTP API port
    #[arg(short, long)]
    port: Option<u16>,

    /// Bind address (default: 0.0.0.0). Use :: for dual-stack IPv4+IPv6.
    #[arg(short, long)]
    bind: Option<String>,

    /// Audio codec: pcm, flac, f32lz4, f32lz4e
    #[arg(long)]
    codec: Option<String>,

    /// Sample rate
    #[arg(long)]
    sample_rate: Option<u32>,

    /// Bit depth (16, 24, 32)
    #[arg(long)]
    bit_depth: Option<u16>,

    /// Snapcast streaming port
    #[arg(long)]
    streaming_port: Option<u16>,

    /// Server display name (`mDNS`, MQTT, KNX). Default: "`SnapDog`"
    #[arg(long)]
    name: Option<String>,

    /// Log level: trace, debug, info, warn, error
    #[arg(short, long)]
    log_level: Option<String>,

    /// Run as a Windows service (Windows only)
    #[cfg(target_os = "windows")]
    #[arg(long)]
    service: bool,
}

/// Volume coalescing window — rapid volume changes within this window are merged.
const VOLUME_COALESCE_MS: u64 = 50;
/// Channel capacity for Snapcast commands from zone players, API, MQTT, KNX.
const SNAPCAST_CMD_CHANNEL_SIZE: usize = 64;
/// Delay after starting snapserver before connecting.
#[cfg(feature = "snapcast-process")]
const SNAPSERVER_STARTUP_DELAY: std::time::Duration = std::time::Duration::from_secs(1);
/// State file names (relative to `config.system.state_dir`).
const STATE_FILE: &str = "zones.json";
const EQ_FILE: &str = "eq.json";

/// Coalesces rapid volume changes per client within a time window.
struct VolumeCoalescer {
    pending: std::collections::HashMap<String, (i32, tokio::time::Instant)>,
    window: std::time::Duration,
}

impl VolumeCoalescer {
    fn new(window: std::time::Duration) -> Self {
        Self {
            pending: std::collections::HashMap::new(),
            window,
        }
    }

    fn push(&mut self, client_id: String, volume: i32) {
        self.pending.insert(
            client_id,
            (volume, tokio::time::Instant::now() + self.window),
        );
    }

    fn flush_expired(&mut self) -> Vec<(String, i32)> {
        let now = tokio::time::Instant::now();
        let expired: Vec<_> = self
            .pending
            .iter()
            .filter(|(_, (_, deadline))| now >= *deadline)
            .map(|(id, (vol, _))| (id.clone(), *vol))
            .collect();
        for (id, _) in &expired {
            self.pending.remove(id);
        }
        expired
    }

    fn next_deadline(&self) -> Option<tokio::time::Instant> {
        self.pending.values().map(|(_, d)| *d).min()
    }
}

#[cfg(target_os = "windows")]
mod service_windows;

#[tokio::main]
async fn main() -> Result<()> {
    // ── Windows service mode ──────────────────────────────────
    #[cfg(target_os = "windows")]
    {
        // Pre-parse just to check --service flag (before full init)
        let cli = Cli::parse();
        if cli.service {
            service_windows::run_as_service()
                .map_err(|e| anyhow::anyhow!("Windows service error: {e}"))?;
            return Ok(());
        }
    }

    run_app().await
}

/// Core application logic — used by both console mode and Windows service.
///
/// # Errors
///
/// Returns an error if the application fails to start or encounters a fatal error.
///
/// # Panics
///
/// Panics if the OS signal handler cannot be registered (Unix only).
#[allow(clippy::too_many_lines)] // Main app setup + event loop — splitting would obscure control flow
pub async fn run_app() -> Result<()> {
    // ── Parse config ──────────────────────────────────────────
    let cli = Cli::parse();

    if cli.config.is_none() && !cli.knx_device {
        // Neither --config nor --knx-device specified — show help and exit
        use clap::CommandFactory;
        Cli::command().print_help()?;
        eprintln!("\n\nerror: specify --config <PATH> or --knx-device");
        std::process::exit(1);
    }

    let mut app_config = if let Some(ref config_path) = cli.config {
        config::load(config_path)?
    } else {
        // --knx-device without --config: start with defaults, ETS provides config
        config::load_raw_no_validate(config::FileConfig::default())?
    };

    // --knx-device enables device mode
    if cli.knx_device {
        let knx = app_config.knx.get_or_insert_default();
        knx.role = config::KnxRole::Device;
        knx.individual_address = Some(cli.knx_address.unwrap_or_else(|| "15.15.255".into()));
    }

    // --knx-prog-mode: set flag for BAU task to activate on start
    if cli.knx_prog_mode {
        if let Some(ref mut knx) = app_config.knx {
            knx.start_prog_mode = true;
        }
    }

    // CLI overrides
    if let Some(port) = cli.port {
        app_config.http.port = port;
    }
    if let Some(ref bind) = cli.bind {
        app_config.http.bind = bind.clone();
    }
    if let Some(ref codec) = cli.codec {
        app_config.snapcast.codec = match codec.as_str() {
            "pcm" => config::AudioCodec::Pcm,
            "flac" => config::AudioCodec::Flac,
            "f32lz4" => config::AudioCodec::F32lz4,
            "f32lz4e" => config::AudioCodec::F32lz4e,
            other => anyhow::bail!("Unknown codec '{other}' — use pcm, flac, f32lz4, or f32lz4e"),
        };
    }
    if let Some(rate) = cli.sample_rate {
        app_config.audio.sample_rate = rate;
    }
    if let Some(bits) = cli.bit_depth {
        app_config.audio.bit_depth = bits;
    }
    if let Some(port) = cli.streaming_port {
        app_config.snapcast.streaming_port = port;
    }
    if let Some(ref s) = cli.name {
        app_config.name = s.clone();
    }
    if let Some(ref level) = cli.log_level {
        app_config.system.log_level = match level.as_str() {
            "trace" => config::LogLevel::Trace,
            "debug" => config::LogLevel::Debug,
            "info" => config::LogLevel::Info,
            "warn" => config::LogLevel::Warn,
            "error" => config::LogLevel::Error,
            other => anyhow::bail!(
                "Unknown log level '{other}' — use trace, debug, info, warn, or error"
            ),
        };
    }

    let config_label = cli
        .config
        .as_deref()
        .map_or_else(|| "defaults".into(), |p| p.display().to_string());

    let config = Arc::new(app_config);

    // ── Initialize logging ────────────────────────────────────
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::from_default_env()
                .add_directive(format!("snapdog={}", config.system.log_level).parse()?),
        )
        .init();

    tracing::info!(
        zones = config.zones.len(),
        clients = config.clients.len(),
        radios = config.radios.len(),
        "Configuration loaded from {}",
        config_label
    );

    // ── Pre-bind ports ────────────────────────────────────────
    // Bind early — before starting any subsystems — so a port conflict
    // (e.g. a stale snapdog instance) fails fast with an actionable error
    // instead of crashing mid-startup via astro-dnssd or silently degrading.

    // HTTP API listener — passed into api::serve() so no TOCTOU window.
    let http_addr = format!("{}:{}", config.http.bind, config.http.port);
    let http_listener = tokio::net::TcpListener::bind(&http_addr)
        .await
        .with_context(|| {
            format!(
                "HTTP API port {} is already in use — is snapdog already running?",
                config.http.port
            )
        })?;

    // Snapcast streaming port — probe-bind then immediately release so the
    // embedded server can bind it a moment later.  Catches conflicts early.
    #[cfg(feature = "snapcast-embedded")]
    {
        let snap_addr = format!(
            "{}:{}",
            config.snapcast.address, config.snapcast.streaming_port
        );
        tokio::net::TcpListener::bind(&snap_addr)
            .await
            .with_context(|| {
                format!(
                    "Snapcast streaming port {} is already in use — is snapdog already running?",
                    config.snapcast.streaming_port
                )
            })?;
        // listener dropped here — embedded server will bind it below
    }

    // ── Initialize subsystems ─────────────────────────────────
    let state_dir = PathBuf::from(&config.system.state_dir);
    std::fs::create_dir_all(&state_dir)
        .with_context(|| format!("Failed to create state directory: {}", state_dir.display()))?;
    tracing::info!(path = %state_dir.display(), "State directory");

    let store = state::init(&config, Some(&state_dir.join(STATE_FILE)))?;
    state::spawn_auto_save(store.clone());
    let covers = state::cover::new_cache();
    let (notify_tx, _) = api::ws::notification_channel();

    // ── Snapcast backend ──────────────────────────────────────
    #[cfg(feature = "snapcast-process")]
    let mut snapserver = process::SnapserverHandle::start(&config)?;
    #[cfg(feature = "snapcast-process")]
    if config.snapcast.managed {
        tokio::time::sleep(SNAPSERVER_STARTUP_DELAY).await;
    }

    #[cfg(feature = "snapcast-process")]
    let snap = snapcast::SnapcastClient::from_config(&config).await?;
    #[cfg(feature = "snapcast-process")]
    let status = snap.server_get_status().await?;
    #[cfg(feature = "snapcast-process")]
    snapcast::sync_initial_state(&status, &config, &snap, &store).await;
    #[cfg(all(feature = "snapcast-process", not(feature = "snapcast-embedded")))]
    let mut snap_notifications = snap.subscribe();

    // EQ store (needed by event handler + zone players + API)
    let eq_store = Arc::new(std::sync::Mutex::new(audio::eq::EqStore::load(
        &state_dir.join(EQ_FILE),
    )));

    #[cfg(feature = "snapcast-embedded")]
    let (embedded_backend, embedded_events) =
        snapcast::embedded::EmbeddedBackend::start(&config, store.clone())?;
    #[cfg(feature = "snapcast-embedded")]
    let backend: Arc<dyn snapcast::backend::SnapcastBackend> = Arc::new(embedded_backend);
    #[cfg(feature = "snapcast-embedded")]
    snapcast::events::spawn_event_handler(
        embedded_events,
        config.clone(),
        backend.clone(),
        store.clone(),
        notify_tx.clone(),
        eq_store.clone(),
    );

    #[cfg(all(feature = "snapcast-process", not(feature = "snapcast-embedded")))]
    let process_backend =
        Arc::new(snapcast::process::ProcessBackend::start(&config, snap, store.clone()).await?);
    #[cfg(all(feature = "snapcast-process", not(feature = "snapcast-embedded")))]
    let backend: Arc<dyn snapcast::backend::SnapcastBackend> = process_backend.clone();

    // Snapcast command channel (used by zone players, API, MQTT, KNX)
    let (snap_cmd_tx, mut snap_cmd_rx) =
        tokio::sync::mpsc::channel::<player::SnapcastCmd>(SNAPCAST_CMD_CHANNEL_SIZE);

    // ── Zone players ──────────────────────────────────────────
    let zone_commands = player::spawn_zone_players(player::ZonePlayerContext {
        config: config.clone(),
        store: store.clone(),
        covers: covers.clone(),
        notify: notify_tx.clone(),
        snap_tx: snap_cmd_tx.clone(),
        backend: backend.clone(),
        eq_store: eq_store.clone(),
        #[cfg(feature = "snapcast-process")]
        client_mac_map: snapcast::build_client_mac_map(&status),
        #[cfg(not(feature = "snapcast-process"))]
        client_mac_map: std::collections::HashMap::new(),
        #[cfg(feature = "snapcast-process")]
        group_ids: snapcast::build_group_ids(&status),
        #[cfg(not(feature = "snapcast-process"))]
        group_ids: Vec::new(),
        #[cfg(feature = "snapcast-process")]
        group_clients: snapcast::build_group_clients(&status),
        #[cfg(not(feature = "snapcast-process"))]
        group_clients: std::collections::HashMap::new(),
    })
    .await?;

    // ── KNX bridge ────────────────────────────────────────────
    let mut knx_device_control: Option<knx::DeviceControlHandle> = None;
    if config.knx.is_some() {
        let knx_notifications = notify_tx.subscribe();
        match knx::start(
            &config,
            store.clone(),
            knx_notifications,
            zone_commands.clone(),
            snap_cmd_tx.clone(),
        )
        .await
        {
            Ok(handle) => {
                knx_device_control = handle;
            }
            Err(e) => tracing::warn!(error = %e, "KNX connection failed — running without KNX"),
        }
    }

    // ── API server ────────────────────────────────────────────
    // Spawned with a fatal-error channel: if the API task exits unexpectedly
    // (not via the pre-bound listener failing — that's already caught above),
    // the main loop receives the error and shuts down gracefully.
    let http_config = (*config).clone();
    let api_store = store.clone();
    let api_commands = zone_commands.clone();
    let api_covers = covers.clone();
    let api_notify = notify_tx.clone();
    let api_snap_tx = snap_cmd_tx.clone();
    let (api_fatal_tx, api_fatal_rx) = tokio::sync::oneshot::channel::<anyhow::Error>();
    tokio::spawn(async move {
        if let Err(e) = api::serve(
            http_listener,
            http_config,
            api_store,
            api_commands,
            api_snap_tx,
            api_covers,
            api_notify,
            eq_store.clone(),
            knx_device_control,
        )
        .await
        {
            // Only fires if axum::serve itself fails after startup (very rare).
            let _ = api_fatal_tx.send(e);
        }
    });

    // ── MQTT bridge ───────────────────────────────────────────
    let mut mqtt_bridge = if let Some(mqtt_config) = &config.mqtt {
        match mqtt::MqttBridge::connect(mqtt_config, &config.http.base_url, &config.name).await {
            Ok(bridge) => {
                if let Err(e) = bridge.subscribe_commands().await {
                    tracing::warn!(error = %e, "MQTT subscribe failed");
                }
                // Publish HA MQTT Discovery
                if let Err(e) = bridge.publish_ha_discovery(&config.zones).await {
                    tracing::warn!(error = %e, "HA MQTT Discovery publish failed");
                }
                // Publish initial state for all zones and clients
                let (zones, clients) = {
                    let s = store.read().await;
                    let z: Vec<_> = s.zones.iter().map(|(&i, z)| (i, z.clone())).collect();
                    let c: Vec<_> = s.clients.iter().map(|(&i, c)| (i, c.clone())).collect();
                    drop(s);
                    (z, c)
                };
                for (idx, zone) in &zones {
                    if let Err(e) = bridge.publish_zone_state(*idx, zone).await {
                        tracing::warn!(zone = idx, error = %e, "MQTT initial zone publish failed");
                    }
                }
                for (idx, client) in &clients {
                    if let Err(e) = bridge.publish_client_state(*idx, client).await {
                        tracing::warn!(client = idx, error = %e, "MQTT initial client publish failed");
                    }
                }
                Some(bridge)
            }
            Err(e) => {
                tracing::warn!(error = %e, "MQTT connection failed — running without MQTT");
                None
            }
        }
    } else {
        None
    };

    // ── mDNS advertisement ──────────────────────────────────────
    let _mdns = if config.mdns.enabled {
        use std::collections::HashMap;
        let mut txt = HashMap::new();
        txt.insert("api_version".into(), "1".into());
        txt.insert(
            "snapcast_port".into(),
            config.snapcast.streaming_port.to_string(),
        );
        txt.insert("auth".into(), (!config.http.api_keys.is_empty()).to_string());
        if is_docker() {
            txt.insert("docker".into(), "true".into());
        }
        if config.http.base_url != format!("http://localhost:{}", config.http.port) {
            txt.insert("base_url".into(), config.http.base_url.clone());
        }
        let svc = astro_dnssd::DNSServiceBuilder::new("_snapdog._tcp", config.http.port)
            .with_name(&config.name)
            .with_txt_record(txt.clone())
            .register();
        match svc {
            Ok(s) => {
                tracing::info!(
                    port = config.http.port,
                    name = %config.name,
                    ?txt,
                    "mDNS: advertising _snapdog._tcp"
                );
                let snapcast_svc = if config.mdns.advertise_snapcast {
                    astro_dnssd::DNSServiceBuilder::new(
                        "_snapcast._tcp",
                        config.snapcast.streaming_port,
                    )
                    .with_name(&config.name)
                    .register()
                    .map_err(|e| {
                        tracing::warn!(error = %e, "mDNS: _snapcast._tcp advertisement failed");
                    })
                    .ok()
                } else {
                    None
                };
                Some((s, snapcast_svc))
            }
            Err(e) => {
                tracing::warn!(error = %e, "mDNS advertisement failed");
                None
            }
        }
    } else {
        None
    };


    // ── Main loop ─────────────────────────────────────────────
    let mqtt_zone_cmds = zone_commands.clone();
    let mqtt_store = store.clone();
    let mqtt_snap_tx = snap_cmd_tx.clone();
    let mqtt_notify_store = store.clone();
    let mut mqtt_notifications = notify_tx.subscribe();
    let cmd_backend = backend.clone();

    // Volume coalescing: buffer rapid volume changes per client
    let mut coalescer = VolumeCoalescer::new(std::time::Duration::from_millis(VOLUME_COALESCE_MS));

    // Fuse the API fatal receiver so it can be safely polled inside the loop
    // without moving out of a non-Copy type on every iteration.
    let mut api_fatal_rx = Some(api_fatal_rx);

    loop {
        let sleep = async {
            match coalescer.next_deadline() {
                Some(d) => tokio::time::sleep_until(d).await,
                None => std::future::pending().await,
            }
        };

        tokio::select! {
            // Snapcast commands from zone players / API / MQTT / KNX
            Some(cmd) = snap_cmd_rx.recv() => {
                // Convert relative volume adjustments to absolute
                let cmd = if let player::SnapcastCmd::Client { ref client_id, action: player::ClientAction::AdjustVolume(delta) } = cmd {
                    let current = store.read().await.clients.values()
                        .find(|c| c.snapcast_id.as_deref() == Some(client_id))
                        .map_or(state::DEFAULT_VOLUME, |c| c.base_volume);
                    player::SnapcastCmd::Client {
                        client_id: client_id.clone(),
                        action: player::ClientAction::Volume((current + delta).clamp(0, 100)),
                    }
                } else {
                    cmd
                };
                // Coalesce client volume commands
                if let player::SnapcastCmd::Client { ref client_id, action: player::ClientAction::Volume(v) } = cmd {
                    coalescer.push(client_id.clone(), v);
                } else if let Err(e) = cmd_backend.execute(cmd).await {
                    tracing::warn!(error = %e, "Snapcast command failed");
                }
            }
            // Coalesce timer fired — flush expired volumes
            () = sleep => {
                for (client_id, volume) in coalescer.flush_expired() {
                    let cmd = player::SnapcastCmd::Client {
                        client_id,
                        action: player::ClientAction::Volume(volume),
                    };
                    if let Err(e) = cmd_backend.execute(cmd).await {
                        tracing::warn!(error = %e, "Snapcast volume command failed");
                    }
                }
            }
            // Snapcast server notifications → state updates (process backend only)
            () = async {
                #[cfg(all(feature = "snapcast-process", not(feature = "snapcast-embedded")))]
                if let Ok(notification) = snap_notifications.recv().await {
                    snapcast::handle_notification(notification, &config, process_backend.client(), &store, &notify_tx).await;
                }
                #[cfg(not(all(feature = "snapcast-process", not(feature = "snapcast-embedded"))))]
                std::future::pending::<()>().await;
            } => {}
            // MQTT: poll commands + republish state on notifications
            () = async {
                if let Some(ref mut bridge) = mqtt_bridge {
                    tokio::select! {
                        () = bridge.poll_once(&mqtt_zone_cmds, &mqtt_store, &mqtt_snap_tx) => {}
                        Ok(json) = mqtt_notifications.recv() => {
                            if let Ok(notif) = serde_json::from_str::<api::ws::Notification>(&json) {
                                let s = mqtt_notify_store.read().await;
                                match notif {
                                    api::ws::Notification::ZoneStateChanged { zone, .. }
                                    | api::ws::Notification::ZoneTrackChanged { zone, .. }
                                    | api::ws::Notification::ZoneProgress { zone, .. } => {
                                        if let Some(z) = s.zones.get(&zone) {
                                            let _ = bridge.publish_zone_state(zone, z).await;
                                        }
                                    }
                                    api::ws::Notification::ClientStateChanged { client, .. } => {
                                        if let Some(c) = s.clients.get(&client) {
                                            let _ = bridge.publish_client_state(client, c).await;
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                } else {
                    std::future::pending::<()>().await;
                }
            } => {}
            // API fatal error (post-startup axum failure — extremely rare)
            Some(e) = async {
                if let Some(rx) = api_fatal_rx.as_mut() {
                    if let Ok(e) = rx.await {
                        api_fatal_rx = None; // consume so the arm never fires again
                        return Some(e);
                    }
                    api_fatal_rx = None; // sender dropped without sending
                }
                std::future::pending::<Option<anyhow::Error>>().await
            } => {
                tracing::error!(error = %e, "API server failed — shutting down");
                break;
            }
            // Shutdown signals
            () = async {
                let sigint = tokio::signal::ctrl_c();
                #[cfg(unix)]
                let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).unwrap();

                #[cfg(unix)]
                tokio::select! {
                    _ = sigint => { tracing::info!("Received Ctrl-C, shutting down gracefully..."); }
                    _ = sigterm.recv() => { tracing::info!("Received SIGTERM, shutting down gracefully..."); }
                };
                #[cfg(not(unix))]
                let _ = sigint.await;
            } => {
                break;
            }
        }
    }

    // ── Shutdown ──────────────────────────────────────────────

    // Spawn a dedicated thread for the forced-exit safety net.
    // This survives the tokio runtime shutdown and handles second signals.
    std::thread::spawn(|| {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        rt.block_on(async {
            let sigint = tokio::signal::ctrl_c();
            #[cfg(unix)]
            let mut sigterm =
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).unwrap();

            #[cfg(unix)]
            tokio::select! {
                _ = sigint => {
                    tracing::warn!("Second signal received, forcing exit");
                    std::process::exit(1);
                }
                _ = sigterm.recv() => {
                    tracing::warn!("Second signal received, forcing exit");
                    std::process::exit(1);
                }
                () = tokio::time::sleep(std::time::Duration::from_secs(3)) => {
                    tracing::warn!("Graceful shutdown timed out, forcing exit");
                    std::process::exit(0);
                }
            };
            #[cfg(not(unix))]
            tokio::select! {
                _ = sigint => {
                    tracing::warn!("Second signal received, forcing exit");
                    std::process::exit(1);
                }
                _ = tokio::time::sleep(std::time::Duration::from_secs(3)) => {
                    tracing::warn!("Graceful shutdown timed out, forcing exit");
                    std::process::exit(0);
                }
            };
        });
    });

    let _ = backend.stop().await;
    let persist_result = store.write().await.persist();
    if let Err(e) = persist_result {
        tracing::warn!(error = %e, "Failed to persist state");
    }
    #[cfg(feature = "snapcast-process")]
    snapserver.stop().await?;

    tracing::info!("SnapDog server terminated");
    std::process::exit(0);
}

/// Detect if running inside a Docker container.
fn is_docker() -> bool {
    std::path::Path::new("/.dockerenv").exists()
        || std::fs::read_to_string("/proc/1/cgroup")
            .is_ok_and(|s| s.contains("docker") || s.contains("containerd"))
}
