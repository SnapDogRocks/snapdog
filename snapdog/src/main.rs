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

use anyhow::Result;
use clap::Parser;
use tracing_subscriber::EnvFilter;

#[cfg(feature = "snapcast-process")]
use snapdog::process;
use snapdog::{api, audio, config, knx, mqtt, player, snapcast, state};

/// Multi-zone audio controller with AirPlay, Snapcast, MQTT, and KNX integration.
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

    /// mDNS service type (default: _snapdog._tcp.local.)
    #[arg(long)]
    mdns_service_type: Option<String>,

    /// mDNS advertised name (default: SnapDog)
    #[arg(long)]
    mdns_name: Option<String>,

    /// Log level: trace, debug, info, warn, error
    #[arg(short, long)]
    log_level: Option<String>,
}

/// Volume coalescing window — rapid volume changes within this window are merged.
const VOLUME_COALESCE_MS: u64 = 50;
/// Channel capacity for Snapcast commands from zone players, API, MQTT, KNX.
const SNAPCAST_CMD_CHANNEL_SIZE: usize = 64;
/// Delay after starting snapserver before connecting.
#[cfg(feature = "snapcast-process")]
const SNAPSERVER_STARTUP_DELAY: std::time::Duration = std::time::Duration::from_secs(1);
/// Path for persisted zone/client state.
const STATE_FILE: &str = "state.json";
/// Path for persisted EQ configuration.
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

#[tokio::main]
async fn main() -> Result<()> {
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
        config::load_raw(config::RawConfig::default())?
    };

    // --knx-device enables device mode
    if cli.knx_device {
        app_config.knx.enabled = true;
        app_config.knx.role = config::KnxRole::Device;
        app_config.knx.individual_address =
            Some(cli.knx_address.unwrap_or_else(|| "15.15.255".into()));
    }

    // --knx-prog-mode: set flag for BAU task to activate on start
    if cli.knx_prog_mode {
        app_config.knx.start_prog_mode = true;
    }

    // CLI overrides
    if let Some(port) = cli.port {
        app_config.http.port = port;
    }
    if let Some(ref codec) = cli.codec {
        app_config.audio.codec = match codec.as_str() {
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
    if let Some(ref s) = cli.mdns_service_type {
        app_config.snapcast.mdns_service_type = s.clone();
    }
    if let Some(ref s) = cli.mdns_name {
        app_config.snapcast.mdns_name = s.clone();
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

    // ── Initialize subsystems ─────────────────────────────────
    let store = state::init(&config, Some(&PathBuf::from(STATE_FILE)))?;
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
        std::path::Path::new(EQ_FILE),
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
    if config.knx.enabled {
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
    // Intentionally spawned without shutdown propagation: if the API dies,
    // the main loop continues serving MQTT/KNX/Snapcast. The API is non-critical.
    let api_config = (*config).clone();
    let api_store = store.clone();
    let api_commands = zone_commands.clone();
    let api_covers = covers.clone();
    let api_notify = notify_tx.clone();
    let api_snap_tx = snap_cmd_tx.clone();
    tokio::spawn(async move {
        if let Err(e) = api::serve(
            api_config,
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
            tracing::error!(error = %e, "API server failed");
        }
    });

    // ── MQTT bridge ───────────────────────────────────────────
    let mut mqtt_bridge = if let Some(mqtt_config) = &config.mqtt {
        match mqtt::MqttBridge::connect(mqtt_config).await {
            Ok(bridge) => {
                if let Err(e) = bridge.subscribe_commands().await {
                    tracing::warn!(error = %e, "MQTT subscribe failed");
                }
                // Publish initial state for all zones and clients
                let snap = store.read().await;
                for (&idx, zone) in &snap.zones {
                    if let Err(e) = bridge
                        .publish_zone_state(idx, zone)
                        .await
                    {
                        tracing::warn!(zone = idx, error = %e, "MQTT initial zone publish failed");
                    }
                }
                for (&idx, client) in &snap.clients {
                    if let Err(e) = bridge.publish_client_state(idx, client).await {
                        tracing::warn!(client = idx, error = %e, "MQTT initial client publish failed");
                    }
                }
                drop(snap);
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

    // ── Main loop ─────────────────────────────────────────────
    let mqtt_zone_cmds = zone_commands.clone();
    let mqtt_store = store.clone();
    let mqtt_snap_tx = snap_cmd_tx.clone();
    let cmd_backend = backend.clone();

    // Volume coalescing: buffer rapid volume changes per client
    let mut coalescer = VolumeCoalescer::new(std::time::Duration::from_millis(VOLUME_COALESCE_MS));

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
            _ = sleep => {
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
            _ = async {
                #[cfg(all(feature = "snapcast-process", not(feature = "snapcast-embedded")))]
                if let Ok(notification) = snap_notifications.recv().await {
                    snapcast::handle_notification(notification, &config, process_backend.client(), &store, &notify_tx).await;
                }
                #[cfg(not(all(feature = "snapcast-process", not(feature = "snapcast-embedded"))))]
                std::future::pending::<()>().await;
            } => {}
            // MQTT events
            _ = async {
                if let Some(ref mut bridge) = mqtt_bridge {
                    bridge.poll_once(&mqtt_zone_cmds, &mqtt_store, &mqtt_snap_tx).await;
                } else {
                    std::future::pending::<()>().await;
                }
            } => {}
            // Shutdown
            _ = tokio::signal::ctrl_c() => {
                tracing::info!("Shutting down");
                break;
            }
        }
    }

    // ── Shutdown ──────────────────────────────────────────────
    // Second Ctrl+C during graceful shutdown → force exit immediately
    tokio::spawn(async {
        let _ = tokio::signal::ctrl_c().await;
        tracing::warn!("Forced exit");
        std::process::exit(1);
    });

    let _ = backend.stop().await;
    if let Err(e) = store.write().await.persist() {
        tracing::warn!(error = %e, "Failed to persist state");
    }
    #[cfg(feature = "snapcast-process")]
    snapserver.stop().await?;

    // Force exit — background threads (mDNS, AirPlay) don't respond to tokio shutdown
    std::process::exit(0);
}
