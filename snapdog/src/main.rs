// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

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

#[cfg(all(feature = "dbus", target_os = "linux"))]
use snapdog::dbus;
#[cfg(feature = "snapcast-process")]
use snapdog::process;
use snapdog::{api, audio, config, knx, logging, mqtt, net, player, snapcast, state, subsonic};

/// Multi-zone audio controller with `AirPlay`, Snapcast, MQTT, and KNX integration.
#[derive(Parser)]
#[command(version, about)]
struct Cli {
    /// Path to configuration file (TOML)
    #[arg(short, long)]
    config: Option<PathBuf>,

    /// `WebUI` served at `/`: current or legacy (overrides [http].webui)
    #[arg(long, value_enum)]
    webui: Option<WebUiChoice>,

    /// Validate the configuration and exit without starting any services
    #[arg(long, requires = "config")]
    check_config: bool,

    /// KNX device mode — ETS programs everything, no TOML required
    #[arg(long)]
    knx_device: bool,

    /// KNX individual address (device mode). Example: 1.1.100
    #[arg(long, requires = "knx_device")]
    knx_address: Option<String>,

    /// Start with KNX programming mode active (requires --knx-device)
    #[arg(long, requires = "knx_device")]
    knx_prog_mode: bool,

    /// Set the OS system clock from the KNX Time group object (best effort; needs privilege)
    #[arg(long)]
    knx_sync_system_clock: bool,

    /// HTTP API port
    #[arg(short, long)]
    port: Option<u16>,

    /// HTTP bind address, `loopback` or interface name like `eth0`, as `[http].bind`
    /// (default: ::, dual-stack IPv4+IPv6). Use 0.0.0.0 for IPv4 only.
    /// Several entries, separated by commas, listen on each of them.
    #[arg(short, long, value_delimiter = ',')]
    bind: Vec<String>,

    /// TLS certificate path (PEM fullchain). Enables HTTPS.
    #[arg(long)]
    tls_cert: Option<std::path::PathBuf>,

    /// TLS private key path (PEM, unencrypted).
    #[arg(long)]
    tls_key: Option<std::path::PathBuf>,

    /// Snapcast stream codec, overrides [snapcast].codec: pcm, flac, f32lz4, f32lz4e
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

    /// Server display name (`mDNS` instance name, Home Assistant device name,
    /// system version endpoint). Default: "`SnapDog`"
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

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
enum WebUiChoice {
    Current,
    Legacy,
}

const fn selected_webui(configured: config::WebUi, cli: Option<WebUiChoice>) -> config::WebUi {
    match cli {
        Some(WebUiChoice::Current) => config::WebUi::Current,
        Some(WebUiChoice::Legacy) => config::WebUi::Legacy,
        None => configured,
    }
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
    /// Volumes sent recently, with the time they went out. Snapcast reports them back a moment
    /// later, and a relative step in between must build on them, not on the old state.
    recent: std::collections::HashMap<String, (i32, tokio::time::Instant)>,
    window: std::time::Duration,
}

/// How long a sent volume counts as the current one for relative steps.
const RECENT_VOLUME_MS: u64 = 1000;

impl VolumeCoalescer {
    fn new(window: std::time::Duration) -> Self {
        Self {
            pending: std::collections::HashMap::new(),
            recent: std::collections::HashMap::new(),
            window,
        }
    }

    /// The volume a client is about to get or got a moment ago, if any.
    fn latest(&self, client_id: &str) -> Option<i32> {
        if let Some((volume, _)) = self.pending.get(client_id) {
            return Some(*volume);
        }
        self.recent
            .get(client_id)
            .filter(|(_, sent)| sent.elapsed() < std::time::Duration::from_millis(RECENT_VOLUME_MS))
            .map(|(volume, _)| *volume)
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
        for (id, volume) in &expired {
            self.pending.remove(id);
            self.recent.insert(id.clone(), (*volume, now));
        }
        expired
    }

    fn next_deadline(&self) -> Option<tokio::time::Instant> {
        self.pending.values().map(|(_, d)| *d).min()
    }
}

/// A zone reconcile in process mode, which the main loop carries out itself.
const fn is_process_reconcile(cmd: &player::SnapcastCmd) -> bool {
    cfg!(all(
        feature = "snapcast-process",
        not(feature = "snapcast-embedded")
    )) && matches!(cmd, player::SnapcastCmd::ReconcileZones)
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
    // Install the rustls crypto provider before any TLS is set up: both `ring` and
    // `aws-lc-rs` are in the tree, so rustls can't auto-pick one and the axum-server
    // TLS path would otherwise panic when `http.tls_cert`/`tls_key` are configured.
    api::install_crypto_provider();

    // ── Parse config ──────────────────────────────────────────
    let cli = Cli::parse();

    if cli.config.is_none() && !cli.knx_device {
        // Neither --config nor --knx-device specified — show help and exit
        use clap::CommandFactory;
        Cli::command().print_help()?;
        eprintln!("\n\nerror: specify --config <PATH> or --knx-device");
        std::process::exit(1);
    }

    let codec = cli
        .codec
        .as_deref()
        .map(|codec| match codec {
            "pcm" => Ok(config::AudioCodec::Pcm),
            "flac" => Ok(config::AudioCodec::Flac),
            "f32lz4" => Ok(config::AudioCodec::F32lz4),
            "f32lz4e" => Ok(config::AudioCodec::F32lz4e),
            other => Err(anyhow::anyhow!(
                "Unknown codec '{other}' — use pcm, flac, f32lz4, or f32lz4e"
            )),
        })
        .transpose()?;
    let log_level = cli
        .log_level
        .as_deref()
        .map(|level| match level {
            "trace" => Ok(config::LogLevel::Trace),
            "debug" => Ok(config::LogLevel::Debug),
            "info" => Ok(config::LogLevel::Info),
            "warn" => Ok(config::LogLevel::Warn),
            "error" => Ok(config::LogLevel::Error),
            other => Err(anyhow::anyhow!(
                "Unknown log level '{other}' — use trace, debug, info, warn, or error"
            )),
        })
        .transpose()?;
    // The command line is applied to the raw configuration before validation and bind
    // resolution, so a flag behaves like the same key in the file.
    let overrides = config::Overrides {
        knx_device: cli.knx_device.then(|| {
            cli.knx_address
                .clone()
                .unwrap_or_else(|| config::DEFAULT_KNX_DEVICE_ADDRESS.into())
        }),
        knx_prog_mode: cli.knx_prog_mode,
        knx_sync_system_clock: cli.knx_sync_system_clock,
        webui: cli
            .webui
            .map(|choice| selected_webui(config::WebUi::default(), Some(choice))),
        port: cli.port,
        bind: cli.bind.clone(),
        tls_cert: cli.tls_cert.clone(),
        tls_key: cli.tls_key.clone(),
        codec,
        sample_rate: cli.sample_rate,
        bit_depth: cli.bit_depth,
        streaming_port: cli.streaming_port,
        name: cli.name.clone(),
        log_level,
    };
    let mut app_config = config::load_with_overrides(cli.config.as_deref(), &overrides)?;

    // Validate TLS configuration
    match (&app_config.http.tls_cert, &app_config.http.tls_key) {
        (Some(_), None) => anyhow::bail!("tls_cert is set but tls_key is missing"),
        (None, Some(_)) => anyhow::bail!("tls_key is set but tls_cert is missing"),
        _ => {}
    }

    if cli.check_config {
        // A wrong [mqtt] section does not stop the server (MQTT is switched off), but
        // it must not pass a configuration check either.
        if let Some(problem) = &app_config.mqtt_problem {
            anyhow::bail!("Invalid [mqtt] section, the server would start without MQTT: {problem}");
        }
        if let Some(bind) = app_config.snapcast.streaming_bind
            && config::address_is_not_local(bind)
        {
            app_config.warn(format!(
                "[snapcast].streaming_bind = {bind} is not an address of this host (or not one \
                 that can be bound), so the server would not start here. Check the address, or \
                 ignore this if the configuration is for another machine."
            ));
        }
        let planned = net::listener::planned(&app_config);
        net::listener::check_unique(&planned)?;
        for listener in net::listener::unassigned(&planned) {
            if listener.service != "Snapcast streaming" {
                app_config.warn(format!(
                    "{listener} is not an address of this host (or not one that can be bound), \
                     so the server would not start here. Check the bind setting of that \
                     service, or ignore this if the configuration is for another machine."
                ));
            }
        }
        for notice in &app_config.notices {
            let label = match notice.level {
                config::NoticeLevel::Info => "note",
                config::NoticeLevel::Warn => "warning",
            };
            eprintln!("{label}: {}", notice.message);
        }
        if let Some(mqtt) = &app_config.mqtt {
            for warning in &mqtt.warnings {
                eprintln!("warning: {warning}");
            }
        }
        println!(
            "Interfaces of this host:\n{}",
            config::interfaces::InterfaceTable::system().inventory()
        );
        println!(
            "Listeners:{}",
            net::listener::format_table(&net::listener::table(&app_config))
        );
        println!("Configuration is valid");
        return Ok(());
    }

    let config_label = cli
        .config
        .as_deref()
        .map_or_else(|| "defaults".into(), |p| p.display().to_string());

    let config = Arc::new(app_config);

    // ── Initialize logging ────────────────────────────────────
    // `log_guard` must live until shutdown: dropping it stops file logging.
    let logging::LogSetup {
        subscriber,
        file_guard: log_guard,
        file_error: log_file_error,
    } = logging::build(
        &config.system.log_level.to_string(),
        config.system.log_file.as_deref(),
    )?;
    logging::install(subscriber)?;
    if let (Some(error), Some(path)) = (log_file_error, config.system.log_file.as_deref()) {
        tracing::warn!(
            path,
            "log_file is not used, logging to stdout only: {error}"
        );
    }
    config.log_notices();

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
    // instead of crashing mid-startup or silently degrading.

    // Nothing is bound yet: two services on one port, or an address that no interface
    // has, are found here and not as the second bind that fails.
    let planned = net::listener::planned(&config);
    net::listener::check_unique(&planned)?;
    if let Some(listener) = net::listener::unassigned(&planned).first() {
        anyhow::bail!(
            "{listener} cannot be bound: {} is not an address of this host. Check the bind \
             setting of that service.",
            listener.addr.ip()
        );
    }

    // HTTP API listeners — passed into api::serve_on() so no TOCTOU window.
    let http_listeners = api::bind_http_listeners(&config.http.bind_addresses(), config.http.port)
        .await
        .map_err(|(bind, e)| match bind.parse::<std::net::IpAddr>() {
            Ok(ip) => anyhow::anyhow!(net::listener::bind_failure_message(
                "HTTP API",
                std::net::SocketAddr::new(ip, config.http.port),
                "[http].bind",
                &e
            )),
            Err(_) => anyhow::Error::new(e).context(format!(
                "cannot bind the HTTP API to {bind}:{}",
                config.http.port
            )),
        })?;

    // Snapcast streaming port — probe-bind then immediately release so the
    // embedded server can bind it a moment later.  Catches conflicts early.
    #[cfg(feature = "snapcast-embedded")]
    snapcast::embedded::check_streaming_port(&config.snapcast)?;

    // ── Initialize subsystems ─────────────────────────────────
    let state_dir = PathBuf::from(&config.system.state_dir);
    std::fs::create_dir_all(&state_dir)
        .with_context(|| format!("Failed to create state directory: {}", state_dir.display()))?;
    tracing::info!(path = %state_dir.display(), "State directory");

    // Stable server identity — persisted across restarts
    let server_identity = state::server_identity(&state_dir);
    let server_id = server_identity.id.clone();

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
    #[cfg(all(feature = "snapcast-process", not(feature = "snapcast-embedded")))]
    let process_backend =
        Arc::new(snapcast::process::ProcessBackend::start(&config, snap, store.clone()).await?);
    #[cfg(all(feature = "snapcast-process", not(feature = "snapcast-embedded")))]
    let backend: Arc<dyn snapcast::backend::SnapcastBackend> = process_backend.clone();

    // Snapcast command channel (used by zone players, API, MQTT, KNX)
    let (snap_cmd_tx, mut snap_cmd_rx) =
        tokio::sync::mpsc::channel::<player::SnapcastCmd>(SNAPCAST_CMD_CHANNEL_SIZE);

    // ── Zone players ──────────────────────────────────────────
    let playlist_cache = std::sync::Arc::new(subsonic::PlaylistCache::new());
    let zone_commands = player::spawn_zone_players(player::ZonePlayerContext {
        config: config.clone(),
        store: store.clone(),
        covers: covers.clone(),
        notify: notify_tx.clone(),
        snap_tx: snap_cmd_tx.clone(),
        backend: backend.clone(),
        eq_store: eq_store.clone(),
        playlist_cache: playlist_cache.clone(),
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
        start_receivers: true,
        #[cfg(feature = "test-harness")]
        test_pcm_rx: std::sync::Mutex::new(std::collections::HashMap::new()),
        #[cfg(feature = "test-harness")]
        test_airplay_rx: std::sync::Mutex::new(std::collections::HashMap::new()),
    })
    .await?;

    // ── Snapcast event handler ────────────────────────────────
    #[cfg(feature = "snapcast-embedded")]
    snapcast::events::spawn_event_handler(
        embedded_events,
        config.clone(),
        backend.clone(),
        store.clone(),
        notify_tx.clone(),
        eq_store.clone(),
        zone_commands.clone(),
    );

    // ── KNX bridge ────────────────────────────────────────────
    let mut knx_device_control: Option<knx::DeviceControlHandle> = None;
    let mut knx_bridge: Option<knx::KnxBridge> = None;
    if config.knx.is_some() {
        match knx::start(
            &config,
            store.clone(),
            notify_tx.clone(),
            zone_commands.clone(),
            snap_cmd_tx.clone(),
        )
        .await
        {
            Ok(bridge) => {
                knx_device_control = bridge.device_control();
                knx_bridge = Some(bridge);
            }
            Err(e) => {
                tracing::warn!(error = %format_args!("{e:#}"), "KNX could not be started — running without KNX");
            }
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
        if let Err(e) = api::serve_on(
            http_listeners,
            http_config,
            api_store,
            api_commands,
            api_snap_tx,
            api_covers,
            api_notify,
            eq_store.clone(),
            knx_device_control,
            playlist_cache,
            // Production never triggers cooperative shutdown (the process exits via
            // the main loop's signal handler); a never-resolving future preserves
            // the prior serve-until-exit behavior. The seam exists for tests (IT-T84).
            std::future::pending::<()>(),
        )
        .await
        {
            // Only fires if axum::serve itself fails after startup (very rare).
            let _ = api_fatal_tx.send(e);
        }
    });

    tracing::info!(
        "Listeners{}",
        net::listener::format_table(&net::listener::table(&config))
    );

    // ── MQTT bridge ───────────────────────────────────────────
    // Runs in its own tasks: connects (and reconnects), subscribes, announces and
    // publishes state. A configuration error disables MQTT, it never falls back
    // to a different transport.
    let mqtt_bridge = if let Some(mqtt_config) = &config.mqtt {
        for warning in &mqtt_config.warnings {
            tracing::warn!("{warning}");
        }
        match mqtt::MqttBridge::start(
            mqtt_config,
            &config.http.base_url,
            mqtt::MqttRuntime {
                zone_commands: zone_commands.clone(),
                state: store.clone(),
                snap_tx: snap_cmd_tx.clone(),
                notifications: notify_tx.subscribe(),
                notify: notify_tx.clone(),
                server: mqtt::ServerInfo {
                    id: server_id.clone(),
                    stable_id: server_identity.persisted,
                    name: config.name.clone(),
                    base_url: config.http.base_url.clone(),
                },
            },
        )
        .await
        {
            Ok(bridge) => Some(bridge),
            Err(e) => {
                tracing::error!(error = %format!("{e:#}"), "MQTT cannot start, running without MQTT");
                None
            }
        }
    } else {
        if let Some(problem) = &config.mqtt_problem {
            tracing::error!(
                problem = %problem,
                "MQTT is switched off: the [mqtt] section is invalid. The server keeps running \
                 without MQTT; fix the section and restart (snapdog --check-config reports it)"
            );
        }
        None
    };

    // ── mDNS advertisement ──────────────────────────────────────
    let _mdns = if config.mdns.enabled {
        use std::collections::HashMap;
        let mut txt = HashMap::new();
        txt.insert("api_version".into(), "1".into());
        txt.insert("server_id".into(), server_id.clone());
        txt.insert(
            "snapcast_port".into(),
            config.snapcast.streaming_port.to_string(),
        );
        txt.insert(
            "auth".into(),
            (!config.http.api_keys.is_empty()).to_string(),
        );
        if is_docker() {
            txt.insert("docker".into(), "true".into());
        }
        let tls_enabled = config.http.tls_cert.is_some();
        if tls_enabled {
            txt.insert("tls".into(), "true".into());
        }
        // Determine the effective base_url for mDNS advertisement:
        // - If explicitly configured (not the localhost default): use it
        // - If bind address is not wildcard: derive from bind address
        // - If TLS is enabled with wildcard bind: advertise https scheme
        let scheme = if tls_enabled { "https" } else { "http" };
        let default_base = format!("http://localhost:{}", config.http.port);
        let effective_base_url = if config.http.base_url != default_base {
            Some(config.http.base_url.clone())
        } else if let Some(host) = config.http.single_specific_bind() {
            Some(format!("{scheme}://{host}:{}", config.http.port))
        } else if tls_enabled {
            Some(format!("https://localhost:{}", config.http.port))
        } else {
            None
        };
        if let Some(ref url) = effective_base_url {
            txt.insert("base_url".into(), url.clone());
        }
        let snapcast_port = config
            .mdns
            .advertise_snapcast
            .then_some(config.snapcast.streaming_port);
        let mdns_scope = config.mdns_scope();
        match advertise_mdns(
            &config.name,
            config.http.port,
            txt.clone(),
            snapcast_port,
            mdns_scope.as_deref(),
        ) {
            Ok(handle) => {
                tracing::info!(
                    port = config.http.port,
                    name = %config.name,
                    ?txt,
                    scope = ?mdns_scope,
                    "mDNS: advertising _snapdog._tcp"
                );
                Some(handle)
            }
            Err(e) => {
                tracing::warn!(error = %e, "mDNS advertisement failed");
                None
            }
        }
    } else {
        None
    };

    // ── D-Bus MPRIS2 ──────────────────────────────────────────
    #[cfg(all(feature = "dbus", target_os = "linux"))]
    let _dbus = if config.dbus.enabled {
        match dbus::start_mpris(&config, &zone_commands, notify_tx.clone()).await {
            Ok(conns) => {
                tracing::info!(zones = conns.len(), "MPRIS2 D-Bus interfaces registered");
                Some(conns)
            }
            Err(e) => {
                tracing::warn!(error = %e, "D-Bus not available — MPRIS2 disabled");
                None
            }
        }
    } else {
        None
    };

    // ── Main loop ─────────────────────────────────────────────
    let cmd_backend = backend.clone();

    // Forward zone state to snapdog clients via Custom Message Type 14
    {
        let mut meta_rx = notify_tx.subscribe();
        let meta_store = store.clone();
        let meta_backend = backend.clone();
        let meta_covers = covers.clone();
        tokio::spawn(async move {
            let mut last_cover_hash: std::collections::HashMap<usize, String> =
                std::collections::HashMap::new();
            while let Some(json) =
                api::ws::recv_skipping_lag(&mut meta_rx, "metadata forwarder").await
            {
                let Ok(notif) = serde_json::from_str::<api::ws::Notification>(&json) else {
                    continue;
                };
                let zone_index = match &notif {
                    api::ws::Notification::ZoneChanged { zone, .. } => *zone,
                    _ => continue,
                };
                // Copy what is needed out of the store and release the guard before any
                // await on the backend: a stalled client must not hold the store lock.
                let (payload, targets) = {
                    let s = meta_store.read().await;
                    let Some(zone) = s.zones.get(&zone_index) else {
                        continue;
                    };
                    let metadata = snapdog_common::TrackMetadata {
                        playback: zone.playback.to_string(),
                        source: zone.source.to_string(),
                        shuffle: zone.shuffle,
                        repeat: zone.repeat,
                        title: zone
                            .track
                            .as_ref()
                            .map_or_else(String::new, |t| t.title.clone()),
                        artist: zone
                            .track
                            .as_ref()
                            .map_or_else(String::new, |t| t.artist.clone()),
                        album: zone
                            .track
                            .as_ref()
                            .map_or_else(String::new, |t| t.album.clone()),
                        album_artist: zone.track.as_ref().and_then(|t| t.album_artist.clone()),
                        genre: zone.track.as_ref().and_then(|t| t.genre.clone()),
                        year: zone.track.as_ref().and_then(|t| t.year),
                        track_number: zone.track.as_ref().and_then(|t| t.track_number),
                        disc_number: zone.track.as_ref().and_then(|t| t.disc_number),
                        duration_ms: zone.track.as_ref().map_or(0, |t| t.duration_ms),
                        position_ms: zone.track.as_ref().map_or(0, |t| t.position_ms),
                        seekable: zone.track.as_ref().is_some_and(|t| t.seekable),
                        cover_url: zone.cover_url.clone(),
                        bitrate_kbps: zone.track.as_ref().and_then(|t| t.bitrate_kbps),
                        content_type: zone.track.as_ref().and_then(|t| t.content_type.clone()),
                        track_index: zone.playlist_track_index,
                        track_count: zone.playlist_track_count,
                        playlist: zone.playlist_index,
                        playlist_name: zone.playlist_name.clone(),
                        playlist_total: zone.playlist_total,
                        can_playlist_next: zone
                            .playlist_index
                            .zip(zone.playlist_total)
                            .is_some_and(|(p, t)| p + 1 < t),
                        can_playlist_prev: zone.playlist_index.is_some_and(|p| p > 0),
                        can_next: zone
                            .playlist_track_count
                            .is_some_and(|c| zone.playlist_track_index.is_some_and(|i| i + 1 < c)),
                        can_prev: zone.playlist_track_index.is_some_and(|i| i > 0),
                        volume: zone.volume,
                        muted: zone.muted,
                    };
                    let Ok(payload) = serde_json::to_vec(&metadata) else {
                        continue;
                    };
                    let targets: Vec<String> = s
                        .clients
                        .values()
                        .filter(|c| c.is_snapdog && c.zone_index == zone_index)
                        .filter_map(|c| c.snapcast_id.clone())
                        .collect();
                    drop(s);
                    (payload, targets)
                };
                for snap_id in &targets {
                    let _ = meta_backend
                        .execute(player::SnapcastCmd::Client {
                            client_id: snap_id.clone(),
                            action: player::ClientAction::SendCustom {
                                type_id: snapdog_common::MSG_TYPE_TRACK_METADATA,
                                payload: payload.clone(),
                            },
                        })
                        .await;
                }
                // Send cover art (Type 15) if changed
                let cover_cache = meta_covers.read().await;
                let current_hash = cover_cache
                    .get(zone_index)
                    .map(|e| e.hash.clone())
                    .unwrap_or_default();
                let prev_hash = last_cover_hash
                    .get(&zone_index)
                    .cloned()
                    .unwrap_or_default();
                if current_hash != prev_hash {
                    last_cover_hash.insert(zone_index, current_hash);
                    let cover_payload = cover_cache
                        .get(zone_index)
                        .map(|e| e.bytes.clone())
                        .unwrap_or_default();
                    drop(cover_cache);
                    for snap_id in &targets {
                        let _ = meta_backend
                            .execute(player::SnapcastCmd::Client {
                                client_id: snap_id.clone(),
                                action: player::ClientAction::SendCustom {
                                    type_id: snapdog_common::MSG_TYPE_COVER_ART,
                                    payload: cover_payload.clone(),
                                },
                            })
                            .await;
                    }
                }
            }
        });
    }

    // Volume coalescing: buffer rapid volume changes per client
    let mut coalescer = VolumeCoalescer::new(std::time::Duration::from_millis(VOLUME_COALESCE_MS));

    // Fuse the API fatal receiver so it can be safely polled inside the loop
    // without moving out of a non-Copy type on every iteration.
    let mut api_fatal_rx = Some(api_fatal_rx);

    // Signal streams live as long as the loop. Creating them inside the select arm re-registers
    // them on every iteration, so a signal that arrives while another arm runs is lost.
    #[cfg(unix)]
    let mut signals = ShutdownSignals::new()?;
    #[cfg(not(unix))]
    let mut ctrl_c = std::pin::pin!(tokio::signal::ctrl_c());
    // Exit status of the process: non-zero after a fatal error so that a service manager
    // with `Restart=on-failure` starts the server again.
    let mut exit_code = 0;

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
                    // Build on a volume still in flight, so quick relative steps add up.
                    let current = match coalescer.latest(client_id) {
                        Some(v) => v,
                        None => store.read().await.clients.values()
                            .find(|c| c.snapcast_id.as_deref() == Some(client_id))
                            .map_or(state::DEFAULT_VOLUME, |c| c.base_volume),
                    };
                    player::SnapcastCmd::Client {
                        client_id: client_id.clone(),
                        action: player::ClientAction::Volume(current.saturating_add(delta).clamp(0, 100)),
                    }
                } else {
                    cmd
                };
                // Coalesce client volume commands
                if let player::SnapcastCmd::Client { ref client_id, action: player::ClientAction::Volume(v) } = cmd {
                    coalescer.push(client_id.clone(), v);
                } else if is_process_reconcile(&cmd) {
                    // The process backend has no group logic of its own: regroup the clients
                    // through the Snapcast JSON-RPC client and re-announce them.
                    #[cfg(all(feature = "snapcast-process", not(feature = "snapcast-embedded")))]
                    snapcast::reconcile_zone_groups(process_backend.client(), &config, &store, &notify_tx).await;
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
                exit_code = 1;
                break;
            }
            // Shutdown signals
            () = async {
                #[cfg(unix)]
                {
                    let name = signals.recv().await;
                    tracing::info!(signal = name, "Shutting down gracefully...");
                }
                #[cfg(not(unix))]
                {
                    let _ = (&mut ctrl_c).await;
                    tracing::info!("Received Ctrl-C, shutting down gracefully...");
                }
            } => {
                break;
            }
        }
    }

    // ── Shutdown ──────────────────────────────────────────────

    // Tell MQTT subscribers we are gone before anything else: a clean DISCONNECT
    // suppresses the last will, so `offline` is published by hand. Bounded well
    // below the forced-exit timeout below.
    if let Some(bridge) = mqtt_bridge {
        bridge
            .shutdown(std::time::Duration::from_millis(1000))
            .await;
    }

    // Spawn a dedicated thread for the forced-exit safety net.
    // This survives the tokio runtime shutdown and handles second signals.
    std::thread::spawn(move || {
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
                    std::process::exit(exit_code);
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
                    std::process::exit(exit_code);
                }
            };
        });
    });

    if let Some(bridge) = knx_bridge {
        bridge.shutdown().await;
    }
    let _ = backend.stop().await;
    let persist_result = store.write().await.persist();
    if let Err(e) = persist_result {
        tracing::warn!(error = %e, "Failed to persist state");
    }
    #[cfg(feature = "snapcast-process")]
    snapserver.stop().await?;

    tracing::info!("SnapDog server terminated");
    // `exit` skips destructors: flush the log file explicitly.
    drop(log_guard);
    std::process::exit(exit_code);
}

/// RAII guard for the `SnapDog` server's mDNS advertisement.
///
/// Uses pure-Rust `mdns-sd` (no avahi-daemon / Bonjour-compat needed, unlike the
/// previous `astro-dnssd` path which failed inside the container). Unregisters the
/// services and shuts the responder down on drop.
struct MdnsAdvertisement {
    daemon: mdns_sd::ServiceDaemon,
    fullnames: Vec<String>,
}

impl Drop for MdnsAdvertisement {
    fn drop(&mut self) {
        for name in &self.fullnames {
            let _ = self.daemon.unregister(name);
        }
        let _ = self.daemon.shutdown();
    }
}

/// Advertise the `SnapDog` server over mDNS: `_snapdog._tcp` (HTTP/API + web UI, with
/// the full TXT record) and, optionally, a bare `_snapcast._tcp` for standard
/// Snapcast clients. `enable_addr_auto()` publishes A/AAAA for every host interface,
/// so no explicit IP/interface configuration is needed. With a `scope` (the `bind`
/// addresses of the announced services) the announcement is limited to the interfaces
/// that carry those addresses.
fn advertise_mdns(
    name: &str,
    http_port: u16,
    txt: std::collections::HashMap<String, String>,
    snapcast_port: Option<u16>,
    scope: Option<&[std::net::IpAddr]>,
) -> Result<MdnsAdvertisement, mdns_sd::Error> {
    let daemon = mdns_sd::ServiceDaemon::new()?;
    if let Some(addrs) = scope {
        daemon.disable_interface(mdns_sd::IfKind::All)?;
        for addr in addrs {
            daemon.enable_interface(mdns_sd::IfKind::Addr(*addr))?;
        }
    }
    let host = format!("{}.local.", gethostname::gethostname().to_string_lossy());
    let mut fullnames = Vec::new();

    let svc = mdns_sd::ServiceInfo::new("_snapdog._tcp.local.", name, &host, "", http_port, txt)
        .map(mdns_sd::ServiceInfo::enable_addr_auto)?;
    fullnames.push(svc.get_fullname().to_string());
    daemon.register(svc)?;

    // Non-fatal: a failure to advertise the optional `_snapcast._tcp` must not sink
    // the primary `_snapdog._tcp` registration.
    if let Some(port) = snapcast_port {
        match mdns_sd::ServiceInfo::new(
            "_snapcast._tcp.local.",
            name,
            &host,
            "",
            port,
            std::collections::HashMap::new(),
        )
        .map(mdns_sd::ServiceInfo::enable_addr_auto)
        {
            Ok(svc) => {
                let fullname = svc.get_fullname().to_string();
                match daemon.register(svc) {
                    Ok(()) => fullnames.push(fullname),
                    Err(e) => {
                        tracing::warn!(error = %e, "mDNS: _snapcast._tcp advertisement failed");
                    }
                }
            }
            Err(e) => tracing::warn!(error = %e, "mDNS: _snapcast._tcp advertisement failed"),
        }
    }

    Ok(MdnsAdvertisement { daemon, fullnames })
}

/// Detect if running inside a Docker container.
fn is_docker() -> bool {
    std::path::Path::new("/.dockerenv").exists()
        || std::fs::read_to_string("/proc/1/cgroup")
            .is_ok_and(|s| s.contains("docker") || s.contains("containerd"))
}

/// SIGINT and SIGTERM, registered once for the whole run.
///
/// Registering a listener inside a select arm drops it whenever another arm runs, and a signal
/// that arrives in that moment is lost. These streams keep a signal until it is read.
#[cfg(unix)]
struct ShutdownSignals {
    sigint: tokio::signal::unix::Signal,
    sigterm: tokio::signal::unix::Signal,
}

#[cfg(unix)]
impl ShutdownSignals {
    fn new() -> std::io::Result<Self> {
        use tokio::signal::unix::{SignalKind, signal};
        Ok(Self {
            sigint: signal(SignalKind::interrupt())?,
            sigterm: signal(SignalKind::terminate())?,
        })
    }

    /// Wait for the next shutdown signal and name it.
    async fn recv(&mut self) -> &'static str {
        tokio::select! {
            _ = self.sigint.recv() => "SIGINT",
            _ = self.sigterm.recv() => "SIGTERM",
        }
    }
}

#[cfg(all(test, unix))]
mod signal_tests {
    use super::ShutdownSignals;

    /// #501: a SIGTERM that arrives while the main loop is busy in another arm is kept and
    /// read once the loop waits again.
    #[tokio::test]
    async fn a_signal_sent_while_nobody_waits_is_not_lost() {
        let mut signals = ShutdownSignals::new().unwrap();
        let status = std::process::Command::new("kill")
            .args(["-TERM", &std::process::id().to_string()])
            .status()
            .unwrap();
        assert!(status.success());
        // The loop is busy elsewhere for a while.
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let name = tokio::time::timeout(std::time::Duration::from_secs(2), signals.recv())
            .await
            .expect("the signal is kept until it is read");
        assert_eq!(name, "SIGTERM");
    }
}

#[cfg(test)]
mod cli_tests {
    use super::{Cli, WebUiChoice, selected_webui};
    use crate::config::{HttpConfig, WebUi};
    use clap::Parser;

    #[test]
    fn webui_defaults_to_the_configured_current_mode() {
        let cli = Cli::try_parse_from(["snapdog"]).unwrap();
        assert!(cli.webui.is_none());
        assert_eq!(
            selected_webui(HttpConfig::default().webui, cli.webui),
            WebUi::Current
        );
    }

    #[test]
    fn webui_cli_choice_overrides_config_only_when_present() {
        assert_eq!(selected_webui(WebUi::Legacy, None), WebUi::Legacy);
        assert_eq!(
            selected_webui(WebUi::Current, Some(WebUiChoice::Legacy)),
            WebUi::Legacy
        );
        assert_eq!(
            selected_webui(WebUi::Legacy, Some(WebUiChoice::Current)),
            WebUi::Current
        );

        let cli = Cli::try_parse_from(["snapdog", "--webui", "legacy"]).unwrap();
        assert_eq!(selected_webui(WebUi::Current, cli.webui), WebUi::Legacy);
    }

    #[test]
    fn invalid_webui_choice_is_rejected_by_clap() {
        let error = Cli::try_parse_from(["snapdog", "--webui", "retro"])
            .err()
            .expect("an invalid WebUI choice should fail parsing");
        let message = error.to_string();
        assert!(message.contains("current"), "{message}");
        assert!(message.contains("legacy"), "{message}");
    }
}
