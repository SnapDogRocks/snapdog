// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

// Pedantic lints allowed crate-wide: audio code uses intentional numeric casts,
// and the main event loop is necessarily long.
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_sign_loss)]
#![allow(clippy::cast_precision_loss)]

mod cli;
#[cfg(feature = "dbus")]
mod dbus;
mod eq;
mod logging;
mod player;

use clap::Parser;
use snapcast_client::{ClientCommand, ClientConfig, ClientEvent, SnapClient};

use snapdog_common::CLIENT_NAME;

const DEFAULT_SAMPLE_RATE: u32 = snapdog_common::DEFAULT_SAMPLE_RATE;

#[allow(
    clippy::too_many_lines,
    clippy::case_sensitive_file_extension_comparisons,
    clippy::needless_update
)] // Main app setup + event loop — splitting would obscure control flow
fn main() -> anyhow::Result<()> {
    let cli = cli::Cli::parse();

    logging::init(&cli.logsink, &cli.logfilter)?;

    if cli.list {
        list_devices(&cli.player);
        return Ok(());
    }

    if cli.test_tone {
        player::play_test_tone(&cli.soundcard)?;
        return Ok(());
    }

    #[cfg(feature = "encryption")]
    let encryption_psk = cli.encryption_psk.clone();
    let null_player = cli.player == "null";
    let mixer_raw = cli.mixer.clone();
    let mut settings = cli.into_settings()?;

    #[cfg(unix)]
    if let Some(ref daemon) = settings.daemon {
        daemonize(daemon)?;
    }

    // mDNS discovery: browse _snapdog._tcp, read snapcast_port from TXT
    if settings.server.host.is_empty() {
        #[cfg(feature = "mdns")]
        {
            tracing::info!("No server specified, browsing mDNS for _snapdog._tcp...");
            let (host, port) = discover_snapdog()?;
            settings.server.host = host;
            settings.server.port = port;
        }
        #[cfg(not(feature = "mdns"))]
        anyhow::bail!("No server specified and mDNS feature not enabled");
    }

    tracing::info!(
        server = %format!(
            "{}://{}:{}",
            settings.server.scheme, settings.server.host, settings.server.port
        ),
        instance = settings.instance,
        "snapdog-client starting"
    );

    let config = ClientConfig {
        scheme: settings.server.scheme.clone(),
        host: settings.server.host.clone(),
        port: settings.server.port,
        auth: settings.server.auth.clone(),
        #[cfg(feature = "tls")]
        server_certificate: settings.server.server_certificate.clone(),
        #[cfg(feature = "tls")]
        certificate: settings.server.certificate.clone(),
        #[cfg(feature = "tls")]
        certificate_key: settings.server.certificate_key.clone(),
        #[cfg(feature = "tls")]
        key_password: settings.server.key_password.clone(),
        #[cfg(feature = "encryption")]
        encryption_psk: Some(
            encryption_psk.unwrap_or_else(|| snapcast_proto::DEFAULT_ENCRYPTION_PSK.into()),
        ),
        instance: settings.instance,
        host_id: settings.host_id.clone(),
        latency: settings.player.latency,
        client_name: CLIENT_NAME.into(),
        ..ClientConfig::default()
    };
    let rt = tokio::runtime::Runtime::new()?;

    rt.block_on(async {
        let (mut client, mut events, audio_rx) = SnapClient::new(config);
        let cmd = client.command_sender();

        // EQ processors — shared between event loop and audio thread
        let eq = std::sync::Arc::new(std::sync::Mutex::new(eq::ZoneEq::new(
            DEFAULT_SAMPLE_RATE,
            2,
        )));
        let speaker_eq = std::sync::Arc::new(std::sync::Mutex::new(eq::ZoneEq::new(
            DEFAULT_SAMPLE_RATE,
            2,
        )));

        // Fade state — shared between event loop (trigger) and audio thread (apply)
        let fade = std::sync::Arc::new(player::FadeState::new());

        // Stream sample rate — updated when format is detected
        let stream_sample_rate =
            std::sync::Arc::new(std::sync::atomic::AtomicU32::new(DEFAULT_SAMPLE_RATE));

        // Mixer — dispatches volume to software, hardware, midi, or none
        let volume = player::VolumeState::new();
        let mixer = std::sync::Arc::new(player::Mixer::from_cli(&mixer_raw, volume));

        // Audio output: cpal callback reads from Stream directly
        let player_stream = std::sync::Arc::clone(&client.stream);
        let player_tp = std::sync::Arc::clone(&client.time_provider);
        let player_eq = eq.clone();
        let player_speaker_eq = speaker_eq.clone();
        let player_mixer = mixer.clone();
        let player_fade = fade.clone();
        if null_player {
            tracing::info!("Null player — audio output disabled");
            tokio::spawn(async move {
                let mut rx = audio_rx;
                while rx.recv().await.is_some() {}
            });
        } else {
            tokio::spawn(async move {
                player::play_audio(
                    audio_rx,
                    player_stream,
                    player_tp,
                    player_eq,
                    player_speaker_eq,
                    player_mixer,
                    player_fade,
                )
                .await;
            });
        }

        // Event handler
        // D-Bus MPRIS2 interface
        #[cfg(feature = "dbus")]
        let (_dbus_conn, dbus_state, _dbus_iface) = match dbus::start(cmd.clone()).await {
            Ok(v) => (Some(v.0), Some(v.1), Some(v.2)),
            Err(e) => {
                tracing::warn!(error = %e, "D-Bus not available — MPRIS2 disabled");
                (None, None, None)
            }
        };

        let event_eq = eq.clone();
        let event_speaker_eq = speaker_eq.clone();
        let event_fade = fade.clone();
        let event_mixer = mixer.clone();
        let event_sample_rate = stream_sample_rate.clone();
        tokio::spawn(async move {
            let mut last_eq_config: Option<eq::EqConfig> = None;
            let mut last_speaker_config: Option<eq::EqConfig> = None;
            while let Some(event) = events.recv().await {
                match event {
                    ClientEvent::Connected { host, port } => {
                        tracing::info!(host, port, "Connected");
                    }
                    ClientEvent::Disconnected { .. } => {}
                    ClientEvent::VolumeChanged { volume, muted } => {
                        tracing::info!(volume, muted, "Volume changed");
                        event_mixer.set_volume(volume as u8, muted);
                        #[cfg(feature = "dbus")]
                        if let Some(ref state) = dbus_state {
                            state.lock().await.set_volume(volume, muted);
                        }
                        #[cfg(target_os = "linux")]
                        {
                            let status =
                                format!("Volume: {volume}%{}", if muted { " (muted)" } else { "" });
                            let _ = sd_notify::notify(&[sd_notify::NotifyState::Status(&status)]);
                        }
                    }
                    ClientEvent::TimeSyncComplete { diff_ms } => {
                        tracing::info!(diff_ms, "Time sync complete");
                        #[cfg(target_os = "linux")]
                        let _ = sd_notify::notify(&[sd_notify::NotifyState::Ready]);
                    }
                    ClientEvent::StreamStarted { codec, format } => {
                        tracing::info!(%codec, %format, "Stream started");
                        #[cfg(target_os = "linux")]
                        {
                            let status = format!(
                                "Playing {codec} ({} Hz, {} bits, {} ch)",
                                format.rate(),
                                format.bits(),
                                format.channels()
                            );
                            let _ = sd_notify::notify(&[sd_notify::NotifyState::Status(&status)]);
                        }
                        event_sample_rate
                            .store(format.rate(), std::sync::atomic::Ordering::Relaxed);
                        let mut eq = event_eq
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner);
                        *eq = eq::ZoneEq::new(format.rate(), format.channels());
                        if let Some(ref config) = last_eq_config {
                            eq.set_config(config);
                        }
                        drop(eq);
                        let mut spk = event_speaker_eq
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner);
                        *spk = eq::ZoneEq::new(format.rate(), format.channels());
                        if let Some(ref config) = last_speaker_config {
                            spk.set_config(config);
                        }
                    }
                    #[cfg(feature = "custom-protocol")]
                    ClientEvent::CustomMessage(msg) if msg.type_id == eq::TYPE_EQ_CONFIG => {
                        match serde_json::from_slice::<eq::EqConfig>(&msg.payload) {
                            Ok(config) => {
                                tracing::info!(
                                    enabled = config.enabled,
                                    bands = config.bands.len(),
                                    "EQ config received"
                                );
                                event_eq
                                    .lock()
                                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                                    .set_config(&config);
                                last_eq_config = Some(config);
                            }
                            Err(e) => tracing::warn!(error = %e, "Invalid EQ config payload"),
                        }
                    }
                    #[cfg(feature = "custom-protocol")]
                    ClientEvent::CustomMessage(msg) if msg.type_id == eq::TYPE_SPEAKER_EQ => {
                        match serde_json::from_slice::<eq::EqConfig>(&msg.payload) {
                            Ok(config) => {
                                tracing::info!(
                                    enabled = config.enabled,
                                    bands = config.bands.len(),
                                    "Speaker correction received"
                                );
                                event_speaker_eq
                                    .lock()
                                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                                    .set_config(&config);
                                last_speaker_config = Some(config);
                            }
                            Err(e) => {
                                tracing::warn!(error = %e, "Invalid speaker EQ payload");
                            }
                        }
                    }
                    #[cfg(feature = "custom-protocol")]
                    ClientEvent::CustomMessage(msg)
                        if msg.type_id == snapdog_common::MSG_TYPE_FADE_OUT =>
                    {
                        let duration_ms = if msg.payload.len() >= 2 {
                            u16::from_le_bytes([msg.payload[0], msg.payload[1]])
                        } else {
                            snapdog_common::DEFAULT_FADE_MS
                        };
                        tracing::info!(duration_ms, "Fade-out triggered");
                        event_fade.trigger_fade_out(
                            duration_ms,
                            event_sample_rate.load(std::sync::atomic::Ordering::Relaxed),
                        );
                    }
                    #[cfg(feature = "custom-protocol")]
                    ClientEvent::CustomMessage(msg)
                        if msg.type_id == snapdog_common::MSG_TYPE_TRACK_METADATA =>
                    {
                        if let Ok(meta) =
                            serde_json::from_slice::<snapdog_common::TrackMetadata>(&msg.payload)
                        {
                            tracing::debug!(title = %meta.title, artist = %meta.artist, "Metadata received");
                            #[cfg(feature = "dbus")]
                            if let Some(ref state) = dbus_state {
                                let mut s = state.lock().await;
                                s.volume = meta.volume as u16;
                                s.muted = meta.muted;
                                s.playing = meta.playback == "playing";
                            }
                        }
                    }
                    #[cfg(feature = "custom-protocol")]
                    ClientEvent::CustomMessage(msg)
                        if msg.type_id == snapdog_common::MSG_TYPE_COVER_ART =>
                    {
                        let ext = if msg.payload.starts_with(&[0xFF, 0xD8]) {
                            "jpg"
                        } else {
                            "png"
                        };
                        let file_path = std::env::temp_dir().join(format!(
                            "snapdog-cover-{:x}.{ext}",
                            std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_millis()
                        ));
                        if let Err(e) = std::fs::write(&file_path, &msg.payload) {
                            tracing::warn!(error = %e, "Failed to write cover art");
                        } else {
                            tracing::debug!(path = %file_path.display(), bytes = msg.payload.len(), "Cover art saved");
                        }
                    }
                    _ => {}
                }
            }
        });

        // Graceful shutdown: SIGINT/SIGTERM → stop client, forced on second signal or timeout
        tokio::spawn(async move {
            #[cfg(unix)]
            {
                use tokio::signal::unix::{SignalKind, signal};
                let mut sigint = signal(SignalKind::interrupt()).unwrap();
                let mut sigterm = signal(SignalKind::terminate()).unwrap();
                tokio::select! {
                    _ = sigint.recv() => tracing::info!("Received SIGINT, shutting down"),
                    _ = sigterm.recv() => tracing::info!("Received SIGTERM, shutting down"),
                }
            }
            #[cfg(not(unix))]
            {
                tokio::signal::ctrl_c().await.ok();
                tracing::info!("Received Ctrl-C, shutting down");
            }

            cmd.send(ClientCommand::Stop).await.ok();

            // Safety net: force exit on second signal or timeout
            std::thread::spawn(|| {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap();
                rt.block_on(async {
                    #[cfg(unix)]
                    {
                        use tokio::signal::unix::{SignalKind, signal};
                        let mut sigint = signal(SignalKind::interrupt()).unwrap();
                        let mut sigterm = signal(SignalKind::terminate()).unwrap();
                        tokio::select! {
                            _ = sigint.recv() => {}
                            _ = sigterm.recv() => {}
                            () = tokio::time::sleep(std::time::Duration::from_secs(3)) => {}
                        }
                    }
                    #[cfg(not(unix))]
                    {
                        tokio::select! {
                            _ = tokio::signal::ctrl_c() => {}
                            _ = tokio::time::sleep(std::time::Duration::from_secs(3)) => {}
                        }
                    }
                    tracing::warn!("Forced exit");
                    std::process::exit(0);
                });
            });
        });

        client.run().await
    })?;

    tracing::info!("snapdog-client terminated");
    Ok(())
}

fn list_devices(player: &str) {
    let player_name = player.split(':').next().unwrap_or("");
    match player_name {
        #[cfg(target_os = "macos")]
        "coreaudio" | "" => {
            println!("0: Default Output\nCoreAudio default output device\n");
        }
        _ => println!("No device listing available for '{player_name}'"),
    }
}

#[cfg(unix)]
#[allow(unsafe_code)]
fn daemonize(daemon: &snapcast_client::config::DaemonSettings) -> anyhow::Result<()> {
    if let Some(priority) = daemon.priority {
        let priority = priority.clamp(-20, 19);
        unsafe {
            libc::setpriority(libc::PRIO_PROCESS, 0, priority);
        }
        tracing::info!(priority, "Process priority set");
    }

    if let Some(ref user) = daemon.user {
        tracing::info!(user, "Would drop privileges to user (not yet implemented)");
    }

    unsafe {
        let pid = libc::fork();
        if pid < 0 {
            anyhow::bail!("fork failed");
        }
        if pid > 0 {
            std::process::exit(0);
        }
        libc::setsid();
    }

    tracing::info!("Daemonized");
    Ok(())
}

/// Discover a `SnapDog` server via mDNS. Returns `(host, snapcast_port)`.
#[cfg(feature = "mdns")]
fn discover_snapdog() -> anyhow::Result<(String, u16)> {
    use std::time::Duration;
    let mdns = mdns_sd::ServiceDaemon::new()?;
    let service_type = "_snapdog._tcp.local.";
    let receiver = mdns.browse(service_type)?;
    let deadline = std::time::Instant::now() + Duration::from_secs(5);

    loop {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            mdns.stop_browse(service_type).ok();
            anyhow::bail!("mDNS discovery timed out after 5s");
        }
        match receiver.recv_timeout(remaining) {
            Ok(mdns_sd::ServiceEvent::ServiceResolved(info)) => {
                let host = info.get_addresses().iter().next().map_or_else(
                    || info.get_hostname().trim_end_matches('.').to_string(),
                    ToString::to_string,
                );
                let port = info
                    .get_properties()
                    .get("snapcast_port")
                    .and_then(|v| v.val_str().parse::<u16>().ok())
                    .unwrap_or(snapcast_proto::DEFAULT_STREAM_PORT);
                tracing::info!(
                    host = %host,
                    port,
                    name = %info.get_fullname(),
                    "Discovered SnapDog server via mDNS"
                );
                mdns.stop_browse(service_type).ok();
                return Ok((host, port));
            }
            Ok(_) => {}
            Err(_) => {
                mdns.stop_browse(service_type).ok();
                anyhow::bail!("mDNS discovery timed out after 5s");
            }
        }
    }
}
