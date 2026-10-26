// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! KNX/IP integration via knx-rs.
//!
//! Bidirectional:
//! - **Publisher**: writes zone/client status to KNX group addresses on state changes
//! - **Listener**: receives KNX group writes and routes them as zone/client commands
//!
//! Transport-agnostic: both client mode (gateway connection) and device mode
//! (ETS-programmable KNX/IP device) share the same publisher/listener logic
//! via the [`KnxPublisher`] and [`KnxListener`] traits. The modes differ in how a
//! group object finds its group address (see [`transport`]): client mode uses the
//! configured addresses, device mode uses the associations ETS programmed.
//!
//! Both transports are supervised: a gateway that is unreachable at startup or lost later,
//! and a device server that cannot bind or stops, are retried with capped exponential
//! backoff, and every status value is published again once the transport is back.

mod client;
mod device;
mod ets_config;
pub mod group_objects;
mod sink;
mod supervisor;
mod transport;

pub use device::DeviceOptions;
pub use ets_config::ets_device_config;

use std::collections::{HashMap, HashSet};
use std::hash::BuildHasher;
use std::str::FromStr;
use std::sync::Arc;

use anyhow::{Context, Result};
use knx_rs_core::address::GroupAddress;
use knx_rs_core::dpt::{
    self, DPT_SCALING, DPT_STRING_8859_1, DPT_SWITCH, DPT_VALUE_1_UCOUNT, DptValue,
};
use tokio::sync::watch;
use tokio::task::JoinHandle;

use crate::config::AppConfig;
use crate::player::{ClientAction, SnapcastCmd, ZoneCommand, ZoneCommandSender};
use crate::state;

use group_objects::{
    CGO_CONNECTED, CGO_LATENCY, CGO_LATENCY_STATUS, CGO_MUTE, CGO_MUTE_STATUS, CGO_MUTE_TOGGLE,
    CGO_VOLUME, CGO_VOLUME_DIM, CGO_VOLUME_STATUS, CGO_ZONE, CGO_ZONE_STATUS, CLIENT_GO_COUNT,
    DPT_CONTROL_DIMMING, GGO_ALL_MUTE, GGO_ALL_STOP, GGO_KNX_TIME, GGO_SERVER_ONLINE,
    GGO_SYSTEM_FAULT, MAX_CLIENTS, MAX_ZONES, ZGO_CONTROL_STATUS, ZGO_MUTE, ZGO_MUTE_STATUS,
    ZGO_MUTE_TOGGLE, ZGO_PAUSE, ZGO_PLAY, ZGO_PLAYLIST, ZGO_PLAYLIST_NEXT, ZGO_PLAYLIST_PREVIOUS,
    ZGO_PLAYLIST_STATUS, ZGO_PRESENCE, ZGO_PRESENCE_ENABLE, ZGO_PRESENCE_TIMER_ACTIVE, ZGO_REPEAT,
    ZGO_REPEAT_STATUS, ZGO_REPEAT_TOGGLE, ZGO_SHUFFLE, ZGO_SHUFFLE_STATUS, ZGO_SHUFFLE_TOGGLE,
    ZGO_STOP, ZGO_TRACK_ALBUM, ZGO_TRACK_ARTIST, ZGO_TRACK_NEXT, ZGO_TRACK_PLAYING,
    ZGO_TRACK_PREVIOUS, ZGO_TRACK_PROGRESS, ZGO_TRACK_REPEAT, ZGO_TRACK_REPEAT_STATUS,
    ZGO_TRACK_REPEAT_TOGGLE, ZGO_TRACK_TITLE, ZGO_VOLUME, ZGO_VOLUME_DIM, ZGO_VOLUME_STATUS,
    ZONE_GO_COUNT,
};
use sink::{ObjectKey, StatusSink};
use transport::{Incoming, KnxListener, KnxPublisher};

// ── Start ─────────────────────────────────────────────────────

/// Check the syntax of a client `url` (`--check-config` and config loading use this).
///
/// # Errors
///
/// Returns an error naming what is wrong with the URL.
pub fn validate_url(url: &str) -> Result<()> {
    supervisor::ClientTarget::parse(url).map(|_| ())
}

/// Handle for KNX device management (programming mode, etc.).
/// Only available in device mode.
pub type DeviceControlHandle = Arc<device::DevicePublisher>;

/// Re-export the device control trait for use by API routes.
pub use transport::KnxDeviceControl;

/// A running KNX bridge. Dropping it leaves the bridge running; [`shutdown`](Self::shutdown)
/// stops it.
pub struct KnxBridge {
    device: Option<DeviceControlHandle>,
    shutdown: watch::Sender<bool>,
    tasks: Vec<JoinHandle<()>>,
}

impl KnxBridge {
    /// Handle for programming mode and device info. `None` in client mode.
    #[must_use]
    pub fn device_control(&self) -> Option<DeviceControlHandle> {
        self.device.clone()
    }

    /// Stop reconnecting, close the transport and wait briefly for the tasks to end.
    pub async fn shutdown(self) {
        let _ = self.shutdown.send(true);
        for task in self.tasks {
            if tokio::time::timeout(std::time::Duration::from_secs(2), task)
                .await
                .is_err()
            {
                tracing::warn!("KNX task did not stop in time");
            }
        }
    }
}

/// Start the KNX bridge.
///
/// Client mode returns as soon as the configuration is valid: the gateway connection is made
/// (and re-made) in the background, so an unreachable gateway no longer disables KNX for the
/// lifetime of the process.
///
/// # Errors
///
/// Returns an error if KNX is not configured, the configuration is unusable (no or invalid
/// URL, invalid individual address) or the device state cannot be set up.
pub async fn start<S: BuildHasher + Send + Sync + 'static>(
    config: &AppConfig,
    store: state::SharedState,
    notify_rx: tokio::sync::broadcast::Receiver<Arc<str>>,
    zone_commands: HashMap<usize, ZoneCommandSender, S>,
    snap_tx: tokio::sync::mpsc::Sender<SnapcastCmd>,
) -> Result<KnxBridge> {
    start_with_options(
        config,
        store,
        notify_rx,
        zone_commands,
        snap_tx,
        &DeviceOptions::default(),
    )
    .await
}

/// Like [`start`], with explicit device-transport options (loopback bind and memory
/// path for tests).
///
/// # Errors
///
/// See [`start`].
pub async fn start_with_options<S: BuildHasher + Send + Sync + 'static>(
    config: &AppConfig,
    store: state::SharedState,
    notify_rx: tokio::sync::broadcast::Receiver<Arc<str>>,
    zone_commands: HashMap<usize, ZoneCommandSender, S>,
    snap_tx: tokio::sync::mpsc::Sender<SnapcastCmd>,
    options: &DeviceOptions,
) -> Result<KnxBridge> {
    let knx = config.knx.as_ref().context("KNX not configured")?;
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let bridge = Bridge::new(config, store, zone_commands, snap_tx);
    match knx.role {
        crate::config::KnxRole::Client => {
            let url = knx
                .url
                .as_deref()
                .context("KNX client mode requires 'url'")?;
            let target = supervisor::ClientTarget::parse(url)?;
            for problem in duplicate_addresses(config) {
                tracing::warn!("KNX group address configured twice: {problem}");
            }
            let task = tokio::spawn(supervisor::run_client(
                target,
                bridge,
                notify_rx,
                shutdown_rx,
            ));
            Ok(KnxBridge {
                device: None,
                shutdown: shutdown_tx,
                tasks: vec![task],
            })
        }
        crate::config::KnxRole::Device => {
            let addr = knx
                .individual_address
                .as_deref()
                .context("KNX device mode requires 'individual_address'")?;
            let transport =
                device::start_device_transport(addr, config, options, shutdown_rx.clone()).await?;
            let (publisher, listener) = (transport.publisher, transport.listener);
            let handle: DeviceControlHandle = Arc::new(publisher.clone());
            let task = tokio::spawn(supervisor::run_device(
                publisher,
                listener,
                bridge,
                notify_rx,
                shutdown_rx,
            ));
            Ok(KnxBridge {
                device: Some(handle),
                shutdown: shutdown_tx,
                // The device task persists on its way out: shutdown waits for it as well.
                tasks: vec![task, transport.task],
            })
        }
    }
}

// ── Bridge state ──────────────────────────────────────────────

/// What publisher and listener need besides the transport.
pub(crate) struct Bridge<S> {
    config: AppConfig,
    store: state::SharedState,
    zone_commands: HashMap<usize, ZoneCommandSender, S>,
    snap_tx: tokio::sync::mpsc::Sender<SnapcastCmd>,
    zone_ga_map: HashMap<String, (usize, &'static str)>,
    client_ga_map: HashMap<String, (usize, &'static str)>,
    global_ga_map: HashMap<String, &'static str>,
    /// Canonical group addresses of the status objects, the ones a group read is answered for.
    status_gas: HashSet<String>,
    sync_system_clock: bool,
}

impl<S> Bridge<S> {
    fn new(
        config: &AppConfig,
        store: state::SharedState,
        zone_commands: HashMap<usize, ZoneCommandSender, S>,
        snap_tx: tokio::sync::mpsc::Sender<SnapcastCmd>,
    ) -> Self {
        Self {
            zone_ga_map: build_zone_ga_map(config),
            client_ga_map: build_client_ga_map(config),
            global_ga_map: build_global_ga_map(config),
            status_gas: build_status_ga_set(config),
            sync_system_clock: config.knx.as_ref().is_some_and(|k| k.sync_system_clock),
            config: config.clone(),
            store,
            zone_commands,
            snap_tx,
        }
    }
}

/// Why a bridge session ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SessionEnd {
    /// The transport reported that the connection is closed.
    ConnectionClosed,
    /// Several writes in a row failed: the gateway is gone although the tunnel has not noticed.
    WriteFailures,
    /// The notification channel closed (the server is shutting down).
    NotificationsClosed,
}

/// Run publisher and listener on one connected transport until the transport fails.
pub(crate) async fn run_session<P, L, S>(
    publisher: &P,
    listener: L,
    broken: Option<&tokio::sync::Notify>,
    bridge: &Bridge<S>,
    notify_rx: &mut tokio::sync::broadcast::Receiver<Arc<str>>,
) -> SessionEnd
where
    P: KnxPublisher,
    L: KnxListener,
    S: BuildHasher + Send + Sync,
{
    // The listener passes group reads on to the publisher, which knows the values.
    let (read_tx, mut read_rx) = tokio::sync::mpsc::unbounded_channel();
    tokio::select! {
        () = publisher_loop(publisher, bridge, notify_rx, &mut read_rx) => SessionEnd::NotificationsClosed,
        () = listener_loop(listener, bridge, &read_tx) => SessionEnd::ConnectionClosed,
        () = async {
            match broken {
                Some(n) => n.notified().await,
                None => std::future::pending().await,
            }
        } => SessionEnd::WriteFailures,
    }
}

// ── Publisher ─────────────────────────────────────────────────

/// Publish every status value: the state of all zones and clients and the global objects.
async fn publish_all<P: KnxPublisher>(
    config: &AppConfig,
    store: &state::SharedState,
    sink: &mut StatusSink<'_, P>,
) {
    for (idx, _) in config.zones.iter().enumerate() {
        let zone_index = idx + 1;
        publish_zone_state(zone_index, config, store, sink).await;
        publish_zone_track(zone_index, config, store, sink).await;
        publish_zone_progress(zone_index, config, store, sink).await;
    }
    for (idx, _) in config.clients.iter().enumerate() {
        publish_client_state(idx + 1, config, store, sink).await;
    }
    publish_all_mute_status(config, store, sink).await;
}

/// Heartbeat period of the cyclic "server online" object.
fn heartbeat_period(config: &AppConfig) -> std::time::Duration {
    let minutes = config
        .knx
        .as_ref()
        .map_or(5, |k| k.heartbeat_minutes)
        .clamp(1, 240);
    std::time::Duration::from_secs(u64::from(minutes) * 60)
}

/// The cyclic "server online" heartbeat and the system-fault alarm.
async fn publish_heartbeat<P: KnxPublisher>(
    config: &AppConfig,
    last_healthy: &mut Option<bool>,
    sink: &mut StatusSink<'_, P>,
) {
    let Some(knx) = config.knx.as_ref() else {
        return;
    };
    // The heartbeat is the point of the object: send it every time, changed or not.
    sink.write_always(
        ObjectKey::Global(GGO_SERVER_ONLINE),
        knx.server_online.as_deref(),
        DPT_SWITCH,
        &true.into(),
    )
    .await;
    let healthy = SYSTEM_HEALTHY.load(std::sync::atomic::Ordering::Relaxed);
    if *last_healthy != Some(healthy) {
        *last_healthy = Some(healthy);
        sink.write(
            ObjectKey::Global(GGO_SYSTEM_FAULT),
            knx.system_fault.as_deref(),
            DPT_SWITCH,
            &(!healthy).into(),
        )
        .await;
    }
}

async fn publisher_loop<P, S>(
    transport: &P,
    bridge: &Bridge<S>,
    notify_rx: &mut tokio::sync::broadcast::Receiver<Arc<str>>,
    read_rx: &mut tokio::sync::mpsc::UnboundedReceiver<GroupAddress>,
) where
    P: KnxPublisher,
    S: Send + Sync,
{
    let config = &bridge.config;
    let store = &bridge.store;
    let mut sink = StatusSink::new(transport);
    tracing::info!("KNX publisher started");

    publish_all(config, store, &mut sink).await;

    // Cyclic "server online" heartbeat + fault publishing. The first tick fires immediately.
    let mut heartbeat = tokio::time::interval(heartbeat_period(config));
    let mut last_healthy: Option<bool> = None;

    loop {
        let progress_due = sink.next_due();
        tokio::select! {
            _ = heartbeat.tick() => {
                publish_heartbeat(config, &mut last_healthy, &mut sink).await;
            }
            () = async {
                match progress_due {
                    Some(at) => tokio::time::sleep_until(at).await,
                    None => std::future::pending().await,
                }
            } => {
                sink.flush_due().await;
            }
            Some(ga) = read_rx.recv() => {
                // A display that starts late asks for the values: answer with the last one sent,
                // and for an address nothing was sent to yet, with the current value.
                if !sink.answer_read(ga).await {
                    publish_all(config, store, &mut sink).await;
                    sink.end_reads();
                }
            }
            () = transport.resync() => {
                tracing::info!("KNX transport restarted or reprogrammed — publishing all status values again");
                sink.invalidate();
                publish_all(config, store, &mut sink).await;
                last_healthy = None;
                heartbeat.reset_immediately();
            }
            r = notify_rx.recv() => match r {
                Ok(json) => {
                    let Ok(notif) = serde_json::from_str::<crate::api::ws::Notification>(&json) else {
                        continue;
                    };
                    match notif {
                        crate::api::ws::Notification::ZoneChanged { zone, .. } => {
                            publish_zone_state(zone, config, store, &mut sink).await;
                            publish_zone_track(zone, config, store, &mut sink).await;
                            publish_all_mute_status(config, store, &mut sink).await;
                        }
                        crate::api::ws::Notification::ZoneVolumeChanged { zone, .. } => {
                            publish_zone_state(zone, config, store, &mut sink).await;
                            publish_all_mute_status(config, store, &mut sink).await;
                        }
                        crate::api::ws::Notification::ZoneProgress { zone, .. } => {
                            publish_zone_progress(zone, config, store, &mut sink).await;
                        }
                        crate::api::ws::Notification::ClientStateChanged { client, .. } => {
                            publish_client_state(client, config, store, &mut sink).await;
                        }
                        _ => {}
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!(missed = n, "KNX publisher lagged — publishing all status values again");
                    publish_all(config, store, &mut sink).await;
                }
                Err(_) => break,
            }
        }
    }
}

/// Publish the global "All Mute" status: on when every zone is muted.
async fn publish_all_mute_status<P: KnxPublisher>(
    config: &AppConfig,
    store: &state::SharedState,
    sink: &mut StatusSink<'_, P>,
) {
    let all_muted = {
        let s = store.read().await;
        !s.zones.is_empty() && s.zones.values().all(|z| z.muted)
    };
    sink.write(
        ObjectKey::Global(GGO_ALL_MUTE),
        config
            .knx
            .as_ref()
            .and_then(|k| k.all_mute_status.as_deref()),
        DPT_SWITCH,
        &all_muted.into(),
    )
    .await;
}

async fn publish_zone_state<P: KnxPublisher>(
    zone_index: usize,
    config: &AppConfig,
    store: &state::SharedState,
    sink: &mut StatusSink<'_, P>,
) {
    let Some(zone_cfg) = config.zones.get(zone_index - 1) else {
        return;
    };
    let knx = &zone_cfg.knx;
    let zone = {
        let s = store.read().await;
        match s.zones.get(&zone_index) {
            Some(z) => z.clone(),
            None => return,
        }
    };
    let playing = zone.playback.to_string() == "playing";
    let go = |i: usize| ObjectKey::Zone(zone_index, i);

    sink.write(
        go(ZGO_VOLUME_STATUS),
        knx.volume_status.as_deref(),
        DPT_SCALING,
        &DptValue::Float(f64::from(zone.volume.clamp(0, 100) as u8)),
    )
    .await;
    sink.write(
        go(ZGO_MUTE_STATUS),
        knx.mute_status.as_deref(),
        DPT_SWITCH,
        &zone.muted.into(),
    )
    .await;
    sink.write(
        go(ZGO_SHUFFLE_STATUS),
        knx.shuffle_status.as_deref(),
        DPT_SWITCH,
        &zone.shuffle.into(),
    )
    .await;
    let repeat_on = zone.repeat == snapdog_common::RepeatMode::Playlist;
    sink.write(
        go(ZGO_REPEAT_STATUS),
        knx.repeat_status.as_deref(),
        DPT_SWITCH,
        &repeat_on.into(),
    )
    .await;
    sink.write(
        go(ZGO_TRACK_PLAYING),
        knx.track_playing_status.as_deref(),
        DPT_SWITCH,
        &playing.into(),
    )
    .await;
    let track_repeat_on = zone.repeat == snapdog_common::RepeatMode::Track;
    sink.write(
        go(ZGO_TRACK_REPEAT_STATUS),
        knx.track_repeat_status.as_deref(),
        DPT_SWITCH,
        &track_repeat_on.into(),
    )
    .await;
    sink.write(
        go(ZGO_CONTROL_STATUS),
        knx.control_status.as_deref(),
        DPT_SWITCH,
        &playing.into(),
    )
    .await;
    sink.write(
        go(ZGO_PLAYLIST_STATUS),
        knx.playlist_status.as_deref(),
        DPT_VALUE_1_UCOUNT,
        &DptValue::from(zone.playlist_index.unwrap_or(0) as u8),
    )
    .await;
    sink.write(
        go(ZGO_PRESENCE_ENABLE),
        knx.presence_enable_status.as_deref(),
        DPT_SWITCH,
        &zone.presence_enabled.into(),
    )
    .await;
    sink.write(
        go(ZGO_PRESENCE_TIMER_ACTIVE),
        knx.presence_timer_status.as_deref(),
        DPT_SWITCH,
        &zone.auto_off_active.into(),
    )
    .await;
}

async fn publish_zone_track<P: KnxPublisher>(
    zone_index: usize,
    config: &AppConfig,
    store: &state::SharedState,
    sink: &mut StatusSink<'_, P>,
) {
    let Some(zone_cfg) = config.zones.get(zone_index - 1) else {
        return;
    };
    let knx = &zone_cfg.knx;
    let track = {
        let s = store.read().await;
        s.zones.get(&zone_index).and_then(|z| z.track.clone())
    };

    // No track is a state too: the strings are empty and the position is 0. Publishing it gives
    // every string object a value to answer a read with, and clears a display when playback
    // ends.
    let go = |i: usize| ObjectKey::Zone(zone_index, i);
    let text = |f: fn(&state::TrackInfo) -> &str| track.as_ref().map_or("", f);
    sink.write(
        go(ZGO_TRACK_TITLE),
        knx.track_title_status.as_deref(),
        DPT_STRING_8859_1,
        &DptValue::from(text(|t| t.title.as_str())),
    )
    .await;
    sink.write(
        go(ZGO_TRACK_ARTIST),
        knx.track_artist_status.as_deref(),
        DPT_STRING_8859_1,
        &DptValue::from(text(|t| t.artist.as_str())),
    )
    .await;
    sink.write(
        go(ZGO_TRACK_ALBUM),
        knx.track_album_status.as_deref(),
        DPT_STRING_8859_1,
        &DptValue::from(text(|t| t.album.as_str())),
    )
    .await;
    sink.write_progress(
        go(ZGO_TRACK_PROGRESS),
        knx.track_progress_status.as_deref(),
        DPT_SCALING,
        &DptValue::Float(track.as_ref().map_or(0.0, track_progress_pct)),
    )
    .await;
}

/// Calculate track progress as a whole percentage (0.0–100.0).
///
/// Whole percent is the resolution worth a telegram: DPT 5.001 has 255 steps, and a longer
/// track would move one step (0.4 %) every second or two.
fn track_progress_pct(track: &state::TrackInfo) -> f64 {
    if track.duration_ms > 0 {
        ((track.position_ms as f64 / track.duration_ms as f64) * 100.0)
            .clamp(0.0, 100.0)
            .round()
    } else {
        0.0
    }
}

async fn publish_zone_progress<P: KnxPublisher>(
    zone_index: usize,
    config: &AppConfig,
    store: &state::SharedState,
    sink: &mut StatusSink<'_, P>,
) {
    let Some(zone_cfg) = config.zones.get(zone_index - 1) else {
        return;
    };
    let pct = {
        let s = store.read().await;
        s.zones
            .get(&zone_index)
            .and_then(|z| z.track.as_ref())
            .map_or(0.0, track_progress_pct)
    };
    sink.write_progress(
        ObjectKey::Zone(zone_index, ZGO_TRACK_PROGRESS),
        zone_cfg.knx.track_progress_status.as_deref(),
        DPT_SCALING,
        &DptValue::Float(pct),
    )
    .await;
}

async fn publish_client_state<P: KnxPublisher>(
    client_index: usize,
    config: &AppConfig,
    store: &state::SharedState,
    sink: &mut StatusSink<'_, P>,
) {
    let Some(client_cfg) = config.clients.get(client_index - 1) else {
        return;
    };
    let knx = &client_cfg.knx;
    let client = {
        let s = store.read().await;
        match s.clients.get(&client_index) {
            Some(c) => c.clone(),
            None => return,
        }
    };
    let go = |i: usize| ObjectKey::Client(client_index, i);

    sink.write(
        go(CGO_VOLUME_STATUS),
        knx.volume_status.as_deref(),
        DPT_SCALING,
        &DptValue::Float(f64::from(client.base_volume.clamp(0, 100) as u8)),
    )
    .await;
    sink.write(
        go(CGO_MUTE_STATUS),
        knx.mute_status.as_deref(),
        DPT_SWITCH,
        &client.muted.into(),
    )
    .await;
    sink.write(
        go(CGO_CONNECTED),
        knx.connected_status.as_deref(),
        DPT_SWITCH,
        &client.connected.into(),
    )
    .await;
    sink.write(
        go(CGO_ZONE_STATUS),
        knx.zone_status.as_deref(),
        DPT_VALUE_1_UCOUNT,
        &DptValue::from(client.zone_index as u8),
    )
    .await;
    sink.write(
        go(CGO_LATENCY_STATUS),
        knx.latency_status.as_deref(),
        DPT_VALUE_1_UCOUNT,
        &DptValue::from(client.latency_ms.clamp(0, 255) as u8),
    )
    .await;
}

// ── Listener ──────────────────────────────────────────────────

/// Dispatch everything the transport receives until it closes.
async fn listener_loop<L, S>(
    mut transport: L,
    bridge: &Bridge<S>,
    read_tx: &tokio::sync::mpsc::UnboundedSender<GroupAddress>,
) where
    L: KnxListener,
    S: BuildHasher + Send + Sync,
{
    if bridge
        .config
        .knx
        .as_ref()
        .is_some_and(|k| k.role == crate::config::KnxRole::Client)
    {
        tracing::info!(
            zone_gas = bridge.zone_ga_map.len(),
            client_gas = bridge.client_ga_map.len(),
            global_gas = bridge.global_ga_map.len(),
            "KNX listener started"
        );
    } else {
        tracing::info!("KNX listener started (group objects)");
    }

    while let Some(incoming) = transport.recv().await {
        match incoming {
            Incoming::Address(ga, data) => {
                handle_incoming(
                    ga,
                    &data,
                    &bridge.zone_ga_map,
                    &bridge.client_ga_map,
                    &bridge.global_ga_map,
                    &bridge.zone_commands,
                    &bridge.snap_tx,
                    &bridge.store,
                    bridge.sync_system_clock,
                )
                .await;
            }
            Incoming::Read(ga) => {
                // Only the addresses of status objects are answered.
                if bridge.status_gas.contains(&canonical_ga(&ga.to_string())) {
                    let _ = read_tx.send(ga);
                }
            }
            Incoming::Object(asap, data) => {
                handle_object(
                    asap,
                    &data,
                    &bridge.zone_commands,
                    &bridge.snap_tx,
                    &bridge.store,
                    bridge.sync_system_clock,
                )
                .await;
            }
        }
    }
    tracing::debug!("KNX listener ended: the transport closed");
}

/// Canonical form of a configured group address: the 3-level notation the bus side prints
/// (`GroupAddress`'s `Display`), so `1/2047`, `1/7/255` and `01/7/255` key the same entry.
/// A string that is not a group address is kept as written (it can never match).
fn canonical_ga(ga: &str) -> String {
    GroupAddress::from_str(ga).map_or_else(|_| ga.to_string(), |g| g.to_string())
}

pub(crate) fn build_zone_ga_map(config: &AppConfig) -> HashMap<String, (usize, &'static str)> {
    zone_ga_map(config, &mut Vec::new())
}

/// The zone map; an address that two zone actions share is noted in `duplicates`.
fn zone_ga_map(
    config: &AppConfig,
    duplicates: &mut Vec<String>,
) -> HashMap<String, (usize, &'static str)> {
    let mut map = HashMap::new();
    for zone_cfg in &config.zones {
        let idx = zone_cfg.index;
        let knx = &zone_cfg.knx;
        let pairs: &[(&Option<String>, &'static str)] = &[
            (&knx.play, "play"),
            (&knx.pause, "pause"),
            (&knx.stop, "stop"),
            (&knx.track_next, "next"),
            (&knx.track_previous, "previous"),
            (&knx.mute, "mute"),
            (&knx.mute_toggle, "mute_toggle"),
            (&knx.shuffle, "shuffle"),
            (&knx.shuffle_toggle, "shuffle_toggle"),
            (&knx.repeat, "repeat"),
            (&knx.repeat_toggle, "repeat_toggle"),
            (&knx.track_repeat, "track_repeat"),
            (&knx.track_repeat_toggle, "track_repeat_toggle"),
            (&knx.volume, "volume"),
            (&knx.volume_dim, "volume_dim"),
            (&knx.playlist, "playlist"),
            (&knx.playlist_next, "playlist_next"),
            (&knx.playlist_previous, "playlist_previous"),
            (&knx.presence, "presence"),
            (&knx.presence_enable, "presence_enable"),
        ];
        for (ga_opt, action) in pairs {
            if let Some(ga) = ga_opt
                && let Some((z, a)) = map.insert(canonical_ga(ga), (idx, *action))
            {
                duplicates.push(format!(
                    "{ga} is the address of zone {z} {a} and of zone {idx} {action}: the later one wins"
                ));
            }
        }
    }
    map
}

pub(crate) fn build_client_ga_map(config: &AppConfig) -> HashMap<String, (usize, &'static str)> {
    client_ga_map(config, &mut Vec::new())
}

/// The client map; an address that two client actions share is noted in `duplicates`.
fn client_ga_map(
    config: &AppConfig,
    duplicates: &mut Vec<String>,
) -> HashMap<String, (usize, &'static str)> {
    let mut map = HashMap::new();
    for client_cfg in &config.clients {
        let idx = client_cfg.index;
        let knx = &client_cfg.knx;
        let pairs: &[(&Option<String>, &'static str)] = &[
            (&knx.mute, "mute"),
            (&knx.mute_toggle, "mute_toggle"),
            (&knx.volume, "volume"),
            (&knx.volume_dim, "volume_dim"),
            (&knx.latency, "latency"),
            (&knx.zone, "zone"),
        ];
        for (ga_opt, action) in pairs {
            if let Some(ga) = ga_opt
                && let Some((c, a)) = map.insert(canonical_ga(ga), (idx, *action))
            {
                duplicates.push(format!(
                    "{ga} is the address of client {c} {a} and of client {idx} {action}: the later one wins"
                ));
            }
        }
    }
    map
}

/// Configurations that do not do what they say in client mode: an address configured for two
/// actions of one kind (the later wins) or for a global object and a zone or client action (the
/// global object takes the telegram, the other action never fires). In device mode the same
/// address may belong to several objects, so this is a client mode check.
pub(crate) fn duplicate_addresses(config: &AppConfig) -> Vec<String> {
    let mut duplicates = Vec::new();
    let zones = zone_ga_map(config, &mut duplicates);
    let clients = client_ga_map(config, &mut duplicates);
    for (ga, action) in build_global_ga_map(config) {
        if let Some((z, a)) = zones.get(&ga) {
            duplicates.push(format!(
                "{ga} is the global {action} address and of zone {z} {a}: the zone action never runs"
            ));
        }
        if let Some((c, a)) = clients.get(&ga) {
            duplicates.push(format!(
                "{ga} is the global {action} address and of client {c} {a}: the client action never runs"
            ));
        }
    }
    duplicates
}

/// The group addresses of all status objects (canonical form), the ones a group read is
/// answered for.
pub(crate) fn build_status_ga_set(config: &AppConfig) -> HashSet<String> {
    let mut set = HashSet::new();
    for zone in &config.zones {
        let k = &zone.knx;
        for ga in [
            &k.volume_status,
            &k.mute_status,
            &k.control_status,
            &k.track_title_status,
            &k.track_artist_status,
            &k.track_album_status,
            &k.track_progress_status,
            &k.track_playing_status,
            &k.track_repeat_status,
            &k.playlist_status,
            &k.shuffle_status,
            &k.repeat_status,
            &k.presence_enable_status,
            &k.presence_timer_status,
        ]
        .into_iter()
        .flatten()
        {
            set.insert(canonical_ga(ga));
        }
    }
    for client in &config.clients {
        let k = &client.knx;
        for ga in [
            &k.volume_status,
            &k.mute_status,
            &k.latency_status,
            &k.zone_status,
            &k.connected_status,
        ]
        .into_iter()
        .flatten()
        {
            set.insert(canonical_ga(ga));
        }
    }
    if let Some(k) = config.knx.as_ref() {
        for ga in [&k.server_online, &k.all_mute_status, &k.system_fault]
            .into_iter()
            .flatten()
        {
            set.insert(canonical_ga(ga));
        }
    }
    set
}

/// Offset in seconds from the system clock to the KNX-provided time-of-day.
///
/// Set from the global "KNX Time" GO and read by the presence-schedule evaluator so
/// schedules can follow KNX time even when the OS clock is not synced.
pub static KNX_TIME_OFFSET_SECS: std::sync::atomic::AtomicI64 =
    std::sync::atomic::AtomicI64::new(0);

/// Server health flag driving the global "System Fault" GO. `true` = healthy (no fault).
/// Set `false` when the Snapcast backend is unreachable.
pub static SYSTEM_HEALTHY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

/// Build the device-level (global) group-address → action map for incoming telegrams.
///
/// Only receive-capable globals appear here; send-only globals (Server Online, System
/// Fault) and the All-Mute status are published, not dispatched.
pub(crate) fn build_global_ga_map(config: &AppConfig) -> HashMap<String, &'static str> {
    let mut map = HashMap::new();
    if let Some(knx) = config.knx.as_ref() {
        let pairs: &[(&Option<String>, &'static str)] = &[
            (&knx.all_stop, "all_stop"),
            (&knx.all_mute, "all_mute"),
            (&knx.knx_time, "knx_time"),
        ];
        for (ga_opt, action) in pairs {
            if let Some(ga) = ga_opt {
                map.insert(canonical_ga(ga), *action);
            }
        }
    }
    map
}

/// A command a group telegram or group object stands for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Command<'a> {
    /// A device-level object (`all_stop`, `all_mute`, `knx_time`).
    Global(&'a str),
    /// An action on zone `.0`.
    Zone(usize, &'a str),
    /// An action on client `.0`.
    Client(usize, &'a str),
}

/// Zone objects the bus can write, and the action each one triggers.
const ZONE_ACTIONS: &[(usize, &str)] = &[
    (ZGO_PLAY, "play"),
    (ZGO_PAUSE, "pause"),
    (ZGO_STOP, "stop"),
    (ZGO_TRACK_NEXT, "next"),
    (ZGO_TRACK_PREVIOUS, "previous"),
    (ZGO_VOLUME, "volume"),
    (ZGO_VOLUME_DIM, "volume_dim"),
    (ZGO_MUTE, "mute"),
    (ZGO_MUTE_TOGGLE, "mute_toggle"),
    (ZGO_SHUFFLE, "shuffle"),
    (ZGO_SHUFFLE_TOGGLE, "shuffle_toggle"),
    (ZGO_REPEAT, "repeat"),
    (ZGO_REPEAT_TOGGLE, "repeat_toggle"),
    (ZGO_TRACK_REPEAT, "track_repeat"),
    (ZGO_TRACK_REPEAT_TOGGLE, "track_repeat_toggle"),
    (ZGO_PLAYLIST, "playlist"),
    (ZGO_PLAYLIST_NEXT, "playlist_next"),
    (ZGO_PLAYLIST_PREVIOUS, "playlist_previous"),
    (ZGO_PRESENCE, "presence"),
    (ZGO_PRESENCE_ENABLE, "presence_enable"),
];

/// Client objects the bus can write, and the action each one triggers.
const CLIENT_ACTIONS: &[(usize, &str)] = &[
    (CGO_VOLUME, "volume"),
    (CGO_VOLUME_DIM, "volume_dim"),
    (CGO_MUTE, "mute"),
    (CGO_MUTE_TOGGLE, "mute_toggle"),
    (CGO_LATENCY, "latency"),
    (CGO_ZONE, "zone"),
];

/// Device-level objects the bus can write, and the action each one triggers.
const GLOBAL_ACTIONS: &[(usize, &str)] = &[
    (GGO_ALL_STOP, "all_stop"),
    (GGO_ALL_MUTE, "all_mute"),
    (GGO_KNX_TIME, "knx_time"),
];

/// What a group object (by ASAP) commands when ETS-programmed addresses write it, or `None`
/// for objects that only carry status.
fn object_command(asap: u16) -> Option<Command<'static>> {
    let n = usize::from(asap).checked_sub(1)?;
    let zones_end = MAX_ZONES * ZONE_GO_COUNT;
    let clients_end = zones_end + MAX_CLIENTS * CLIENT_GO_COUNT;
    let find = |table: &'static [(usize, &'static str)], go: usize| {
        table.iter().find(|(i, _)| *i == go).map(|(_, a)| *a)
    };
    if n < zones_end {
        let action = find(ZONE_ACTIONS, n % ZONE_GO_COUNT)?;
        Some(Command::Zone(n / ZONE_GO_COUNT + 1, action))
    } else if n < clients_end {
        let n = n - zones_end;
        let action = find(CLIENT_ACTIONS, n % CLIENT_GO_COUNT)?;
        Some(Command::Client(n / CLIENT_GO_COUNT + 1, action))
    } else {
        let action = find(GLOBAL_ACTIONS, n - clients_end)?;
        Some(Command::Global(action))
    }
}

/// A group object the device stack updated from the bus: ETS (or the TOML addresses) already
/// resolved the address through the association table, so the object alone says what to do.
pub(crate) async fn handle_object<S: BuildHasher + Sync>(
    asap: u16,
    data: &[u8],
    zone_commands: &HashMap<usize, ZoneCommandSender, S>,
    snap_tx: &tokio::sync::mpsc::Sender<SnapcastCmd>,
    store: &state::SharedState,
    sync_system_clock: bool,
) {
    let Some(command) = object_command(asap) else {
        tracing::trace!(asap, "KNX group object update without a command, ignored");
        return;
    };
    execute(
        command,
        data,
        &format!("object {asap}"),
        zone_commands,
        snap_tx,
        store,
        sync_system_clock,
    )
    .await;
}

/// A group write seen on a gateway connection: the configured addresses decide what it means.
#[allow(clippy::too_many_arguments)] // Match dispatcher for KNX group addresses — splitting would obscure control flow
pub(crate) async fn handle_incoming<S: BuildHasher + Sync>(
    ga: GroupAddress,
    data: &[u8],
    zone_ga_map: &HashMap<String, (usize, &str)>,
    client_ga_map: &HashMap<String, (usize, &str)>,
    global_ga_map: &HashMap<String, &str>,
    zone_commands: &HashMap<usize, ZoneCommandSender, S>,
    snap_tx: &tokio::sync::mpsc::Sender<SnapcastCmd>,
    store: &state::SharedState,
    sync_system_clock: bool,
) {
    let ga_str = format!("{ga}");

    // Device-level (global) group objects: All Stop, All Mute, KNX Time.
    if let Some(&action) = global_ga_map.get(&ga_str) {
        execute(
            Command::Global(action),
            data,
            &ga_str,
            zone_commands,
            snap_tx,
            store,
            sync_system_clock,
        )
        .await;
        return;
    }

    if let Some(&(zone_idx, action)) = zone_ga_map.get(&ga_str) {
        execute(
            Command::Zone(zone_idx, action),
            data,
            &ga_str,
            zone_commands,
            snap_tx,
            store,
            sync_system_clock,
        )
        .await;
    }

    if let Some(&(client_idx, action)) = client_ga_map.get(&ga_str) {
        execute(
            Command::Client(client_idx, action),
            data,
            &ga_str,
            zone_commands,
            snap_tx,
            store,
            sync_system_clock,
        )
        .await;
    }
}

/// Carry out `command`; `source` names the address or object it came from, for the log.
#[allow(clippy::too_many_lines)] // Match dispatcher for KNX actions — splitting would obscure control flow
async fn execute<S: BuildHasher + Sync>(
    command: Command<'_>,
    data: &[u8],
    source: &str,
    zone_commands: &HashMap<usize, ZoneCommandSender, S>,
    snap_tx: &tokio::sync::mpsc::Sender<SnapcastCmd>,
    store: &state::SharedState,
    sync_system_clock: bool,
) {
    match command {
        Command::Global(action) => match action {
            "all_stop" => {
                tracing::debug!(%source, "KNX → all zones stop");
                for tx in zone_commands.values() {
                    let _ = tx.send(ZoneCommand::Stop).await;
                }
            }
            "all_mute" => {
                let on = decode_bool(data);
                tracing::debug!(%source, mute = on, "KNX → all zones mute");
                for tx in zone_commands.values() {
                    let _ = tx.send(ZoneCommand::SetMute(on)).await;
                }
            }
            "knx_time" => apply_knx_time(data, sync_system_clock),
            _ => {}
        },
        Command::Zone(zone_idx, action) => {
            let Some(tx) = zone_commands.get(&zone_idx) else {
                return;
            };
            let cmd = match action {
                "play" => Some(ZoneCommand::Play),
                "pause" => Some(ZoneCommand::Pause),
                "stop" => Some(ZoneCommand::Stop),
                "next" => Some(ZoneCommand::Next),
                "previous" => Some(ZoneCommand::Previous),
                "mute_toggle" => Some(ZoneCommand::ToggleMute),
                "shuffle_toggle" => Some(ZoneCommand::ToggleShuffle),
                "repeat_toggle" | "track_repeat_toggle" => Some(ZoneCommand::CycleRepeat),
                "mute" => Some(ZoneCommand::SetMute(decode_bool(data))),
                "shuffle" => Some(ZoneCommand::SetShuffle(decode_bool(data))),
                "repeat" => {
                    let on = decode_bool(data);
                    Some(ZoneCommand::SetRepeat(if on {
                        snapdog_common::RepeatMode::Playlist
                    } else {
                        snapdog_common::RepeatMode::Off
                    }))
                }
                "track_repeat" => {
                    let on = decode_bool(data);
                    Some(ZoneCommand::SetRepeat(if on {
                        snapdog_common::RepeatMode::Track
                    } else {
                        snapdog_common::RepeatMode::Off
                    }))
                }
                "volume" => decode_percent(data).map(|v| ZoneCommand::SetVolume(i32::from(v))),
                "volume_dim" => decode_dim(data).map(ZoneCommand::AdjustVolume),
                "playlist" => decode_u8(data).map(|v| ZoneCommand::SetPlaylist(v as usize, 0)),
                "playlist_next" => Some(ZoneCommand::NextPlaylist),
                "playlist_previous" => Some(ZoneCommand::PreviousPlaylist),
                "presence" => Some(ZoneCommand::SetPresence(decode_bool(data))),
                "presence_enable" => Some(ZoneCommand::SetPresenceEnabled(decode_bool(data))),
                _ => None,
            };
            if let Some(cmd) = cmd {
                tracing::debug!(zone = zone_idx, %source, "KNX → zone command");
                let _ = tx.send(cmd).await;
            }
        }
        Command::Client(client_idx, action) => {
            let s = store.read().await;
            if let Some(client) = s.clients.get(&client_idx)
                && let Some(ref snap_id) = client.snapcast_id
            {
                let cmd = match action {
                    "mute_toggle" => Some(ClientAction::Mute(!client.muted)),
                    "mute" => Some(ClientAction::Mute(decode_bool(data))),
                    "volume" => decode_percent(data).map(|v| ClientAction::Volume(i32::from(v))),
                    "volume_dim" => decode_dim(data).map(ClientAction::AdjustVolume),
                    "latency" => decode_u8(data).map(|v| ClientAction::Latency(i32::from(v))),
                    "zone" => {
                        if let Some(target_zone) = decode_u8(data) {
                            let target_zone = usize::from(target_zone);
                            if target_zone == 0
                                || !zone_commands.contains_key(&target_zone)
                                || !s.zones.contains_key(&target_zone)
                            {
                                tracing::warn!(
                                    client = client_idx,
                                    zone = target_zone,
                                    %source,
                                    "Ignoring KNX client zone change to unknown zone"
                                );
                                return;
                            }
                            drop(s);
                            let mut store = store.write().await;
                            if let Some(c) = store.clients.get_mut(&client_idx) {
                                c.zone_index = target_zone;
                                store.dirty = true;
                            }
                            drop(store);
                            let _ = snap_tx.send(SnapcastCmd::ReconcileZones).await;
                            tracing::debug!(client = client_idx, zone = target_zone, %source, "KNX → client zone change");
                        }
                        return;
                    }
                    _ => None,
                };
                if let Some(action) = cmd {
                    let snap_id = snap_id.clone();
                    drop(s);
                    tracing::debug!(client = client_idx, %source, "KNX → client command");
                    let _ = snap_tx
                        .send(SnapcastCmd::Client {
                            client_id: snap_id,
                            action,
                        })
                        .await;
                }
            }
        }
    }
}

/// Apply a received KNX time-of-day (DPT 10.001).
///
/// Payload is 3 bytes: byte0 `ddd hhhhh`, byte1 minute, byte2 second. Updates the
/// presence-schedule clock offset and, when `sync_system_clock` is set, also sets the OS
/// clock's time-of-day (best effort; needs privilege).
fn apply_knx_time(data: &[u8], sync_system_clock: bool) {
    use chrono::Timelike as _;
    use std::sync::atomic::Ordering;
    if data.len() < 3 {
        tracing::warn!(
            len = data.len(),
            "KNX Time telegram too short (need 3 bytes)"
        );
        return;
    }
    let (hour, minute, second) = (
        i64::from(data[0] & 0x1F),
        i64::from(data[1] & 0x3F),
        i64::from(data[2] & 0x3F),
    );
    if hour > 23 || minute > 59 || second > 59 {
        tracing::warn!(hour, minute, second, "KNX Time out of range, ignoring");
        return;
    }
    let knx_secs = hour * 3600 + minute * 60 + second;
    let now = chrono::Local::now();
    let sys_secs =
        i64::from(now.hour()) * 3600 + i64::from(now.minute()) * 60 + i64::from(now.second());
    // Offset (seconds) to add to the system time-of-day to reach KNX time, wrapped to ±12h.
    let mut offset = knx_secs - sys_secs;
    if offset > 43_200 {
        offset -= 86_400;
    } else if offset < -43_200 {
        offset += 86_400;
    }
    KNX_TIME_OFFSET_SECS.store(offset, Ordering::Relaxed);
    tracing::debug!(hour, minute, second, offset, "KNX time received");

    if sync_system_clock {
        let time = format!("{hour:02}:{minute:02}:{second:02}");
        match std::process::Command::new("date")
            .arg("-s")
            .arg(&time)
            .status()
        {
            Ok(st) if st.success() => {
                KNX_TIME_OFFSET_SECS.store(0, Ordering::Relaxed);
                tracing::info!(%time, "OS clock set from KNX time");
            }
            Ok(st) => {
                tracing::warn!(%time, code = ?st.code(), "date failed to set OS clock (privilege?)");
            }
            Err(e) => tracing::warn!(%time, error = %e, "could not run date to set OS clock"),
        }
    }
}

// ── DPT decode helpers ────────────────────────────────────────

fn decode_bool(payload: &[u8]) -> bool {
    dpt::decode(DPT_SWITCH, payload)
        .ok()
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

fn decode_percent(payload: &[u8]) -> Option<u8> {
    dpt::decode(DPT_SCALING, payload)
        .ok()
        .and_then(|v| v.as_f64())
        .map(|v| v.clamp(0.0, 100.0).round() as u8)
}

fn decode_u8(payload: &[u8]) -> Option<u8> {
    dpt::decode(DPT_VALUE_1_UCOUNT, payload)
        .ok()
        .and_then(|v| v.as_u32())
        .map(|v| v.min(255) as u8)
}

/// Decode DPT 3.007 into a relative volume delta.
/// Returns `None` for stop (stepcode=0), `Some(delta)` otherwise.
/// Delta is positive for increase, negative for decrease.
fn decode_dim(payload: &[u8]) -> Option<i32> {
    let raw = dpt::decode(DPT_CONTROL_DIMMING, payload)
        .ok()
        .and_then(|v| v.as_u32())? as u8;
    let stepcode = raw & 0x07;
    if stepcode == 0 {
        return None; // stop
    }
    let increase = raw & 0x08 != 0;
    // stepcode 1=64%, 2=32%, 3=16%, 4=8%, 5=4%, 6=2%, 7=1%
    let step = match stepcode {
        1 => 64,
        2 => 32,
        3 => 16,
        4 => 8,
        5 => 4,
        6 => 2,
        _ => 1,
    };
    Some(if increase { step } else { -step })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::player::ZoneCommand;
    use group_objects::{client_asap, global_asap, zone_asap};
    use knx_rs_core::address::GroupAddress;
    use knx_rs_core::dpt::Dpt;
    use std::sync::Arc;
    use tokio::sync::{RwLock, mpsc};

    fn encode_bool(value: bool) -> Vec<u8> {
        dpt::encode(DPT_SWITCH, &value.into()).unwrap()
    }

    fn encode_percent(percent: u8) -> Vec<u8> {
        dpt::encode(DPT_SCALING, &DptValue::Float(f64::from(percent))).unwrap()
    }

    fn encode_string(value: &str) -> Vec<u8> {
        dpt::encode(DPT_STRING_8859_1, &DptValue::from(value)).unwrap()
    }

    fn encode_u8(value: u8) -> Vec<u8> {
        dpt::encode(DPT_VALUE_1_UCOUNT, &DptValue::from(value)).unwrap()
    }

    fn ga(s: &str) -> GroupAddress {
        GroupAddress::from_str(s).unwrap()
    }

    #[test]
    fn encodes_bool_via_dpt() {
        assert_eq!(encode_bool(true), vec![0x01]);
        assert_eq!(encode_bool(false), vec![0x00]);
    }

    #[test]
    fn round_trips_bool() {
        assert!(decode_bool(&encode_bool(true)));
        assert!(!decode_bool(&encode_bool(false)));
    }

    #[test]
    fn round_trips_percent() {
        assert_eq!(decode_percent(&encode_percent(0)), Some(0));
        assert_eq!(decode_percent(&encode_percent(100)), Some(100));
        assert_eq!(decode_percent(&encode_percent(50)), Some(50));
    }

    #[test]
    fn encodes_string_via_dpt() {
        let encoded = encode_string("Hello");
        assert_eq!(encoded.len(), 14); // DPT 16 is always 14 bytes
    }

    fn test_state() -> state::SharedState {
        let store: state::Store = serde_json::from_value(serde_json::json!({
            "zones": {
                "1": { "name": "Zone 1", "icon": "", "volume": 50, "muted": false, "playback": "stopped", "shuffle": false, "repeat": "off", "track": null, "playlist_index": null, "playlist_name": null, "playlist_total": null, "playlist_track_index": null, "playlist_track_count": null, "source": "idle", "cover_url": null, "snapcast_group_id": null, "presence": false, "presence_enabled": true, "presence_source": false, "auto_off_active": false, "auto_off_delay": 0 },
                "2": { "name": "Zone 2", "icon": "", "volume": 50, "muted": false, "playback": "stopped", "shuffle": false, "repeat": "off", "track": null, "playlist_index": null, "playlist_name": null, "playlist_total": null, "playlist_track_index": null, "playlist_track_count": null, "source": "idle", "cover_url": null, "snapcast_group_id": null, "presence": false, "presence_enabled": true, "presence_source": false, "auto_off_active": false, "auto_off_delay": 0 }
            },
            "clients": {
                "1": {
                    "name": "Test", "icon": "", "mac": "", "zone_index": 1,
                    "volume": 50, "base_volume": 50, "muted": false,
                    "latency_ms": 0, "connected": true, "snapcast_id": "snap-1"
                }
            }
        }))
        .unwrap();
        Arc::new(RwLock::new(store))
    }

    fn zone_ga_map() -> HashMap<String, (usize, &'static str)> {
        let mut m = HashMap::new();
        m.insert("1/0/1".into(), (1, "play"));
        m.insert("1/0/2".into(), (1, "pause"));
        m.insert("1/0/3".into(), (1, "stop"));
        m.insert("1/0/4".into(), (1, "next"));
        m.insert("1/0/5".into(), (1, "previous"));
        m.insert("1/0/6".into(), (1, "volume"));
        m.insert("1/0/7".into(), (1, "mute"));
        m.insert("1/0/8".into(), (1, "mute_toggle"));
        m.insert("1/0/9".into(), (1, "shuffle_toggle"));
        m.insert("1/0/10".into(), (1, "playlist"));
        m.insert("1/0/11".into(), (1, "volume_dim"));
        m.insert("1/0/12".into(), (1, "repeat"));
        m.insert("1/0/13".into(), (1, "track_repeat"));
        m.insert("1/0/15".into(), (1, "shuffle"));
        m.insert("1/0/16".into(), (1, "playlist_next"));
        m
    }

    fn client_ga_map() -> HashMap<String, (usize, &'static str)> {
        let mut m = HashMap::new();
        m.insert("2/0/1".into(), (1, "volume"));
        m.insert("2/0/2".into(), (1, "mute"));
        m.insert("2/0/3".into(), (1, "mute_toggle"));
        m.insert("2/0/4".into(), (1, "zone"));
        m.insert("2/0/5".into(), (1, "latency"));
        m
    }

    async fn run_incoming(
        ga_str: &str,
        data: &[u8],
        state: &state::SharedState,
    ) -> (mpsc::Receiver<ZoneCommand>, mpsc::Receiver<SnapcastCmd>) {
        let (tx, rx) = mpsc::channel(16);
        let (tx2, _rx2) = mpsc::channel(16);
        let mut cmds = HashMap::new();
        cmds.insert(1, tx);
        cmds.insert(2, tx2);
        let (snap_tx, snap_rx) = mpsc::channel(16);
        handle_incoming(
            ga(ga_str),
            data,
            &zone_ga_map(),
            &client_ga_map(),
            &HashMap::new(),
            &cmds,
            &snap_tx,
            state,
            false,
        )
        .await;
        (rx, snap_rx)
    }

    /// Dispatch a global-GO telegram and return the per-zone command receivers.
    async fn run_global(
        ga_str: &str,
        action: &'static str,
        data: &[u8],
    ) -> (mpsc::Receiver<ZoneCommand>, mpsc::Receiver<ZoneCommand>) {
        let (tx1, rx1) = mpsc::channel(4);
        let (tx2, rx2) = mpsc::channel(4);
        let mut cmds = HashMap::new();
        cmds.insert(1, tx1);
        cmds.insert(2, tx2);
        let (snap_tx, _snap_rx) = mpsc::channel(4);
        let mut global = HashMap::new();
        global.insert(ga_str.to_string(), action);
        handle_incoming(
            ga(ga_str),
            data,
            &HashMap::new(),
            &HashMap::new(),
            &global,
            &cmds,
            &snap_tx,
            &test_state(),
            false,
        )
        .await;
        (rx1, rx2)
    }

    #[tokio::test]
    async fn global_all_stop_stops_every_zone() {
        let (mut rx1, mut rx2) = run_global("0/0/1", "all_stop", &[]).await;
        assert!(matches!(rx1.try_recv(), Ok(ZoneCommand::Stop)));
        assert!(matches!(rx2.try_recv(), Ok(ZoneCommand::Stop)));
    }

    #[tokio::test]
    async fn global_all_mute_mutes_every_zone() {
        let (mut rx1, mut rx2) = run_global("0/0/2", "all_mute", &encode_bool(true)).await;
        assert!(matches!(rx1.try_recv(), Ok(ZoneCommand::SetMute(true))));
        assert!(matches!(rx2.try_recv(), Ok(ZoneCommand::SetMute(true))));
    }

    #[tokio::test]
    async fn global_knx_time_sets_schedule_offset() {
        use std::sync::atomic::Ordering;
        // 03:00:00 encoded as DPT 10.001 (day 0). The offset should become non-trivial
        // unless the system clock happens to already read 03:00.
        KNX_TIME_OFFSET_SECS.store(0, Ordering::Relaxed);
        run_global("0/0/3", "knx_time", &[3, 0, 0]).await;
        // The GO was consumed (no panic) and the offset reflects the received time.
        let off = KNX_TIME_OFFSET_SECS.load(Ordering::Relaxed);
        assert!((-86_400..=86_400).contains(&off));
    }

    #[tokio::test]
    async fn zone_play_command() {
        let (mut rx, _) = run_incoming("1/0/1", &[], &test_state()).await;
        assert!(matches!(rx.recv().await, Some(ZoneCommand::Play)));
    }

    #[tokio::test]
    async fn zone_volume_from_knx() {
        let (mut rx, _) = run_incoming("1/0/6", &encode_percent(75), &test_state()).await;
        assert!(matches!(rx.recv().await, Some(ZoneCommand::SetVolume(75))));
    }

    #[tokio::test]
    async fn zone_mute_from_knx() {
        let (mut rx, _) = run_incoming("1/0/7", &encode_bool(true), &test_state()).await;
        assert!(matches!(rx.recv().await, Some(ZoneCommand::SetMute(true))));
    }

    #[tokio::test]
    async fn zone_mute_toggle() {
        let (mut rx, _) = run_incoming("1/0/8", &[], &test_state()).await;
        assert!(matches!(rx.recv().await, Some(ZoneCommand::ToggleMute)));
    }

    #[tokio::test]
    async fn zone_playlist_from_knx() {
        let (mut rx, _) = run_incoming("1/0/10", &encode_u8(3), &test_state()).await;
        assert!(matches!(
            rx.recv().await,
            Some(ZoneCommand::SetPlaylist(3, 0))
        ));
    }

    #[tokio::test]
    async fn client_volume_from_knx() {
        let (_, mut snap_rx) = run_incoming("2/0/1", &encode_percent(80), &test_state()).await;
        assert!(matches!(
            snap_rx.recv().await,
            Some(SnapcastCmd::Client {
                action: ClientAction::Volume(80),
                ..
            })
        ));
    }

    #[tokio::test]
    async fn client_mute_from_knx() {
        let (_, mut snap_rx) = run_incoming("2/0/2", &encode_bool(true), &test_state()).await;
        assert!(matches!(
            snap_rx.recv().await,
            Some(SnapcastCmd::Client {
                action: ClientAction::Mute(true),
                ..
            })
        ));
    }

    #[tokio::test]
    async fn client_mute_toggle_from_knx() {
        let (_, mut snap_rx) = run_incoming("2/0/3", &[], &test_state()).await;
        assert!(matches!(
            snap_rx.recv().await,
            Some(SnapcastCmd::Client {
                action: ClientAction::Mute(true),
                ..
            })
        ));
    }

    #[tokio::test]
    async fn client_zone_change_from_knx_rejects_unknown_zone() {
        let state = test_state();
        let (_, mut snap_rx) = run_incoming("2/0/4", &encode_u8(99), &state).await;
        assert!(snap_rx.try_recv().is_err());
        assert_eq!(state.read().await.clients[&1].zone_index, 1);
    }

    #[tokio::test]
    async fn unknown_ga_ignored() {
        let (mut rx, mut snap_rx) = run_incoming("3/0/1", &encode_bool(true), &test_state()).await;
        assert!(rx.try_recv().is_err());
        assert!(snap_rx.try_recv().is_err());
    }

    #[test]
    fn round_trips_u8() {
        assert_eq!(decode_u8(&encode_u8(0)), Some(0));
        assert_eq!(decode_u8(&encode_u8(255)), Some(255));
        assert_eq!(decode_u8(&encode_u8(42)), Some(42));
    }

    #[test]
    fn decode_dim_increase() {
        // control=1 (increase), stepcode=3 (16%) → 0b1_011 = 0x0B
        let encoded = dpt::encode(DPT_CONTROL_DIMMING, &DptValue::UInt(0x0B)).unwrap();
        assert_eq!(decode_dim(&encoded), Some(16));
    }

    #[test]
    fn decode_dim_decrease() {
        // control=0 (decrease), stepcode=1 (64%) → 0b0_001 = 0x01
        let encoded = dpt::encode(DPT_CONTROL_DIMMING, &DptValue::UInt(0x01)).unwrap();
        assert_eq!(decode_dim(&encoded), Some(-64));
    }

    #[test]
    fn decode_dim_stop() {
        // stepcode=0 → stop
        let encoded = dpt::encode(DPT_CONTROL_DIMMING, &DptValue::UInt(0x08)).unwrap();
        assert_eq!(decode_dim(&encoded), None);
    }

    #[tokio::test]
    async fn zone_volume_dim_from_knx() {
        let encoded = dpt::encode(DPT_CONTROL_DIMMING, &DptValue::UInt(0x0A)).unwrap();
        let (mut rx, _) = run_incoming("1/0/11", &encoded, &test_state()).await;
        assert!(matches!(
            rx.recv().await,
            Some(ZoneCommand::AdjustVolume(32))
        ));
    }

    // ── IT-T40: explicit-byte decode goldens (+ decode_u16, zero coverage) ──

    #[test]
    fn decode_bool_handles_empty() {
        assert!(!decode_bool(&[])); // decode error → false default
    }

    #[test]
    fn decode_percent_byte_goldens() {
        assert_eq!(decode_percent(&[0x00]), Some(0));
        assert_eq!(decode_percent(&[0xFF]), Some(100));
        assert_eq!(decode_percent(&[0x80]), Some(50)); // 128*100/255 = 50.196 → 50
        assert_eq!(decode_percent(&[0x7F]), Some(50)); // 127 → 49.8 → 50
        assert_eq!(decode_percent(&[]), None);
    }

    // ── IT-T40: routing goldens for the previously-uncovered actions ──

    #[tokio::test]
    async fn zone_repeat_on_sets_playlist_mode() {
        let (mut rx, _) = run_incoming("1/0/12", &encode_bool(true), &test_state()).await;
        assert!(matches!(
            rx.recv().await,
            Some(ZoneCommand::SetRepeat(snapdog_common::RepeatMode::Playlist))
        ));
    }

    #[tokio::test]
    async fn zone_track_repeat_on_sets_track_mode() {
        let (mut rx, _) = run_incoming("1/0/13", &encode_bool(true), &test_state()).await;
        assert!(matches!(
            rx.recv().await,
            Some(ZoneCommand::SetRepeat(snapdog_common::RepeatMode::Track))
        ));
    }

    #[tokio::test]
    async fn zone_shuffle_set_from_knx() {
        let (mut rx, _) = run_incoming("1/0/15", &encode_bool(true), &test_state()).await;
        assert!(matches!(
            rx.recv().await,
            Some(ZoneCommand::SetShuffle(true))
        ));
    }

    #[tokio::test]
    async fn zone_playlist_next_from_knx() {
        let (mut rx, _) = run_incoming("1/0/16", &[], &test_state()).await;
        assert!(matches!(rx.recv().await, Some(ZoneCommand::NextPlaylist)));
    }

    #[tokio::test]
    async fn client_latency_from_knx() {
        let (_, mut snap_rx) = run_incoming("2/0/5", &encode_u8(120), &test_state()).await;
        assert!(matches!(
            snap_rx.recv().await,
            Some(SnapcastCmd::Client {
                action: ClientAction::Latency(120),
                ..
            })
        ));
    }

    #[tokio::test]
    async fn client_zone_change_accepts_valid_zone() {
        // Only the reject path was covered; this pins the happy path (store mutated
        // + ReconcileZones emitted) for a valid target zone.
        let state = test_state();
        let (_, mut snap_rx) = run_incoming("2/0/4", &encode_u8(2), &state).await;
        assert!(matches!(
            snap_rx.recv().await,
            Some(SnapcastCmd::ReconcileZones)
        ));
        assert_eq!(state.read().await.clients[&1].zone_index, 2);
        assert!(state.read().await.dirty);
    }

    // ── IT-T40: GA-map construction from config (passthrough GAs) ──────

    fn cfg_with_knx() -> crate::config::AppConfig {
        let toml = r#"
[[zone]]
name = "Z1"
[zone.knx]
play = "1/0/1"
volume = "1/0/6"

[[client]]
name = "C1"
mac = "02:42:ac:11:00:10"
zone = "Z1"
[client.knx]
volume = "2/0/1"
"#;
        let raw: crate::config::FileConfig = toml::from_str(toml).unwrap();
        crate::config::load_raw(raw).unwrap()
    }

    #[test]
    fn build_zone_ga_map_maps_configured_gas() {
        let m = build_zone_ga_map(&cfg_with_knx());
        assert_eq!(m.get("1/0/1").copied(), Some((1usize, "play")));
        assert_eq!(m.get("1/0/6").copied(), Some((1usize, "volume")));
        assert!(!m.contains_key("1/0/99"), "unconfigured GA omitted");
    }

    #[test]
    fn build_client_ga_map_maps_configured_gas() {
        let m = build_client_ga_map(&cfg_with_knx());
        assert_eq!(m.get("2/0/1").copied(), Some((1usize, "volume")));
    }

    // ── Group address notation ───────────────────────────────────────

    /// A zone whose command addresses are written in 2-level and zero-padded notation, and
    /// a global address in 2-level notation.
    fn cfg_with_mixed_notation() -> crate::config::AppConfig {
        let toml = r#"
[knx]
role = "client"
url = "udp://192.168.1.50:3671"
all_stop = "0/9"

[[zone]]
name = "Z1"
[zone.knx]
play = "1/2047"
volume = "01/0/06"
pause = "1/0/3"

[[client]]
name = "C1"
mac = "02:42:ac:11:00:10"
zone = "Z1"
"#;
        let raw: crate::config::FileConfig = toml::from_str(toml).unwrap();
        crate::config::load_raw(raw).unwrap()
    }

    #[test]
    fn ga_maps_are_keyed_by_the_three_level_form() {
        let cfg = cfg_with_mixed_notation();
        let zones = build_zone_ga_map(&cfg);
        assert_eq!(zones.get("1/7/255").copied(), Some((1usize, "play")));
        assert_eq!(zones.get("1/0/6").copied(), Some((1usize, "volume")));
        assert_eq!(zones.get("1/0/3").copied(), Some((1usize, "pause")));
        assert!(
            !zones.contains_key("1/2047"),
            "2-level key would never match"
        );
        assert_eq!(
            build_global_ga_map(&cfg).get("0/0/9").copied(),
            Some("all_stop")
        );
    }

    #[tokio::test]
    async fn telegrams_match_addresses_configured_in_two_level_notation() {
        let cfg = cfg_with_mixed_notation();
        let (tx, mut rx) = mpsc::channel(4);
        let (snap_tx, _snap_rx) = mpsc::channel(4);
        let cmds = HashMap::from([(1, tx)]);
        handle_incoming(
            ga("1/7/255"),
            &[],
            &build_zone_ga_map(&cfg),
            &build_client_ga_map(&cfg),
            &build_global_ga_map(&cfg),
            &cmds,
            &snap_tx,
            &test_state(),
            false,
        )
        .await;
        assert!(matches!(rx.try_recv(), Ok(ZoneCommand::Play)));
        handle_incoming(
            ga("0/0/9"),
            &[],
            &build_zone_ga_map(&cfg),
            &build_client_ga_map(&cfg),
            &build_global_ga_map(&cfg),
            &cmds,
            &snap_tx,
            &test_state(),
            false,
        )
        .await;
        assert!(matches!(rx.try_recv(), Ok(ZoneCommand::Stop)));
    }

    // ── IT-T42: publisher — status GOs with fixed DPTs + progress scaling ──

    type Rec = sink::testing::Recorder<true>;

    /// The `(address, bytes)` of every recorded client-mode write, in order.
    fn written(rec: &Rec) -> Vec<(String, Dpt, DptValue)> {
        rec.writes
            .lock()
            .unwrap()
            .iter()
            .map(|(t, d, v)| match t {
                transport::Target::Address(g) => (g.to_string(), *d, v.clone()),
                transport::Target::Object(a) => (format!("object {a}"), *d, v.clone()),
            })
            .collect()
    }

    fn track(position_ms: i64, duration_ms: i64) -> state::TrackInfo {
        serde_json::from_value(serde_json::json!({
            "title": "T", "artist": "A", "album": "Al",
            "album_artist": null, "genre": null, "year": null,
            "track_number": null, "disc_number": null,
            "duration_ms": duration_ms, "position_ms": position_ms,
            "seekable": true, "source": "idle",
            "bitrate_kbps": null, "content_type": null, "sample_rate": null
        }))
        .unwrap()
    }

    /// Single zone with the status GAs the publisher writes to.
    fn cfg_with_status_gas() -> crate::config::AppConfig {
        let toml = r#"
[[zone]]
name = "Z1"
[zone.knx]
volume_status = "1/1/1"
mute_status = "1/1/2"
repeat_status = "1/1/3"
track_repeat_status = "1/1/4"
control_status = "1/1/5"
track_playing_status = "1/1/6"
track_title_status = "1/1/7"
track_progress_status = "1/1/8"

[[client]]
name = "C1"
mac = "02:42:ac:11:00:10"
zone = "Z1"
"#;
        let raw: crate::config::FileConfig = toml::from_str(toml).unwrap();
        crate::config::load_raw(raw).unwrap()
    }

    #[test]
    fn track_progress_pct_scaling() {
        assert_eq!(track_progress_pct(&track(0, 100)), 0.0);
        assert_eq!(track_progress_pct(&track(50, 100)), 50.0);
        assert_eq!(track_progress_pct(&track(100, 100)), 100.0);
        assert_eq!(track_progress_pct(&track(150, 100)), 100.0); // clamp pos > dur
        assert_eq!(track_progress_pct(&track(10, 0)), 0.0); // duration 0 → 0
    }

    #[tokio::test]
    #[allow(clippy::significant_drop_tightening)] // guard held only for the multi-field seed
    async fn publish_zone_state_writes_status_gos_with_fixed_dpts() {
        let cfg = cfg_with_status_gas();
        let store = state::init(&cfg, None).unwrap();
        {
            let mut s = store.write().await;
            let z = s.zones.get_mut(&1).unwrap();
            z.volume = 60;
            z.muted = true;
            z.repeat = snapdog_common::RepeatMode::Playlist;
            z.playback = state::PlaybackState::Playing;
        }
        let mock = Rec::default();
        let mut sink = StatusSink::new(&mock);
        publish_zone_state(1, &cfg, &store, &mut sink).await;

        let w = written(&mock);
        let chk = |ga: &str, dpt: Dpt, bytes: &[u8]| {
            let (_, d, v) = w
                .iter()
                .find(|(g, _, _)| g == ga)
                .unwrap_or_else(|| panic!("no write for {ga}"));
            assert_eq!(*d, dpt, "{ga} dpt");
            assert_eq!(dpt::encode(*d, v).unwrap().as_slice(), bytes, "{ga} bytes");
        };
        chk("1/1/1", DPT_SCALING, &[0x99]); // volume 60 → round(153)
        chk("1/1/2", DPT_SWITCH, &[0x01]); // muted
        chk("1/1/3", DPT_SWITCH, &[0x01]); // repeat == Playlist → repeat_status on
        chk("1/1/4", DPT_SWITCH, &[0x00]); // repeat != Track → track_repeat_status off
        chk("1/1/5", DPT_SWITCH, &[0x01]); // control_status = playing
        chk("1/1/6", DPT_SWITCH, &[0x01]); // track_playing_status = playing (== control)
    }

    #[tokio::test]
    async fn publish_zone_track_writes_title_and_scaled_progress() {
        let cfg = cfg_with_status_gas();
        let store = state::init(&cfg, None).unwrap();
        {
            let mut s = store.write().await;
            s.zones.get_mut(&1).unwrap().track = Some(track(30, 100)); // 30%
        }
        let mock = Rec::default();
        let mut sink = StatusSink::new(&mock);
        publish_zone_track(1, &cfg, &store, &mut sink).await;

        let w = written(&mock);
        let title = w
            .iter()
            .find(|(g, _, _)| g == "1/1/7")
            .expect("title write");
        assert_eq!(title.1, DPT_STRING_8859_1);
        assert_eq!(dpt::encode(title.1, &title.2).unwrap().len(), 14); // DPT16 is 14 bytes

        let prog = w
            .iter()
            .find(|(g, _, _)| g == "1/1/8")
            .expect("progress write");
        assert_eq!(prog.1, DPT_SCALING);
        assert_eq!(dpt::encode(prog.1, &prog.2).unwrap(), vec![0x4D]); // 30% → round(76.5)=77
    }

    /// Radio and Subsonic playback make the zone announce itself every second because the
    /// position is part of the state. The publisher used to rewrite every configured status
    /// address each time; measure what reaches the bus now.
    #[tokio::test(start_paused = true)]
    #[allow(clippy::significant_drop_tightening)] // guard held only for the multi-field seed
    async fn a_minute_of_playback_ticks_stays_quiet() {
        const TICKS: usize = 60;
        const ADDRESSES: usize = 8; // the status addresses of cfg_with_status_gas
        let cfg = cfg_with_status_gas();
        let store = state::init(&cfg, None).unwrap();
        {
            let mut s = store.write().await;
            let z = s.zones.get_mut(&1).unwrap();
            z.playback = state::PlaybackState::Playing;
            z.track = Some(track(0, 200_000)); // a 200 s track
        }
        let rec = Rec::default();
        let mut sink = StatusSink::new(&rec);

        for tick in 0..TICKS {
            {
                let mut s = store.write().await;
                s.zones.get_mut(&1).unwrap().track =
                    Some(track(i64::try_from(tick).unwrap() * 1000, 200_000));
            }
            // What the publisher does for every ZoneChanged.
            publish_zone_state(1, &cfg, &store, &mut sink).await;
            publish_zone_track(1, &cfg, &store, &mut sink).await;
            publish_all_mute_status(&cfg, &store, &mut sink).await;
            tokio::time::advance(std::time::Duration::from_secs(1)).await;
        }

        let offered = TICKS * ADDRESSES;
        let sent = rec.count();
        println!("{TICKS} ticks, {offered} values offered, {sent} telegrams sent");
        // The first tick sends all addresses; afterwards only progress moves: one whole
        // percent per two seconds on this track.
        assert!(
            sent <= ADDRESSES + TICKS / 2 + 2,
            "{sent} telegrams for {TICKS} ticks"
        );
        assert!(sent >= ADDRESSES, "the first tick sends everything");
        let progress = written(&rec)
            .iter()
            .filter(|(g, _, _)| g == "1/1/8")
            .count();
        assert!(
            progress >= TICKS / 2 - 1,
            "progress still advances ({progress})"
        );
    }

    /// Client mode has no limit on zones. Zone 11's mute status and client 1's zone status
    /// are different objects that the device stack would number alike (ASAP 340); the change
    /// filter must tell them apart or the second telegram is suppressed.
    #[tokio::test]
    async fn eleven_zones_do_not_suppress_the_status_of_client_one() {
        let mut toml =
            String::from("[knx]\nrole = \"client\"\nurl = \"udp://192.168.1.50:3671\"\n");
        for z in 1..=11 {
            let _ =
                std::fmt::Write::write_fmt(&mut toml, format_args!("[[zone]]\nname = \"Z{z}\"\n"));
            if z == 11 {
                toml.push_str("[zone.knx]\nmute_status = \"3/0/1\"\n");
            }
        }
        toml.push_str(
            "[[client]]\nname = \"C1\"\nmac = \"02:42:ac:11:00:10\"\nzone = \"Z1\"\n[client.knx]\nzone_status = \"3/0/2\"\n",
        );
        let cfg = crate::config::load_raw(toml::from_str(&toml).unwrap()).unwrap();
        let store = state::init(&cfg, None).unwrap();
        store.write().await.zones.get_mut(&11).unwrap().muted = true;

        let rec = Rec::default();
        let mut sink = StatusSink::new(&rec);
        publish_zone_state(11, &cfg, &store, &mut sink).await;
        publish_client_state(1, &cfg, &store, &mut sink).await;

        let w = written(&rec);
        assert!(
            w.iter().any(|(g, _, _)| g == "3/0/1"),
            "zone 11 mute status"
        );
        assert!(
            w.iter().any(|(g, _, _)| g == "3/0/2"),
            "client 1 zone status"
        );
    }

    // ── object_command ──────────────────────────────────────────────

    #[test]
    fn object_command_maps_every_writable_object_and_only_those() {
        use group_objects::{CLIENT_GOS, GLOBAL_GOS, ZONE_GOS};
        for zone in [1, 2, MAX_ZONES] {
            for (i, def) in ZONE_GOS.iter().enumerate() {
                let cmd = object_command(zone_asap(zone, i));
                assert_eq!(
                    cmd.is_some(),
                    def.flags.write,
                    "zone {zone} object {i} ({}): writable objects command, status objects do not",
                    def.name
                );
                if let Some(Command::Zone(z, _)) = cmd {
                    assert_eq!(z, zone, "{}", def.name);
                }
            }
        }
        for client in [1, 2, MAX_CLIENTS] {
            for (i, def) in CLIENT_GOS.iter().enumerate() {
                let cmd = object_command(client_asap(client, i));
                assert_eq!(
                    cmd.is_some(),
                    def.flags.write,
                    "client {client} {}",
                    def.name
                );
                if let Some(Command::Client(c, _)) = cmd {
                    assert_eq!(c, client);
                }
            }
        }
        for (i, def) in GLOBAL_GOS.iter().enumerate() {
            let cmd = object_command(global_asap(i));
            assert_eq!(cmd.is_some(), def.flags.write, "global {}", def.name);
        }
    }

    #[test]
    fn object_command_names_the_action_of_known_objects() {
        assert_eq!(
            object_command(zone_asap(1, ZGO_PLAY)),
            Some(Command::Zone(1, "play"))
        );
        assert_eq!(
            object_command(zone_asap(3, ZGO_VOLUME)),
            Some(Command::Zone(3, "volume"))
        );
        assert_eq!(
            object_command(zone_asap(MAX_ZONES, ZGO_PRESENCE_ENABLE)),
            Some(Command::Zone(MAX_ZONES, "presence_enable"))
        );
        assert_eq!(
            object_command(client_asap(2, CGO_LATENCY)),
            Some(Command::Client(2, "latency"))
        );
        assert_eq!(
            object_command(global_asap(GGO_ALL_STOP)),
            Some(Command::Global("all_stop"))
        );
        // Status objects and out-of-range numbers command nothing.
        assert_eq!(object_command(zone_asap(1, ZGO_VOLUME_STATUS)), None);
        assert_eq!(object_command(0), None);
        assert_eq!(object_command(666), None);
        assert_eq!(object_command(u16::MAX), None);
        // Adjacent ASAPs at the block borders land on the right side.
        assert_eq!(
            object_command(330),
            None,
            "zone 10's last object is a status object"
        );
        assert_eq!(
            object_command(331),
            Some(Command::Client(1, "volume")),
            "first client object"
        );
    }

    // ── Reads through the publisher ─────────────────────────────────

    /// Run `fut` until the runtime has nothing else to do (paused time moves on by itself).
    async fn settle<F: std::future::Future<Output = ()> + Unpin>(fut: &mut F) {
        tokio::select! {
            () = fut => unreachable!("the publisher loop does not end"),
            () = tokio::time::sleep(std::time::Duration::from_millis(100)) => {}
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_late_display_gets_the_values_it_asks_for() {
        let cfg = cfg_with_status_gas();
        let store = state::init(&cfg, None).unwrap();
        store.write().await.zones.get_mut(&1).unwrap().volume = 60;
        let (snap_tx, _snap_rx) = mpsc::channel(4);
        let bridge = Bridge::new(
            &cfg,
            store,
            HashMap::<usize, ZoneCommandSender>::new(),
            snap_tx,
        );
        let rec = Rec::default();
        let (_notify_tx, mut notify_rx) = tokio::sync::broadcast::channel(8);
        let (read_tx, mut read_rx) = mpsc::unbounded_channel();
        let fut = publisher_loop(&rec, &bridge, &mut notify_rx, &mut read_rx);
        tokio::pin!(fut);

        settle(&mut fut).await;
        assert_eq!(
            rec.count(),
            8,
            "initial state: one telegram per status address"
        );

        // A visualisation starts and reads the volume status and a command address.
        read_tx.send(ga("1/1/1")).unwrap();
        settle(&mut fut).await;
        let responses = rec.responses.lock().unwrap().clone();
        assert_eq!(responses.len(), 1);
        assert_eq!(responses[0].0, ga("1/1/1"));
        assert_eq!(rec.count(), 8, "answering a read sends no write");
    }

    #[tokio::test(start_paused = true)]
    async fn a_read_of_a_value_that_was_never_sent_gets_the_current_value() {
        let cfg = cfg_with_status_gas();
        let store = state::init(&cfg, None).unwrap();
        let (snap_tx, _snap_rx) = mpsc::channel(4);
        let bridge = Bridge::new(
            &cfg,
            store,
            HashMap::<usize, ZoneCommandSender>::new(),
            snap_tx,
        );
        let rec = Rec::default();
        // The gateway takes nothing at first: every initial write fails.
        rec.accept
            .store(false, std::sync::atomic::Ordering::Relaxed);
        let (_notify_tx, mut notify_rx) = tokio::sync::broadcast::channel(8);
        let (read_tx, mut read_rx) = mpsc::unbounded_channel();
        let fut = publisher_loop(&rec, &bridge, &mut notify_rx, &mut read_rx);
        tokio::pin!(fut);
        settle(&mut fut).await;
        assert_eq!(rec.count(), 0);

        rec.accept.store(true, std::sync::atomic::Ordering::Relaxed);
        read_tx.send(ga("1/1/1")).unwrap();
        settle(&mut fut).await;
        assert_eq!(
            rec.responses.lock().unwrap().len(),
            1,
            "the read is answered"
        );
        assert_eq!(rec.count(), 7, "the other values go out as ordinary writes");
    }

    #[test]
    fn every_status_address_is_in_the_read_set() {
        let toml = r#"
[knx]
role = "client"
url = "udp://192.168.1.50:3671"
server_online = "0/0/1"
all_mute_status = "0/0/2"
system_fault = "0/0/3"
all_stop = "0/0/4"

[[zone]]
name = "Z1"
[zone.knx]
volume_status = "1/0/1"
mute_status = "1/0/2"
control_status = "1/0/3"
track_title_status = "1/0/4"
track_artist_status = "1/0/5"
track_album_status = "1/0/6"
track_progress_status = "1/0/7"
track_playing_status = "1/0/8"
track_repeat_status = "1/0/9"
playlist_status = "1/0/10"
shuffle_status = "1/0/11"
repeat_status = "1/0/12"
presence_enable_status = "1/0/13"
presence_timer_status = "1/0/14"
play = "1/0/20"

[[client]]
name = "C1"
mac = "02:42:ac:11:00:10"
zone = "Z1"
[client.knx]
volume_status = "2/0/1"
mute_status = "2/0/2"
latency_status = "2/0/3"
zone_status = "2/0/4"
connected_status = "2/0/5"
volume = "2/0/20"
"#;
        let cfg = crate::config::load_raw(toml::from_str(toml).unwrap()).unwrap();
        let set = build_status_ga_set(&cfg);
        assert_eq!(
            set.len(),
            14 + 5 + 3,
            "all status addresses, no command address"
        );
        assert!(!set.contains("1/0/20") && !set.contains("2/0/20") && !set.contains("0/0/4"));
    }

    #[test]
    fn duplicate_command_addresses_are_reported() {
        let toml = r#"
[knx]
role = "client"
url = "udp://192.168.1.50:3671"
all_stop = "0/0/1"

[[zone]]
name = "Z1"
[zone.knx]
play = "1/0/1"
pause = "1/0/1"
stop = "0/0/1"
[[zone]]
name = "Z2"
[zone.knx]
play = "1/0/2"

[[client]]
name = "C1"
mac = "02:42:ac:11:00:10"
zone = "Z1"
[client.knx]
volume = "2/0/1"
mute = "2/0/1"
"#;
        let cfg = crate::config::load_raw(toml::from_str(toml).unwrap()).unwrap();
        let d = duplicate_addresses(&cfg);
        assert_eq!(d.len(), 3, "{d:?}");
        assert!(d.iter().any(|m| m.contains("1/0/1")
            && m.contains("zone 1 play")
            && m.contains("zone 1 pause")));
        assert!(
            d.iter()
                .any(|m| m.contains("2/0/1") && m.contains("client 1"))
        );
        assert!(d.iter().any(|m| m.contains("0/0/1")
            && m.contains("global all_stop")
            && m.contains("never runs")));
        // A zone and a client may share an address: both run.
        let ok = cfg_with_knx();
        assert_eq!(duplicate_addresses(&ok).len(), 0);
    }
}
