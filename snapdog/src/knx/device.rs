// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! Device-mode KNX transport — runs as ETS-programmable KNX/IP device.
//!
//! Wraps `DeviceServer` + `Bau`. The BAU runs in a dedicated task;
//! the transport communicates via channels.

use std::net::Ipv4Addr;
use std::path::PathBuf;
use std::str::FromStr;

use anyhow::{Context, Result};
use knx_rs_core::address::{GroupAddress, IndividualAddress};
use knx_rs_core::dpt::{Dpt, DptValue};
use knx_rs_device::bau::Bau;
use knx_rs_device::device_object;
use knx_rs_ip::tunnel_server::{DeviceServer, ServerEvent};
use tokio::sync::{mpsc, oneshot};

use super::group_objects::mem;

/// Current monotonic time in milliseconds (for transport layer timeouts).
fn now_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// KNX device serial number: manufacturer prefix (0x00FA) + last 4 bytes of host MAC.
/// Unique per host, no configuration needed.
fn device_serial() -> [u8; 6] {
    let mac = mac_address::get_mac_address().ok().flatten().map_or(
        [0x01, 0x02, 0x03, 0x04, 0x05, 0x06],
        mac_address::MacAddress::bytes,
    );
    [0x00, 0xFA, mac[2], mac[3], mac[4], mac[5]]
}

/// Maximum concurrent KNXnet/IP tunnelling connections.
const MAX_TUNNEL_CONNECTIONS: usize = 1;

/// Channel buffer size for BAU command/update channels.
const BAU_CHANNEL_CAPACITY: usize = 64;

/// Debounce delay after ETS memory changes before persisting to disk.
const ETS_PERSIST_DEBOUNCE: std::time::Duration = std::time::Duration::from_secs(2);

/// Path for persisted ETS memory.
const ETS_MEMORY_PATH: &str = "knx-memory.bin";

// ── Persistence format ────────────────────────────────────────
//
// File layout: [magic 4B] [version 1B] [length 2B (LE)] [payload …] [crc32 4B (LE)]
// Total overhead: 11 bytes. Atomic write via temp file + rename.

/// Magic bytes identifying a `SnapDog` KNX memory file.
const PERSIST_MAGIC: &[u8; 4] = b"SDKM";

/// Current persistence format version. Bump when memory layout changes.
const PERSIST_VERSION: u8 = 1;

/// Header size: magic (4) + version (1) + length (2).
const PERSIST_HEADER: usize = 7;

/// Write ETS memory to disk with integrity envelope (atomic).
fn persist_memory(path: &std::path::Path, payload: &[u8]) -> std::io::Result<()> {
    let len = payload.len() as u16;
    let crc = crc32(payload);

    let mut buf = Vec::with_capacity(PERSIST_HEADER + payload.len() + 4);
    buf.extend_from_slice(PERSIST_MAGIC);
    buf.push(PERSIST_VERSION);
    buf.extend_from_slice(&len.to_le_bytes());
    buf.extend_from_slice(payload);
    buf.extend_from_slice(&crc.to_le_bytes());

    // Atomic write: temp file + rename
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, &buf)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// Load ETS memory from disk, verifying magic, version, length, and CRC32.
fn load_memory(path: &std::path::Path) -> Result<Vec<u8>> {
    let data = std::fs::read(path).context("reading ETS memory file")?;
    anyhow::ensure!(
        data.len() >= PERSIST_HEADER + 4,
        "file too short ({} bytes)",
        data.len()
    );
    anyhow::ensure!(
        &data[..4] == PERSIST_MAGIC,
        "invalid magic (not a SnapDog KNX memory file)"
    );
    anyhow::ensure!(
        data[4] == PERSIST_VERSION,
        "unsupported version {} (expected {})",
        data[4],
        PERSIST_VERSION
    );

    let len = u16::from_le_bytes([data[5], data[6]]) as usize;
    anyhow::ensure!(
        data.len() == PERSIST_HEADER + len + 4,
        "length mismatch: header says {len} bytes, file has {}",
        data.len() - PERSIST_HEADER - 4
    );

    let payload = &data[PERSIST_HEADER..PERSIST_HEADER + len];
    let stored_crc = u32::from_le_bytes([
        data[PERSIST_HEADER + len],
        data[PERSIST_HEADER + len + 1],
        data[PERSIST_HEADER + len + 2],
        data[PERSIST_HEADER + len + 3],
    ]);
    let actual_crc = crc32(payload);
    anyhow::ensure!(
        stored_crc == actual_crc,
        "CRC32 mismatch (file corrupted): stored {stored_crc:#010x}, computed {actual_crc:#010x}"
    );

    Ok(payload.to_vec())
}

/// CRC32 (ISO 3309 / ITU-T V.42 — same as zlib/PNG).
fn crc32(data: &[u8]) -> u32 {
    crc32fast::hash(data)
}

use super::group_objects::{
    self, CGO_CONNECTED, CGO_LATENCY, CGO_LATENCY_STATUS, CGO_MUTE, CGO_MUTE_STATUS,
    CGO_MUTE_TOGGLE, CGO_VOLUME, CGO_VOLUME_DIM, CGO_VOLUME_STATUS, CGO_ZONE, CGO_ZONE_STATUS,
    CLIENT_GOS, MAX_CLIENTS, MAX_ZONES, TOTAL_GO_COUNT, ZGO_CONTROL_STATUS, ZGO_MUTE,
    ZGO_MUTE_STATUS, ZGO_MUTE_TOGGLE, ZGO_PAUSE, ZGO_PLAY, ZGO_PLAYLIST, ZGO_PLAYLIST_NEXT,
    ZGO_PLAYLIST_PREVIOUS, ZGO_PLAYLIST_STATUS, ZGO_PRESENCE, ZGO_PRESENCE_ENABLE,
    ZGO_PRESENCE_SOURCE_OVERRIDE, ZGO_PRESENCE_TIMEOUT, ZGO_PRESENCE_TIMER_ACTIVE, ZGO_REPEAT,
    ZGO_REPEAT_STATUS, ZGO_REPEAT_TOGGLE, ZGO_SHUFFLE, ZGO_SHUFFLE_STATUS, ZGO_SHUFFLE_TOGGLE,
    ZGO_STOP, ZGO_TRACK_ALBUM, ZGO_TRACK_ARTIST, ZGO_TRACK_NEXT, ZGO_TRACK_PLAYING,
    ZGO_TRACK_PREVIOUS, ZGO_TRACK_PROGRESS, ZGO_TRACK_REPEAT, ZGO_TRACK_REPEAT_STATUS,
    ZGO_TRACK_REPEAT_TOGGLE, ZGO_TRACK_TITLE, ZGO_VOLUME, ZGO_VOLUME_DIM, ZGO_VOLUME_STATUS,
    ZONE_GOS,
};

/// Parsed ETS parameters from BAU memory.
#[derive(Debug, Default)]
pub struct EtsParams {
    // Numeric
    pub zone_active: [bool; MAX_ZONES],
    pub zone_default_volume: [u8; MAX_ZONES],
    pub zone_max_volume: [u8; MAX_ZONES],
    pub zone_airplay: [bool; MAX_ZONES],
    pub zone_spotify: [bool; MAX_ZONES],
    pub zone_presence_enabled: [bool; MAX_ZONES],
    pub zone_presence_timeout: [u16; MAX_ZONES],
    pub client_active: [bool; MAX_CLIENTS],
    pub client_default_zone: [u8; MAX_CLIENTS],
    pub client_default_volume: [u8; MAX_CLIENTS],
    pub client_max_volume: [u8; MAX_CLIENTS],
    pub client_default_latency: [u8; MAX_CLIENTS],
    pub http_port: u16,
    pub log_level: u8,
    pub radio_active: [bool; mem::MAX_RADIOS],
    // Strings
    pub zone_names: [String; MAX_ZONES],
    pub client_names: [String; MAX_CLIENTS],
    pub client_macs: [String; MAX_CLIENTS],
    pub subsonic_url: String,
    pub subsonic_user: String,
    pub subsonic_pass: String,
    pub mqtt_broker: String,
    pub mqtt_topic: String,
    pub radio_names: [String; mem::MAX_RADIOS],
    pub radio_urls: [String; mem::MAX_RADIOS],
}

/// Read a null-terminated or fixed-length string from a byte slice.
fn read_string(data: &[u8], offset: usize, max_len: usize) -> String {
    if offset + max_len > data.len() {
        return String::new();
    }
    let bytes = &data[offset..offset + max_len];
    // Find null terminator or use full length
    let len = bytes.iter().position(|&b| b == 0).unwrap_or(max_len);
    String::from_utf8_lossy(&bytes[..len]).into_owned()
}

/// Parse ETS parameters from BAU memory area.
pub fn parse_ets_memory(data: &[u8]) -> EtsParams {
    let mut p = EtsParams::default();
    if data.len() < mem::TOTAL {
        return p;
    }
    // Numeric — zones
    for i in 0..MAX_ZONES {
        p.zone_active[i] = data[mem::ZONE_ACTIVE + i] != 0;
        p.zone_default_volume[i] = data[mem::ZONE_DEF_VOL + i];
        p.zone_max_volume[i] = data[mem::ZONE_MAX_VOL + i];
        p.zone_airplay[i] = data[mem::ZONE_AIRPLAY + i] != 0;
        p.zone_spotify[i] = data[mem::ZONE_SPOTIFY + i] != 0;
        p.zone_presence_enabled[i] = data[mem::ZONE_PRESENCE_EN + i] != 0;
        let to_off = mem::ZONE_PRESENCE_TO + i * 2;
        p.zone_presence_timeout[i] = u16::from_be_bytes([data[to_off], data[to_off + 1]]);
    }
    // Numeric — clients
    for i in 0..MAX_CLIENTS {
        p.client_active[i] = data[mem::CLIENT_ACTIVE + i] != 0;
        p.client_default_zone[i] = data[mem::CLIENT_DEF_ZONE + i];
        p.client_default_volume[i] = data[mem::CLIENT_DEF_VOL + i];
        p.client_max_volume[i] = data[mem::CLIENT_MAX_VOL + i];
        p.client_default_latency[i] = data[mem::CLIENT_DEF_LAT + i];
    }
    // Numeric — global
    p.http_port =
        u16::from_be_bytes([data[mem::GLOBAL_HTTP_PORT], data[mem::GLOBAL_HTTP_PORT + 1]]);
    p.log_level = data[mem::GLOBAL_LOG_LVL];
    for i in 0..mem::MAX_RADIOS {
        p.radio_active[i] = data[mem::RADIO_ACTIVE + i] != 0;
    }
    // Strings
    for i in 0..MAX_ZONES {
        p.zone_names[i] = read_string(
            data,
            mem::ZONE_NAME + i * mem::ZONE_NAME_SIZE,
            mem::ZONE_NAME_SIZE,
        );
    }
    for i in 0..MAX_CLIENTS {
        p.client_names[i] = read_string(
            data,
            mem::CLIENT_NAME + i * mem::CLIENT_NAME_SIZE,
            mem::CLIENT_NAME_SIZE,
        );
        p.client_macs[i] = read_string(
            data,
            mem::CLIENT_MAC + i * mem::CLIENT_MAC_SIZE,
            mem::CLIENT_MAC_SIZE,
        );
    }
    p.subsonic_url = read_string(data, mem::GLOBAL_SUB_URL, mem::GLOBAL_SUB_URL_SIZE);
    p.subsonic_user = read_string(data, mem::GLOBAL_SUB_USER, mem::GLOBAL_SUB_USER_SIZE);
    p.subsonic_pass = read_string(data, mem::GLOBAL_SUB_PASS, mem::GLOBAL_SUB_PASS_SIZE);
    p.mqtt_broker = read_string(data, mem::GLOBAL_MQTT_BROKER, mem::GLOBAL_MQTT_BROKER_SIZE);
    p.mqtt_topic = read_string(data, mem::GLOBAL_MQTT_TOPIC, mem::GLOBAL_MQTT_TOPIC_SIZE);
    for i in 0..mem::MAX_RADIOS {
        p.radio_names[i] = read_string(
            data,
            mem::RADIO_NAME + i * mem::RADIO_NAME_SIZE,
            mem::RADIO_NAME_SIZE,
        );
        p.radio_urls[i] = read_string(
            data,
            mem::RADIO_URL + i * mem::RADIO_URL_SIZE,
            mem::RADIO_URL_SIZE,
        );
    }
    p
}

/// Command sent to the BAU task.
enum BauCmd {
    /// Write a DPT-encoded value to a group object by GA string.
    Write {
        ga: GroupAddress,
        dpt: Dpt,
        value: DptValue,
    },
    /// Set programming mode on/off.
    SetProgMode { enabled: bool },
    /// Get current programming mode state.
    GetProgMode { reply: oneshot::Sender<bool> },
}

/// Device-mode publisher: sends values to the BAU task.
#[derive(Clone)]
pub struct DevicePublisher {
    cmd_tx: mpsc::Sender<BauCmd>,
}

impl DevicePublisher {
    async fn send_cmd(&self, cmd: BauCmd) {
        let _ = self.cmd_tx.send(cmd).await;
    }
}

impl super::transport::KnxDeviceControl for DevicePublisher {
    async fn set_prog_mode(&self, enabled: bool) {
        self.send_cmd(BauCmd::SetProgMode { enabled }).await;
    }

    async fn get_prog_mode(&self) -> bool {
        let (tx, rx) = oneshot::channel();
        self.send_cmd(BauCmd::GetProgMode { reply: tx }).await;
        rx.await.unwrap_or(false)
    }
}

/// Device-mode listener: receives group object updates from the BAU task.
pub struct DeviceListener {
    update_rx: mpsc::Receiver<(GroupAddress, Vec<u8>)>,
}

/// Start the device server and BAU, returning a publisher/listener pair.
pub async fn start_device_transport(
    individual_address: &str,
    config: &crate::config::AppConfig,
) -> Result<(DevicePublisher, DeviceListener, Option<EtsParams>)> {
    let ia = IndividualAddress::from_str(individual_address)
        .map_err(|e| anyhow::anyhow!("Invalid individual address: {e}"))?;

    let server = DeviceServer::start(Ipv4Addr::UNSPECIFIED)
        .await
        .context("Failed to start KNX device server")?;

    let knx = config.knx.as_ref().expect("KNX config required");
    let persist = knx.persist_ets_config.unwrap_or(true);
    let restart_after_ets = knx.restart_after_ets.unwrap_or(true);
    let persist_path = if persist {
        Some(PathBuf::from(ETS_MEMORY_PATH))
    } else {
        None
    };

    // Build BAU and restore state before spawning the task
    let mut bau = build_bau(ia, config);
    let mut ets_params = None;

    if let Some(ref path) = persist_path {
        match load_memory(path) {
            Ok(data) => {
                if bau.restore(&data).is_ok() {
                    tracing::info!(
                        path = %path.display(),
                        addr_table = ?bau.addr_table_object().load_state(),
                        assoc_table = ?bau.assoc_table_object().load_state(),
                        "Restored ETS device state"
                    );
                } else {
                    tracing::warn!(path = %path.display(), "ETS state restore failed — ETS will need to reprogram");
                }
            }
            Err(e) if path.exists() => {
                tracing::warn!(path = %path.display(), error = %e, "Discarding corrupted ETS memory file — ETS will need to reprogram");
            }
            Err(_) => {}
        }
    }

    let ets_programmed = bau.configured();
    if ets_programmed {
        tracing::info!("Using ETS-programmed group address tables (TOML KNX addresses ignored)");
        let ets = parse_ets_memory(bau.memory_area());
        tracing::info!(
            zones = ets.zone_active.iter().filter(|&&a| a).count(),
            clients = ets.client_active.iter().filter(|&&a| a).count(),
            radios = ets.radio_active.iter().filter(|&&a| a).count(),
            subsonic = !ets.subsonic_url.is_empty(),
            mqtt = !ets.mqtt_broker.is_empty(),
            "ETS parameters loaded"
        );
        ets_params = Some(ets);
    } else {
        tracing::info!("No ETS programming found — using group addresses from TOML config");
        build_tables_from_config(&mut bau, config);
    }

    let (cmd_tx, cmd_rx) = mpsc::channel(BAU_CHANNEL_CAPACITY);
    let (update_tx, update_rx) = mpsc::channel(BAU_CHANNEL_CAPACITY);

    // Activate programming mode if requested via CLI
    if config.knx.as_ref().is_some_and(|k| k.start_prog_mode) {
        knx_rs_device::device_object::set_prog_mode(bau.device_mut(), true);
        tracing::info!("KNX programming mode active (--knx-prog-mode)");
    }

    tokio::spawn(bau_task_loop(
        bau,
        server,
        cmd_rx,
        update_tx,
        persist_path,
        restart_after_ets,
    ));

    tracing::info!(%individual_address, persist, "KNX device mode started");

    Ok((
        DevicePublisher { cmd_tx },
        DeviceListener { update_rx },
        ets_params,
    ))
}

impl super::transport::KnxPublisher for DevicePublisher {
    async fn write(&self, ga: GroupAddress, dpt: Dpt, value: &DptValue) {
        let _ = self
            .cmd_tx
            .send(BauCmd::Write {
                ga,
                dpt,
                value: value.clone(),
            })
            .await;
    }
}

impl super::transport::KnxListener for DeviceListener {
    async fn recv_group_write(&mut self) -> Option<(GroupAddress, Vec<u8>)> {
        self.update_rx.recv().await
    }
}

// ── BAU task ──────────────────────────────────────────────────

async fn bau_task_loop(
    mut bau: Bau,
    mut server: DeviceServer,
    mut cmd_rx: mpsc::Receiver<BauCmd>,
    update_tx: mpsc::Sender<(GroupAddress, Vec<u8>)>,
    persist_path: Option<PathBuf>,
    restart_after_ets: bool,
) {
    let mut memory_dirty = false;
    let persist_timer = tokio::time::sleep(ETS_PERSIST_DEBOUNCE);
    tokio::pin!(persist_timer);
    let mut persist_armed = false;
    let mut tunnel_active = false;

    loop {
        tokio::select! {
            event = server.recv() => {
                let Some(event) = event else { break };
                let is_tunnel = matches!(event, ServerEvent::TunnelFrame(_));
                let frame = match event {
                    ServerEvent::TunnelFrame(f) | ServerEvent::RoutingFrame(f) => f,
                };

                // Log tunnel client connections
                if is_tunnel && !tunnel_active {
                    tunnel_active = true;
                    tracing::info!(
                        source = %frame.source_address(),
                        "KNX tunnel client connected (ETS programming session)"
                    );
                }

                bau.process_frame(&frame, now_ms());
                bau.poll(now_ms());

                while let Some(out) = bau.next_outgoing_frame() {
                    let _ = server.send_frame(out).await;
                }

                // ETS programs via tunnel — mark dirty for persistence
                if is_tunnel && persist_path.is_some() && !bau.memory_area().is_empty() {
                    memory_dirty = true;
                    persist_timer.as_mut().reset(tokio::time::Instant::now() + ETS_PERSIST_DEBOUNCE);
                    persist_armed = true;
                }

                // An A_Restart / master reset from ETS ends the programming session
                // (knx-rs drops the connection and raises this flag). Persist the
                // freshly-downloaded memory immediately — matching KNX's
                // persist-before-restart ordering — instead of waiting for the
                // debounce, and cancel the now-redundant pending persist. This
                // closes the window where a crash between the last write and the
                // debounce firing would lose the programming.
                if bau.take_restart_pending() {
                    persist_armed = false;
                    memory_dirty = false;
                    tunnel_active = false;
                    // Persist SYNCHRONOUSLY here: if we restart below, the re-exec
                    // abandons any spawned task, so the write must complete first.
                    if let Some(path) = persist_path.as_ref() {
                        if !bau.memory_area().is_empty() {
                            let state = bau.save();
                            if let Err(e) = persist_memory(path, &state) {
                                tracing::warn!(error = %e, "Failed to persist ETS state before restart");
                            } else {
                                tracing::info!(bytes = state.len(), "ETS programming persisted before restart");
                            }
                        }
                    }
                    if restart_after_ets {
                        // Reboot into the new programming — the boot-time config
                        // path (KEA-0004) then applies the ETS parameters.
                        tracing::info!("ETS programming complete — restarting snapdog to apply parameters");
                        crate::process::restart_process(); // does not return on success
                    } else {
                        tracing::info!(
                            "ETS programming complete — restart snapdog to apply the new parameters"
                        );
                    }
                }

                dispatch_updated_gos(&mut bau, &update_tx).await;
            }

            cmd = cmd_rx.recv() => {
                let Some(cmd) = cmd else { break };
                match cmd {
                    BauCmd::Write { ga, dpt, value } => {
                        handle_write(&mut bau, &server, ga, dpt, &value).await;
                    }
                    BauCmd::SetProgMode { enabled } => {
                        knx_rs_device::device_object::set_prog_mode(bau.device_mut(), enabled);
                        tracing::info!(enabled, "KNX programming mode changed");
                    }
                    BauCmd::GetProgMode { reply } => {
                        let enabled = knx_rs_device::device_object::prog_mode(bau.device());
                        let _ = reply.send(enabled);
                    }
                }
            }
            // Debounced persist timer
            () = &mut persist_timer, if persist_armed => {
                persist_armed = false;
                memory_dirty = false;
                tunnel_active = false;
                if let Some(ref path) = persist_path {
                    spawn_persist(bau.save(), path.clone());
                }
            }
        }
    }

    // Persist on shutdown if dirty
    if memory_dirty {
        if let Some(ref path) = persist_path {
            let state = bau.save();
            if let Err(e) = persist_memory(path, &state) {
                tracing::warn!(error = %e, "Failed to persist ETS state on shutdown");
            } else {
                tracing::info!(path = %path.display(), "ETS state persisted on shutdown");
            }
        }
    }

    tracing::info!("KNX BAU task ended");
}

/// Persist a saved BAU memory snapshot to `path` in a blocking task, logging the
/// outcome. Shared by the debounced-persist and restart-persist paths.
fn spawn_persist(state: Vec<u8>, path: PathBuf) {
    tokio::task::spawn_blocking(move || match persist_memory(&path, &state) {
        Ok(()) => {
            tracing::info!(path = %path.display(), bytes = state.len(), "ETS programming persisted");
        }
        Err(e) => tracing::warn!(error = %e, "Failed to persist ETS state"),
    });
}

/// Load and parse persisted ETS device memory into [`EtsParams`], or `None` if the
/// file is absent/unreadable or the device was never fully programmed.
///
/// Reads the same [`ETS_MEMORY_PATH`] the BAU task persists to, and builds a
/// throwaway BAU purely to restore the memory image and parse it — no
/// `DeviceServer` and no UDP bind — so it is safe to call at boot **before** the
/// real KNX device transport starts (KEA-0004 boot-time apply).
pub(super) fn load_persisted_ets_params(ia: IndividualAddress) -> Option<EtsParams> {
    let data = load_memory(std::path::Path::new(ETS_MEMORY_PATH)).ok()?;
    let device = device_object::new_device_object(device_serial(), group_objects::HARDWARE_TYPE);
    let mut bau = Bau::new(device, TOTAL_GO_COUNT as u16, MAX_TUNNEL_CONNECTIONS);
    device_object::set_individual_address(bau.device_mut(), ia.raw());
    if bau.restore(&data).is_err() || !bau.configured() {
        return None;
    }
    Some(parse_ets_memory(bau.memory_area()))
}

/// Build a BAU with 460 group objects configured with correct DPTs,
/// and address/association tables from TOML config.
fn build_bau(ia: IndividualAddress, config: &crate::config::AppConfig) -> Bau {
    let device = device_object::new_device_object(
        device_serial(),
        group_objects::HARDWARE_TYPE, // must match the .knxprod LdCtrlCompareProp (PID 78)
    );
    let mut bau = Bau::new(device, TOTAL_GO_COUNT as u16, MAX_TUNNEL_CONNECTIONS);
    device_object::set_individual_address(bau.device_mut(), ia.raw());

    // Configure zone GOs with DPTs
    for zone in 1..=MAX_ZONES {
        for (i, go_def) in ZONE_GOS.iter().enumerate() {
            let asap = group_objects::zone_asap(zone, i);
            if let Some(go) = bau.group_objects_mut().get_mut(asap) {
                go.set_dpt(go_def.dpt);
            }
        }
    }

    // Configure client GOs with DPTs
    for client in 1..=MAX_CLIENTS {
        for (i, go_def) in CLIENT_GOS.iter().enumerate() {
            let asap = group_objects::client_asap(client, i);
            if let Some(go) = bau.group_objects_mut().get_mut(asap) {
                go.set_dpt(go_def.dpt);
            }
        }
    }

    // Build address table and association table from TOML KNX config
    build_tables_from_config(&mut bau, config);

    bau
}

/// Build address and association tables from TOML zone/client KNX addresses.
#[allow(clippy::too_many_lines)] // Table construction from config — linear data flow
fn build_tables_from_config(bau: &mut Bau, config: &crate::config::AppConfig) {
    // Collect all (GA string, ASAP) pairs
    let mut ga_asap_pairs: Vec<(u16, u16)> = Vec::new();

    for zone_cfg in &config.zones {
        let idx = zone_cfg.index;
        let knx = &zone_cfg.knx;
        // Map each configured GA to its zone GO ASAP
        let zone_gas: &[(&Option<String>, usize)] = &[
            (&knx.play, ZGO_PLAY),
            (&knx.pause, ZGO_PAUSE),
            (&knx.stop, ZGO_STOP),
            (&knx.track_next, ZGO_TRACK_NEXT),
            (&knx.track_previous, ZGO_TRACK_PREVIOUS),
            (&knx.volume, ZGO_VOLUME),
            (&knx.volume_status, ZGO_VOLUME_STATUS),
            (&knx.volume_dim, ZGO_VOLUME_DIM),
            (&knx.mute, ZGO_MUTE),
            (&knx.mute_status, ZGO_MUTE_STATUS),
            (&knx.mute_toggle, ZGO_MUTE_TOGGLE),
            (&knx.control_status, ZGO_CONTROL_STATUS),
            (&knx.track_playing_status, ZGO_TRACK_PLAYING),
            (&knx.shuffle, ZGO_SHUFFLE),
            (&knx.shuffle_status, ZGO_SHUFFLE_STATUS),
            (&knx.shuffle_toggle, ZGO_SHUFFLE_TOGGLE),
            (&knx.repeat, ZGO_REPEAT),
            (&knx.repeat_status, ZGO_REPEAT_STATUS),
            (&knx.repeat_toggle, ZGO_REPEAT_TOGGLE),
            (&knx.track_repeat, ZGO_TRACK_REPEAT),
            (&knx.track_repeat_status, ZGO_TRACK_REPEAT_STATUS),
            (&knx.track_repeat_toggle, ZGO_TRACK_REPEAT_TOGGLE),
            (&knx.playlist, ZGO_PLAYLIST),
            (&knx.playlist_status, ZGO_PLAYLIST_STATUS),
            (&knx.playlist_next, ZGO_PLAYLIST_NEXT),
            (&knx.playlist_previous, ZGO_PLAYLIST_PREVIOUS),
            (&knx.track_title_status, ZGO_TRACK_TITLE),
            (&knx.track_artist_status, ZGO_TRACK_ARTIST),
            (&knx.track_album_status, ZGO_TRACK_ALBUM),
            (&knx.track_progress_status, ZGO_TRACK_PROGRESS),
            (&knx.presence, ZGO_PRESENCE),
            (&knx.presence_enable, ZGO_PRESENCE_ENABLE),
            (&knx.presence_enable_status, ZGO_PRESENCE_ENABLE),
            (&knx.presence_timeout, ZGO_PRESENCE_TIMEOUT),
            (&knx.presence_timeout_status, ZGO_PRESENCE_TIMEOUT),
            (&knx.presence_timer_status, ZGO_PRESENCE_TIMER_ACTIVE),
            (&knx.presence_source_override, ZGO_PRESENCE_SOURCE_OVERRIDE),
        ];
        for (ga_opt, go_idx) in zone_gas {
            if let Some(ga_str) = ga_opt {
                if let Ok(ga) = GroupAddress::from_str(ga_str) {
                    let asap = group_objects::zone_asap(idx, *go_idx);
                    ga_asap_pairs.push((ga.raw(), asap));
                }
            }
        }
    }

    for client_cfg in &config.clients {
        let idx = client_cfg.index;
        let knx = &client_cfg.knx;
        let client_gas: &[(&Option<String>, usize)] = &[
            (&knx.volume, CGO_VOLUME),
            (&knx.volume_status, CGO_VOLUME_STATUS),
            (&knx.volume_dim, CGO_VOLUME_DIM),
            (&knx.mute, CGO_MUTE),
            (&knx.mute_status, CGO_MUTE_STATUS),
            (&knx.mute_toggle, CGO_MUTE_TOGGLE),
            (&knx.latency, CGO_LATENCY),
            (&knx.latency_status, CGO_LATENCY_STATUS),
            (&knx.zone, CGO_ZONE),
            (&knx.zone_status, CGO_ZONE_STATUS),
            (&knx.connected_status, CGO_CONNECTED),
        ];
        for (ga_opt, go_idx) in client_gas {
            if let Some(ga_str) = ga_opt {
                if let Ok(ga) = GroupAddress::from_str(ga_str) {
                    let asap = group_objects::client_asap(idx, *go_idx);
                    ga_asap_pairs.push((ga.raw(), asap));
                }
            }
        }
    }

    if ga_asap_pairs.is_empty() {
        return;
    }

    // Build address table: unique GAs → TSAPs (1-based)
    let mut unique_gas: Vec<u16> = ga_asap_pairs.iter().map(|(ga, _)| *ga).collect();
    unique_gas.sort_unstable();
    unique_gas.dedup();

    let mut addr_data = Vec::new();
    let count = unique_gas.len() as u16;
    addr_data.extend_from_slice(&count.to_be_bytes());
    for ga in &unique_gas {
        addr_data.extend_from_slice(&ga.to_be_bytes());
    }
    bau.address_table_mut().load(&addr_data);

    // Build association table: (TSAP, ASAP) pairs
    let mut assoc_data = Vec::new();
    let mut assoc_entries: Vec<(u16, u16)> = Vec::new();
    for (ga, asap) in &ga_asap_pairs {
        let tsap = unique_gas
            .iter()
            .position(|g| g == ga)
            .map(|i| (i + 1) as u16);
        if let Some(tsap) = tsap {
            assoc_entries.push((tsap, *asap));
        }
    }
    let assoc_count = assoc_entries.len() as u16;
    assoc_data.extend_from_slice(&assoc_count.to_be_bytes());
    for (tsap, asap) in &assoc_entries {
        assoc_data.extend_from_slice(&tsap.to_be_bytes());
        assoc_data.extend_from_slice(&asap.to_be_bytes());
    }
    bau.association_table_mut().load(&assoc_data);

    tracing::info!(
        gas = unique_gas.len(),
        associations = assoc_entries.len(),
        "KNX address/association tables loaded from TOML"
    );
}

/// Handle a write from the publisher: encode value, write to GO, poll BAU, send frames.
async fn handle_write(
    bau: &mut Bau,
    server: &DeviceServer,
    ga: GroupAddress,
    _dpt: Dpt,
    value: &DptValue,
) {
    // Find the ASAP for this GA via the address table
    let ga_raw = ga.raw();
    let Some(tsap) = bau.address_table().get_tsap(ga_raw) else {
        // GA not in address table — can't map to a GO
        tracing::trace!(ga = %ga, "GA not in address table, skipping write");
        return;
    };

    // Find associated GO and write value
    for asap in bau.association_table().asaps_for_tsap(tsap) {
        if let Some(go) = bau.group_objects_mut().get_mut(asap) {
            let _ = go.set_value_if_changed(value);
        }
    }

    // Poll to send pending writes
    bau.poll(now_ms());
    while let Some(out) = bau.next_outgoing_frame() {
        let _ = server.send_frame(out).await;
    }
}

/// Check all GOs for Updated flag and forward to the listener channel.
async fn dispatch_updated_gos(bau: &mut Bau, update_tx: &mpsc::Sender<(GroupAddress, Vec<u8>)>) {
    while let Some(asap) = bau.group_objects_mut().next_updated() {
        if let Some(result) = resolve_go_update(bau, asap) {
            let _ = update_tx.send(result).await;
        }
        // Acknowledge the update
        if let Some(go) = bau.group_objects_mut().get_mut(asap) {
            go.set_comm_flag(knx_rs_device::group_object::ComFlag::Ok);
        }
    }
}

/// Find the next updated GO and return its GA + data.
/// Resolve an ASAP to (`GroupAddress`, data) via the association and address tables.
fn resolve_go_update(bau: &Bau, asap: u16) -> Option<(GroupAddress, Vec<u8>)> {
    let go = bau.group_objects().get(asap)?;
    let data = go.value_ref().to_vec();
    let tsap = bau.association_table().translate_asap(asap)?;
    let ga_raw = bau.address_table().get_group_address(tsap)?;
    Some((GroupAddress::from_raw(ga_raw), data))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_ets_memory_defaults() {
        let data = vec![0u8; mem::TOTAL];
        let p = parse_ets_memory(&data);
        assert!(!p.zone_active[0]);
        assert_eq!(p.zone_max_volume[0], 0);
        assert!(!p.client_active[0]);
    }

    #[test]
    fn parse_ets_memory_values() {
        let mut data = vec![0u8; mem::TOTAL];
        data[mem::ZONE_ACTIVE] = 1;
        data[mem::ZONE_ACTIVE + 2] = 1;
        data[mem::ZONE_MAX_VOL] = 80;
        data[mem::CLIENT_ACTIVE] = 1;
        data[mem::CLIENT_MAX_VOL] = 60;
        data[mem::CLIENT_DEF_ZONE] = 3;
        data[mem::CLIENT_DEF_LAT] = 50;
        let p = parse_ets_memory(&data);
        assert!(p.zone_active[0]);
        assert!(!p.zone_active[1]);
        assert!(p.zone_active[2]);
        assert_eq!(p.zone_max_volume[0], 80);
        assert!(p.client_active[0]);
        assert_eq!(p.client_max_volume[0], 60);
        assert_eq!(p.client_default_zone[0], 3);
        assert_eq!(p.client_default_latency[0], 50);
    }

    #[test]
    fn parse_ets_memory_too_short() {
        let data = vec![0u8; 10]; // too short
        let p = parse_ets_memory(&data);
        // Should return defaults without panic
        assert!(!p.zone_active[0]);
    }

    #[test]
    fn build_tables_from_minimal_config() {
        let raw: crate::config::FileConfig = toml::from_str(
            r#"
            [[zone]]
            name = "Test"
            [zone.knx]
            play = "1/0/1"
            volume = "1/0/2"
            volume_status = "1/0/3"

            [[client]]
            name = "Speaker"
            mac = "00:00:00:00:00:00"
            zone = "Test"
            [client.knx]
            volume = "2/0/1"
            mute = "2/0/2"
        "#,
        )
        .unwrap();
        let config = crate::config::load_raw(raw).unwrap();
        let ia = IndividualAddress::from_raw(0x1101);
        let bau = build_bau(ia, &config);

        // Should have 5 unique GAs
        assert_eq!(bau.address_table().entry_count(), 5);

        // Zone 1 Play (ASAP 1) should be mapped to GA 1/0/1
        let tsap = bau
            .address_table()
            .get_tsap(GroupAddress::from_str("1/0/1").unwrap().raw());
        assert!(tsap.is_some());

        // Client 1 Volume (ASAP 351) should be mapped to GA 2/0/1
        let tsap = bau
            .address_table()
            .get_tsap(GroupAddress::from_str("2/0/1").unwrap().raw());
        assert!(tsap.is_some());
    }

    #[test]
    fn persist_roundtrip() {
        let dir = std::env::temp_dir().join("snapdog_test_persist");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("test.bin");

        let payload = vec![0xDE, 0xAD, 0xBE, 0xEF, 0x42];
        super::persist_memory(&path, &payload).unwrap();
        let loaded = super::load_memory(&path).unwrap();
        assert_eq!(loaded, payload);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn persist_detects_corruption() {
        let dir = std::env::temp_dir().join("snapdog_test_corrupt");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("corrupt.bin");

        let payload = vec![1, 2, 3, 4, 5];
        super::persist_memory(&path, &payload).unwrap();

        // Flip a byte in the payload area
        let mut raw = std::fs::read(&path).unwrap();
        raw[super::PERSIST_HEADER] ^= 0xFF;
        std::fs::write(&path, &raw).unwrap();

        let err = super::load_memory(&path).unwrap_err();
        assert!(
            err.to_string().contains("CRC32 mismatch"),
            "expected CRC error, got: {err}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn persist_rejects_wrong_version() {
        let dir = std::env::temp_dir().join("snapdog_test_version");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("version.bin");

        super::persist_memory(&path, &[0; 10]).unwrap();

        let mut raw = std::fs::read(&path).unwrap();
        raw[4] = 99;
        std::fs::write(&path, &raw).unwrap();

        let err = super::load_memory(&path).unwrap_err();
        assert!(
            err.to_string().contains("unsupported version"),
            "expected version error, got: {err}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn persist_rejects_truncated_file() {
        let dir = std::env::temp_dir().join("snapdog_test_truncated");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("truncated.bin");

        std::fs::write(&path, b"SDKM").unwrap();
        let err = super::load_memory(&path).unwrap_err();
        assert!(
            err.to_string().contains("too short"),
            "expected too-short error, got: {err}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn persist_envelope_byte_layout() {
        // Pin the on-disk format (the IT-T43 §9.2 Bau.save drift risk):
        // [4 magic]["SDKM"][1 version][2 len LE][payload][4 crc32 LE].
        let dir = std::env::temp_dir().join("snapdog_test_envelope");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("envelope.bin");
        let payload = vec![0xDE_u8, 0xAD, 0xBE, 0xEF, 0x42];
        super::persist_memory(&path, &payload).unwrap();
        let raw = std::fs::read(&path).unwrap();
        let n = payload.len();

        assert_eq!(&raw[0..4], b"SDKM", "magic");
        assert_eq!(raw[4], super::PERSIST_VERSION, "version byte");
        assert_eq!(
            u16::from_le_bytes([raw[5], raw[6]]),
            n as u16,
            "length is little-endian u16"
        );
        assert_eq!(
            &raw[super::PERSIST_HEADER..super::PERSIST_HEADER + n],
            &payload[..],
            "payload verbatim"
        );
        let crc = u32::from_le_bytes([
            raw[super::PERSIST_HEADER + n],
            raw[super::PERSIST_HEADER + n + 1],
            raw[super::PERSIST_HEADER + n + 2],
            raw[super::PERSIST_HEADER + n + 3],
        ]);
        assert_eq!(
            crc,
            super::crc32(&payload),
            "trailing CRC32 is little-endian"
        );
        assert_eq!(
            raw.len(),
            super::PERSIST_HEADER + n + 4,
            "total = 7-byte header + payload + 4-byte CRC"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolve_go_update_translates_asap_to_group_address() {
        let raw: crate::config::FileConfig = toml::from_str(
            r#"
            [[zone]]
            name = "Test"
            [zone.knx]
            play = "1/0/1"
            volume = "1/0/2"
            volume_status = "1/0/3"

            [[client]]
            name = "Speaker"
            mac = "00:00:00:00:00:00"
            zone = "Test"
            [client.knx]
            volume = "2/0/1"
            mute = "2/0/2"
        "#,
        )
        .unwrap();
        let config = crate::config::load_raw(raw).unwrap();
        let bau = build_bau(IndividualAddress::from_raw(0x1101), &config);

        // ASAP 1 = zone 1 Play → GA 1/0/1, via the asap→tsap→GA translation chain.
        let (ga, _data) = resolve_go_update(&bau, 1).expect("asap 1 resolves");
        assert_eq!(ga.raw(), GroupAddress::from_str("1/0/1").unwrap().raw());

        // A non-existent ASAP resolves to None.
        assert!(resolve_go_update(&bau, 60000).is_none());
    }
}
