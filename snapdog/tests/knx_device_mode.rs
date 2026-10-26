// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! KNX device mode over loopback: an ETS-style programmed device must dispatch group
//! telegrams to zone/client commands and send status values on the programmed addresses.
//!
//! The device transport is started through the public `knx::start_with_options` seam on a
//! loopback UDP port (never 3671, no multicast) and programmed through a real KNXnet/IP
//! tunnelling connection from knx-rs. The ETS download is *simulated*: the management
//! telegrams (load state machine, memory writes) are the same cEMI frames knx-rs's own
//! frame-driven download harness uses, not output of a real ETS.

#![allow(clippy::doc_markdown, clippy::unwrap_used, clippy::expect_used)]

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::str::FromStr;
use std::time::Duration;

use knx_rs_core::address::GroupAddress;
use knx_rs_core::cemi::CemiFrame;
use knx_rs_core::dpt::{self, DPT_SCALING, DPT_SWITCH, DptValue};
use knx_rs_core::message::ApduType;
use knx_rs_ip::ops::GroupOps;
use knx_rs_ip::{KnxConnection, TunnelConnection};
use snapdog::api::ws::Notification;
use snapdog::config;
use snapdog::knx::group_objects::{
    CGO_VOLUME, GGO_ALL_MUTE, GGO_ALL_STOP, GGO_KNX_TIME, GGO_SERVER_ONLINE, ZGO_MUTE,
    ZGO_MUTE_STATUS, ZGO_PLAY, ZGO_VOLUME, ZGO_VOLUME_STATUS, client_asap, global_asap, mem,
    zone_asap,
};
use snapdog::knx::{self, DeviceOptions, KnxDeviceControl as _};
use snapdog::player::{ClientAction, SnapcastCmd, ZoneCommand};
use snapdog::state;
use tokio::sync::{broadcast, mpsc};
use tokio::time::timeout;

const T: Duration = Duration::from_secs(5);
/// Management address of the (simulated) ETS and the device's own address.
const ETS_IA: u16 = 0xFFB1;
const DEVICE_IA: u16 = 0x1164; // 1.1.100

thread_local! {
    /// The address the simulated ETS manages. Tests run on a current-thread runtime, so a
    /// thread-local is per test. It changes once ETS has assigned a new individual address.
    static MANAGED_IA: std::cell::Cell<u16> = const { std::cell::Cell::new(DEVICE_IA) };
}

fn managed_ia() -> u16 {
    MANAGED_IA.with(std::cell::Cell::get)
}

// ── ETS download simulation ───────────────────────────────────

fn ga(s: &str) -> GroupAddress {
    GroupAddress::from_str(s).unwrap()
}

const fn apci_hdr(t: ApduType) -> [u8; 2] {
    let b = (t as u16).to_be_bytes();
    [b[0] & 0x03, b[1]]
}

/// Point-to-point `L_Data.req` carrying `apdu` from the ETS to the device.
fn mgmt_frame(dst: u16, apdu: &[u8]) -> CemiFrame {
    let mut f = vec![
        0x11,
        0x00,
        0xB0,
        0x60,
        (ETS_IA >> 8) as u8,
        (ETS_IA & 0xFF) as u8,
        (dst >> 8) as u8,
        (dst & 0xFF) as u8,
        u8::try_from(apdu.len() - 1).unwrap(),
    ];
    f.extend_from_slice(apdu);
    CemiFrame::parse(&f).unwrap()
}

/// Broadcast (`group 0`) `L_Data.req`, e.g. for individual-address services.
fn broadcast_frame(apdu: &[u8]) -> CemiFrame {
    let mut f = vec![
        0x11,
        0x00,
        0xB0,
        0xE0,
        (ETS_IA >> 8) as u8,
        (ETS_IA & 0xFF) as u8,
        0x00,
        0x00,
        u8::try_from(apdu.len() - 1).unwrap(),
    ];
    f.extend_from_slice(apdu);
    CemiFrame::parse(&f).unwrap()
}

fn prop_write(obj: u8, pid: u8, data: &[u8]) -> Vec<u8> {
    let [hi, lo] = apci_hdr(ApduType::PropertyValueWrite);
    let mut a = vec![hi, lo, obj, pid, 0x10, 0x01];
    a.extend_from_slice(data);
    a
}

fn mem_write(address: u16, data: &[u8]) -> Vec<u8> {
    let [hi, lo] = apci_hdr(ApduType::MemoryWrite);
    let mut a = vec![
        hi,
        lo | u8::try_from(data.len()).unwrap(),
        (address >> 8) as u8,
        (address & 0xFF) as u8,
    ];
    a.extend_from_slice(data);
    a
}

/// Write a property and wait for the device's response, as ETS does. A client that keeps
/// writing while the server waits for its acknowledgement of that response only makes both
/// sides retry.
async fn property_write(ets: &mut TunnelConnection, obj: u8, pid: u8, data: Vec<u8>) {
    ets.send(mgmt_frame(managed_ia(), &prop_write(obj, pid, &data)))
        .await
        .unwrap();
    timeout(T, async {
        loop {
            let frame = ets.recv().await.expect("tunnel open");
            if frame.as_group_write().is_none() {
                assert_eq!(
                    frame.message_code_raw(),
                    0x29,
                    "a management response is an L_Data.ind as well"
                );
                return;
            }
        }
    })
    .await
    .expect("property response");
}

/// Group objects the simulated ETS associates with group addresses.
struct Programming {
    /// `(group address, ASAP)` in association-table order. The first entry of an object is
    /// its sending address.
    links: Vec<(&'static str, u16)>,
    /// Parameter memory image (what ETS writes into the application program segment).
    params: Vec<u8>,
    /// Sort the association table by address, then object, instead of writing it in link order.
    sorted_associations: bool,
}

/// ETS reads where the device placed a segment (`PID_TABLE_REFERENCE`) before it writes there.
async fn table_reference(ets: &mut TunnelConnection, obj: u8) -> u16 {
    const TABLE_REFERENCE: u8 = 7;
    let [hi, lo] = apci_hdr(ApduType::PropertyValueRead);
    ets.send(mgmt_frame(
        managed_ia(),
        &[hi, lo, obj, TABLE_REFERENCE, 0x10, 0x01],
    ))
    .await
    .unwrap();
    let response = timeout(T, async {
        loop {
            let frame = ets.recv().await.expect("tunnel open");
            if frame.as_group_write().is_none() {
                return frame;
            }
        }
    })
    .await
    .expect("table reference response");
    let p = response.payload();
    u16::try_from(u32::from_be_bytes(p[p.len() - 4..].try_into().unwrap())).unwrap()
}

/// Allocate a relative segment of `size` bytes for `obj` (start loading, then allocate).
async fn allocate(ets: &mut TunnelConnection, obj: u8, size: usize) -> u16 {
    const LSC: u8 = 5; // PID_LOAD_STATE_CONTROL
    property_write(ets, obj, LSC, vec![1]).await;
    let mut alc = vec![0x03, 0x0B];
    alc.extend_from_slice(&u32::try_from(size).unwrap().to_be_bytes());
    alc.extend_from_slice(&[0x01, 0x00]); // relative allocation, fill with 0
    property_write(ets, obj, LSC, alc).await;
    table_reference(ets, obj).await
}

async fn write_segment(ets: &TunnelConnection, at: u16, data: &[u8]) {
    for (i, chunk) in data.chunks(60).enumerate() {
        let to = at + u16::try_from(i * 60).unwrap();
        ets.send(mgmt_frame(managed_ia(), &mem_write(to, chunk)))
            .await
            .unwrap();
    }
}

/// The download order of the generated product: the application segment is allocated first
/// (load procedure 2), then the address and association tables are loaded, then the
/// parameters are written into the application segment and it is completed.
async fn ets_download(ets: &mut TunnelConnection, p: &Programming) {
    const LSC: u8 = 5; // PID_LOAD_STATE_CONTROL
    const ADDR_TABLE: u8 = 1;
    const ASSOC_TABLE: u8 = 2;
    const APP_PROGRAM: u8 = 4;

    let mut gas: Vec<u16> = p.links.iter().map(|(g, _)| ga(g).raw()).collect();
    gas.sort_unstable();
    gas.dedup();
    let mut addr = u16::try_from(gas.len()).unwrap().to_be_bytes().to_vec();
    for g in &gas {
        addr.extend_from_slice(&g.to_be_bytes());
    }
    let mut entries: Vec<(usize, u16)> = p
        .links
        .iter()
        .map(|(g, asap)| {
            (
                gas.iter().position(|x| *x == ga(g).raw()).unwrap() + 1,
                *asap,
            )
        })
        .collect();
    if p.sorted_associations {
        entries.sort_unstable();
    }
    let mut assoc = u16::try_from(entries.len()).unwrap().to_be_bytes().to_vec();
    for (tsap, asap) in entries {
        assoc.extend_from_slice(&u16::try_from(tsap).unwrap().to_be_bytes());
        assoc.extend_from_slice(&asap.to_be_bytes());
    }

    // ETS unloads what it is about to load before it starts.
    for obj in [APP_PROGRAM, ADDR_TABLE, ASSOC_TABLE] {
        property_write(ets, obj, LSC, vec![4]).await;
    }
    let app_at = allocate(ets, APP_PROGRAM, p.params.len()).await;
    for (obj, data) in [(ADDR_TABLE, &addr), (ASSOC_TABLE, &assoc)] {
        let at = allocate(ets, obj, data.len()).await;
        write_segment(ets, at, data).await;
        property_write(ets, obj, LSC, vec![2]).await; // load completed
        // Completing a table can make a device configured, which then sends its status.
        drain(ets).await;
    }
    write_segment(ets, app_at, &p.params).await;
    property_write(ets, APP_PROGRAM, LSC, vec![2]).await;
}

/// Parameter memory with two zones, one client and no radios.
fn ets_params() -> Vec<u8> {
    let mut m = vec![0u8; mem::TOTAL];
    m[mem::NUM_ZONES] = 2;
    m[mem::NUM_CLIENTS] = 1;
    m[mem::GLOBAL_HTTP_PORT..mem::GLOBAL_HTTP_PORT + 2].copy_from_slice(&5555u16.to_be_bytes());
    m
}

// ── Fixture ───────────────────────────────────────────────────

struct Device {
    addr: SocketAddr,
    handle: knx::DeviceControlHandle,
    bridge: knx::KnxBridge,
    store: state::SharedState,
    notify: broadcast::Sender<std::sync::Arc<str>>,
    zone_rx: HashMap<usize, mpsc::Receiver<ZoneCommand>>,
    snap_rx: mpsc::Receiver<SnapcastCmd>,
}

/// Zones and a client as `--knx-device` derives them from ETS parameters: no group
/// addresses anywhere in the configuration (`RawZoneKnxConfig::default()`).
fn ets_style_config(extra_knx: &str, zone_knx: &str) -> config::AppConfig {
    let toml = format!(
        r#"
[knx]
role = "device"
individual_address = "1.1.100"
restart_after_ets = false
{extra_knx}

[[zone]]
name = "Living"
{zone_knx}

[[zone]]
name = "Kitchen"

[[client]]
name = "Speaker"
mac = "02:42:ac:11:00:10"
zone = "Living"
"#
    );
    let raw: config::FileConfig = toml::from_str(&toml).unwrap();
    config::load_raw(raw).unwrap()
}

async fn start_device(cfg: &config::AppConfig, memory: PathBuf) -> Device {
    start_device_on(cfg, memory, "127.0.0.1:0".parse().unwrap()).await
}

/// Start the device on `bind`. For an address with a port the server may not be up yet when
/// this returns; for port 0 it is, and its address is read back.
async fn start_device_on(cfg: &config::AppConfig, memory: PathBuf, bind: SocketAddr) -> Device {
    // `RUST_LOG=snapdog=info cargo test ... -- --nocapture` shows the startup summary.
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_test_writer()
        .try_init();
    let store = state::init(cfg, None).unwrap();
    // Commands for a client only go out once Snapcast has told us its id.
    store.write().await.clients.get_mut(&1).unwrap().snapcast_id = Some("snap-1".into());
    let (notify, notify_rx) = broadcast::channel(64);
    let mut zone_commands = HashMap::new();
    let mut zone_rx = HashMap::new();
    for z in 1..=cfg.zones.len() {
        let (tx, rx) = mpsc::channel(16);
        zone_commands.insert(z, tx);
        zone_rx.insert(z, rx);
    }
    let (snap_tx, snap_rx) = mpsc::channel(16);
    let options = DeviceOptions {
        bind: Some(bind),
        memory_path: Some(memory.clone()),
    };
    let bridge = knx::start_with_options(
        cfg,
        store.clone(),
        notify_rx,
        zone_commands,
        snap_tx,
        &options,
    )
    .await
    .expect("device transport starts");
    let handle = bridge
        .device_control()
        .expect("device mode returns a control handle");
    Device {
        addr: if bind.port() == 0 {
            handle.local_addr().expect("server bound")
        } else {
            bind
        },
        handle,
        bridge,
        store,
        notify,
        zone_rx,
        snap_rx,
    }
}

async fn connect(dev: &Device) -> TunnelConnection {
    let mut conn = TunnelConnection::connect(dev.addr).await.unwrap();
    // A device that is already configured publishes its state at startup; let that settle.
    drain(&mut conn).await;
    conn
}

/// Receive (and thereby acknowledge) whatever the device sends until the tunnel is quiet.
///
/// A knx-rs tunnel client does not acknowledge frames from the server while it waits for
/// the acknowledgement of its own request, and the server in turn waits for that
/// acknowledgement before it handles the request. A write that crosses a server-initiated
/// frame therefore stalls until the retries run out. Tests drain before they write.
async fn drain(conn: &mut TunnelConnection) -> Vec<CemiFrame> {
    let mut frames = Vec::new();
    while let Ok(Some(frame)) = timeout(Duration::from_millis(500), conn.recv()).await {
        frames.push(frame);
    }
    frames
}

/// Wait until the next group write to `dst` arrives on the tunnel and return its payload.
async fn expect_group_write(conn: &mut TunnelConnection, dst: &str) -> Vec<u8> {
    let want = ga(dst);
    timeout(T, async {
        loop {
            let frame = conn.recv().await.expect("tunnel open");
            if let Some((g, data)) = frame.as_group_write()
                && g == want
            {
                // A server tells its clients about a telegram with L_Data.ind (0x29). The device
                // stack builds its frames as L_Data.req (0x11), which a client or router that
                // follows the specification may ignore.
                assert_eq!(
                    frame.message_code_raw(),
                    0x29,
                    "status telegram to {dst} is not an L_Data.ind"
                );
                return data;
            }
        }
    })
    .await
    .unwrap_or_else(|_| panic!("no group write to {dst} within {T:?}"))
}

/// Assert that no group write to `dst` arrives within a short window.
async fn expect_no_group_write(conn: &mut TunnelConnection, dst: &str) {
    let want = ga(dst);
    let r = timeout(Duration::from_millis(700), async {
        loop {
            let frame = conn.recv().await?;
            if let Some((g, _)) = frame.as_group_write()
                && g == want
            {
                return Some(());
            }
        }
    })
    .await;
    assert!(
        !matches!(r, Ok(Some(()))),
        "unexpected group write to {dst}"
    );
}

fn notify_volume(dev: &Device, zone: usize, volume: i32, muted: bool) {
    let n = Notification::ZoneVolumeChanged {
        zone,
        volume,
        muted,
    };
    dev.notify
        .send(serde_json::to_string(&n).unwrap().into())
        .unwrap();
}

async fn set_zone_volume(dev: &Device, zone: usize, volume: i32, muted: bool) {
    let mut s = dev.store.write().await;
    let z = s.zones.get_mut(&zone).unwrap();
    z.volume = volume;
    z.muted = muted;
    drop(s);
    notify_volume(dev, zone, volume, muted);
}

fn ets_links() -> Programming {
    Programming {
        links: vec![
            ("1/1/1", zone_asap(1, ZGO_PLAY)),
            ("1/1/2", zone_asap(1, ZGO_VOLUME)),
            ("1/1/3", zone_asap(1, ZGO_VOLUME_STATUS)),
            ("1/1/4", zone_asap(1, ZGO_MUTE_STATUS)),
            ("1/2/1", zone_asap(2, ZGO_MUTE)),
            ("2/1/1", client_asap(1, CGO_VOLUME)),
            ("0/0/1", global_asap(GGO_ALL_STOP)),
            ("0/0/2", global_asap(GGO_ALL_MUTE)),
            ("0/0/3", global_asap(GGO_KNX_TIME)),
            ("0/0/4", global_asap(GGO_SERVER_ONLINE)),
            // One address for a command and a status object, as installations often do.
            ("1/1/9", zone_asap(1, ZGO_VOLUME)),
            ("1/1/9", zone_asap(1, ZGO_VOLUME_STATUS)),
        ],
        params: ets_params(),
        sorted_associations: false,
    }
}

/// No zone command within a short window.
async fn expect_no_command(rx: &mut mpsc::Receiver<ZoneCommand>) {
    let r = timeout(Duration::from_millis(600), rx.recv()).await;
    assert!(r.is_err(), "unexpected zone command: {r:?}");
}

// ── Tests ─────────────────────────────────────────────────────

/// Commands on ETS-programmed addresses reach the zones, the client and the global objects,
/// although the configuration carries no group address at all.
#[tokio::test]
async fn ets_programmed_device_dispatches_telegrams() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = ets_style_config("", "");
    let mut dev = start_device(&cfg, tmp.path().join("knx-memory.bin")).await;
    let mut ets = connect(&dev).await;
    ets_download(&mut ets, &ets_links()).await;
    drain(&mut ets).await;

    // Zone 1: trigger and percentage.
    ets.group_write_value(ga("1/1/1"), DPT_SWITCH, &true.into())
        .await
        .unwrap();
    let cmd = timeout(T, dev.zone_rx.get_mut(&1).unwrap().recv()).await;
    assert!(
        matches!(cmd, Ok(Some(ZoneCommand::Play))),
        "zone 1 play: {cmd:?}"
    );

    ets.group_write_value(ga("1/1/2"), DPT_SCALING, &DptValue::Float(75.0))
        .await
        .unwrap();
    let cmd = timeout(T, dev.zone_rx.get_mut(&1).unwrap().recv()).await;
    assert!(
        matches!(cmd, Ok(Some(ZoneCommand::SetVolume(75)))),
        "zone 1 volume: {cmd:?}"
    );

    // Zone 2 is addressed by its own object, not zone 1's.
    ets.group_write_value(ga("1/2/1"), DPT_SWITCH, &true.into())
        .await
        .unwrap();
    let cmd = timeout(T, dev.zone_rx.get_mut(&2).unwrap().recv()).await;
    assert!(
        matches!(cmd, Ok(Some(ZoneCommand::SetMute(true)))),
        "zone 2 mute: {cmd:?}"
    );
    assert!(dev.zone_rx.get_mut(&1).unwrap().try_recv().is_err());

    // Client object.
    ets.group_write_value(ga("2/1/1"), DPT_SCALING, &DptValue::Float(40.0))
        .await
        .unwrap();
    let cmd = timeout(T, dev.snap_rx.recv()).await;
    assert!(
        matches!(
            cmd,
            Ok(Some(SnapcastCmd::Client {
                action: ClientAction::Volume(40),
                ..
            }))
        ),
        "client volume: {cmd:?}"
    );

    // Global "All Stop" reaches every zone.
    ets.group_write_value(ga("0/0/1"), DPT_SWITCH, &true.into())
        .await
        .unwrap();
    for z in [1, 2] {
        let cmd = timeout(T, dev.zone_rx.get_mut(&z).unwrap().recv()).await;
        assert!(
            matches!(cmd, Ok(Some(ZoneCommand::Stop))),
            "all stop, zone {z}: {cmd:?}"
        );
    }
}

/// A state change is written to the address ETS associated with the status object.
#[tokio::test]
async fn ets_programmed_device_sends_status_on_programmed_addresses() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = ets_style_config("", "");
    let dev = start_device(&cfg, tmp.path().join("knx-memory.bin")).await;
    let mut ets = connect(&dev).await;
    ets_download(&mut ets, &ets_links()).await;
    drain(&mut ets).await;

    set_zone_volume(&dev, 1, 30, true).await;

    let vol = expect_group_write(&mut ets, "1/1/3").await;
    assert_eq!(
        dpt::decode(DPT_SCALING, &vol)
            .unwrap()
            .as_f64()
            .map(f64::round),
        Some(30.0)
    );
    let muted = expect_group_write(&mut ets, "1/1/4").await;
    assert!(dpt::decode(DPT_SWITCH, &muted).unwrap().as_bool().unwrap());
}

/// Global objects work in device mode as well: the three-byte KNX time arrives intact and
/// the "All Mute" status is sent on its address.
#[tokio::test]
async fn ets_programmed_device_handles_global_objects() {
    use std::sync::atomic::Ordering;

    let tmp = tempfile::tempdir().unwrap();
    let cfg = ets_style_config("", "");
    let dev = start_device(&cfg, tmp.path().join("knx-memory.bin")).await;
    let mut ets = connect(&dev).await;
    ets_download(&mut ets, &ets_links()).await;
    drain(&mut ets).await;

    // 03:00:00, DPT 10.001. The schedule offset must follow the received time.
    knx::KNX_TIME_OFFSET_SECS.store(i64::MAX, Ordering::Relaxed);
    ets.group_write_value(
        ga("0/0/3"),
        dpt::DPT_TIMEOFDAY,
        &DptValue::Bytes(vec![3, 0, 0]),
    )
    .await
    .unwrap_or_else(|e| panic!("time write: {e}"));
    timeout(T, async {
        while knx::KNX_TIME_OFFSET_SECS.load(Ordering::Relaxed) == i64::MAX {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("KNX time telegram applied");

    // Both zones muted: the all-mute status goes out.
    for z in [1, 2] {
        set_zone_volume(&dev, z, 50, true).await;
    }
    let all = expect_group_write(&mut ets, "0/0/2").await;
    assert!(dpt::decode(DPT_SWITCH, &all).unwrap().as_bool().unwrap());
}

/// The programming survives a restart: a second device instance restores the persisted
/// tables and dispatches on the same addresses.
#[tokio::test]
async fn programming_survives_restart() {
    let tmp = tempfile::tempdir().unwrap();
    let memory = tmp.path().join("knx-memory.bin");
    let cfg = ets_style_config("", "");
    {
        let dev = start_device(&cfg, memory.clone()).await;
        let mut ets = connect(&dev).await;
        ets_download(&mut ets, &ets_links()).await;
        drain(&mut ets).await;
        // Stopping the bridge closes the server and persists what ETS wrote.
        dev.bridge.shutdown().await;
    }
    timeout(T, async {
        while !memory.exists() {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("ETS programming persisted");

    let mut dev = start_device(&cfg, memory).await;
    let mut ets = connect(&dev).await;
    ets.group_write_value(ga("1/1/2"), DPT_SCALING, &DptValue::Float(20.0))
        .await
        .unwrap();
    let cmd = timeout(T, dev.zone_rx.get_mut(&1).unwrap().recv()).await;
    assert!(
        matches!(cmd, Ok(Some(ZoneCommand::SetVolume(20)))),
        "restored tables dispatch: {cmd:?}"
    );

    // The restored device also sends on the programmed address.
    set_zone_volume(&dev, 1, 45, false).await;
    let vol = expect_group_write(&mut ets, "1/1/3").await;
    assert_eq!(
        dpt::decode(DPT_SCALING, &vol)
            .unwrap()
            .as_f64()
            .map(f64::round),
        Some(45.0)
    );
}

/// The device stack appends the segments of every download. The persisted state keeps only the
/// segments in use, so re-downloading does not grow the memory file (and, past 64 KB, did not
/// corrupt it).
#[tokio::test]
async fn repeated_downloads_do_not_grow_the_memory_file() {
    let tmp = tempfile::tempdir().unwrap();
    let memory = tmp.path().join("knx-memory.bin");
    let cfg = ets_style_config("", "");
    let dev = start_device(&cfg, memory.clone()).await;
    let mut ets = connect(&dev).await;
    for _ in 0..4 {
        ets_download(&mut ets, &ets_links()).await;
        drain(&mut ets).await;
    }
    dev.bridge.shutdown().await;

    let raw = std::fs::read(&memory).unwrap();
    let payload = usize::from(u16::from_le_bytes([raw[5], raw[6]]));
    assert!(
        payload < mem::TOTAL + 2_000,
        "{payload} bytes after four downloads (one parameter block is {})",
        mem::TOTAL
    );

    // And it restores to a device that works.
    let mut dev = start_device(&cfg, memory).await;
    let ets = connect(&dev).await;
    ets.group_write_value(ga("1/1/1"), DPT_SWITCH, &true.into())
        .await
        .unwrap();
    let cmd = timeout(T, dev.zone_rx.get_mut(&1).unwrap().recv()).await;
    assert!(matches!(cmd, Ok(Some(ZoneCommand::Play))), "{cmd:?}");
}

/// A download that adds an address for an object that had none: the status the object holds goes
/// out on it, without any state change. (The publisher offered it when the device was started,
/// when there was no address to send it to.)
#[tokio::test]
async fn a_download_that_adds_an_address_publishes_its_status() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = ets_style_config("", "");
    let dev = start_device(&cfg, tmp.path().join("knx-memory.bin")).await;
    let mut ets = connect(&dev).await;

    let mut links = ets_links();
    links.links.retain(|(g, _)| *g != "1/1/4");
    ets_download(&mut ets, &links).await;
    drain(&mut ets).await;

    ets_download(&mut ets, &ets_links()).await;
    let muted = expect_group_write(&mut ets, "1/1/4").await;
    assert!(!dpt::decode(DPT_SWITCH, &muted).unwrap().as_bool().unwrap());
}

/// The stack sends an object's value to the first association of the object in the table, so
/// the order in which ETS writes the table decides the sending address. Pinned for both
/// orders (the real order of ETS is not verified).
#[tokio::test]
async fn an_object_sends_to_the_first_of_its_associations_in_table_order() {
    for sorted in [false, true] {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = ets_style_config("", "");
        let dev = start_device(&cfg, tmp.path().join("knx-memory.bin")).await;
        let mut ets = connect(&dev).await;
        let mut programming = ets_links();
        // The zone volume status is linked to 1/1/9 first and 1/1/3 second.
        programming
            .links
            .retain(|(_, asap)| *asap != zone_asap(1, ZGO_VOLUME_STATUS));
        programming
            .links
            .push(("1/1/9", zone_asap(1, ZGO_VOLUME_STATUS)));
        programming
            .links
            .push(("1/1/3", zone_asap(1, ZGO_VOLUME_STATUS)));
        programming.sorted_associations = sorted;
        ets_download(&mut ets, &programming).await;
        drain(&mut ets).await;

        set_zone_volume(&dev, 1, 30, false).await;
        let send = if sorted { "1/1/3" } else { "1/1/9" };
        let other = if sorted { "1/1/9" } else { "1/1/3" };
        expect_group_write(&mut ets, send).await;
        expect_no_group_write(&mut ets, other).await;
    }
}

/// `A_Restart` ends a download: the state is persisted at once, not after the debounce, because
/// the process may restart (and re-exec) right after it.
#[tokio::test]
async fn a_restart_persists_at_once() {
    let tmp = tempfile::tempdir().unwrap();
    let memory = tmp.path().join("knx-memory.bin");
    let cfg = ets_style_config("", "");
    let dev = start_device(&cfg, memory.clone()).await;
    let mut ets = connect(&dev).await;
    ets_download(&mut ets, &ets_links()).await;
    drain(&mut ets).await; // the status the device sends once it is configured
    assert!(!memory.exists(), "inside the debounce");

    let [hi, lo] = apci_hdr(ApduType::Restart);
    ets.send(mgmt_frame(managed_ia(), &[hi, lo])).await.unwrap();
    timeout(Duration::from_millis(1200), async {
        while !memory.exists() {
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("persisted by the restart, well before the two-second debounce");
}

/// Shutdown returns only after the device task has written the state it holds. The debounce for
/// a persist after a download is two seconds, so a shutdown right after the download has to
/// write the file itself, and the bridge has to wait for that.
#[tokio::test]
async fn shutdown_returns_after_the_state_is_on_disk() {
    let tmp = tempfile::tempdir().unwrap();
    let memory = tmp.path().join("knx-memory.bin");
    let cfg = ets_style_config("", "");
    let dev = start_device(&cfg, memory.clone()).await;
    let mut ets = connect(&dev).await;
    ets_download(&mut ets, &ets_links()).await;
    assert!(!memory.exists(), "the debounce has not elapsed yet");
    dev.bridge.shutdown().await;
    assert!(
        memory.exists(),
        "shutdown() returned before the state was written"
    );
    assert!(
        knx_memory_loads(&memory),
        "and what it wrote is a complete file"
    );
}

fn knx_memory_loads(path: &std::path::Path) -> bool {
    let raw = std::fs::read(path).unwrap();
    raw.len() > 11 && &raw[..4] == b"SDKM"
}

/// Status is sent when it changes, not on every state notification.
#[tokio::test]
async fn status_is_sent_on_change_only() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = ets_style_config("", "");
    let dev = start_device(&cfg, tmp.path().join("knx-memory.bin")).await;
    let mut ets = connect(&dev).await;
    ets_download(&mut ets, &ets_links()).await;
    drain(&mut ets).await;

    set_zone_volume(&dev, 1, 30, false).await;
    expect_group_write(&mut ets, "1/1/3").await;
    drain(&mut ets).await;

    // Playback progress makes the zone announce itself about once a second.
    for _ in 0..5 {
        notify_volume(&dev, 1, 30, false);
    }
    expect_no_group_write(&mut ets, "1/1/3").await;
    expect_no_group_write(&mut ets, "1/1/4").await;

    set_zone_volume(&dev, 1, 31, false).await;
    expect_group_write(&mut ets, "1/1/3").await;
}

/// A group value response answers somebody else's read; it never executes a command. Reads of
/// command objects are not answered, reads of status objects are, with the actual value, also
/// when that value was never sent on the bus (the device was not programmed yet when the status
/// was first published).
#[tokio::test]
async fn responses_and_reads_follow_the_object_flags() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = ets_style_config("", "");
    let mut dev = start_device(&cfg, tmp.path().join("knx-memory.bin")).await;
    let mut ets = connect(&dev).await;
    ets_download(&mut ets, &ets_links()).await;
    drain(&mut ets).await;

    ets.group_respond_value(ga("1/1/2"), DPT_SCALING, &DptValue::Float(66.0))
        .await
        .unwrap();
    expect_no_command(dev.zone_rx.get_mut(&1).unwrap()).await;
    ets.group_write_value(ga("1/1/2"), DPT_SCALING, &DptValue::Float(66.0))
        .await
        .unwrap();
    let cmd = timeout(T, dev.zone_rx.get_mut(&1).unwrap().recv()).await;
    assert!(
        matches!(cmd, Ok(Some(ZoneCommand::SetVolume(66)))),
        "a write does: {cmd:?}"
    );

    // The volume command address is not readable.
    ets.group_read(ga("1/1/2")).await.unwrap();
    expect_no_group_write(&mut ets, "1/1/2").await;

    // The status address is, and says what the zone's volume is (50 by default).
    ets.group_read(ga("1/1/3")).await.unwrap();
    let vol = expect_group_write(&mut ets, "1/1/3").await;
    assert_eq!(
        dpt::decode(DPT_SCALING, &vol)
            .unwrap()
            .as_f64()
            .map(f64::round),
        Some(50.0)
    );

    // After a change the answer follows.
    set_zone_volume(&dev, 1, 30, false).await;
    expect_group_write(&mut ets, "1/1/3").await;
    ets.group_read(ga("1/1/3")).await.unwrap();
    let vol = expect_group_write(&mut ets, "1/1/3").await;
    assert_eq!(
        dpt::decode(DPT_SCALING, &vol)
            .unwrap()
            .as_f64()
            .map(f64::round),
        Some(30.0)
    );
}

/// A visualisation that writes onto a status address does not change the zone and does not stay
/// on the bus: the device says what the value is. An address that a command object listens to
/// as well is the command's business.
#[tokio::test]
async fn a_foreign_write_on_a_status_address_is_corrected() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = ets_style_config("", "");
    let mut dev = start_device(&cfg, tmp.path().join("knx-memory.bin")).await;
    let mut ets = connect(&dev).await;
    ets_download(&mut ets, &ets_links()).await;
    drain(&mut ets).await;

    ets.group_write_value(ga("1/1/3"), DPT_SCALING, &DptValue::Float(90.0))
        .await
        .unwrap();
    let vol = expect_group_write(&mut ets, "1/1/3").await;
    assert_eq!(
        dpt::decode(DPT_SCALING, &vol)
            .unwrap()
            .as_f64()
            .map(f64::round),
        Some(50.0),
        "the actual volume is sent again"
    );
    expect_no_command(dev.zone_rx.get_mut(&1).unwrap()).await;

    // 1/1/9 is a command and a status address at once: the command runs, nothing is "corrected".
    ets.group_write_value(ga("1/1/9"), DPT_SCALING, &DptValue::Float(70.0))
        .await
        .unwrap();
    let cmd = timeout(T, dev.zone_rx.get_mut(&1).unwrap().recv()).await;
    assert!(
        matches!(cmd, Ok(Some(ZoneCommand::SetVolume(70)))),
        "{cmd:?}"
    );
    expect_no_group_write(&mut ets, "1/1/9").await;
}

/// While ETS reloads a table the address and association tables disagree. A telegram in that
/// window is not dispatched, and dispatch works again once the device is programmed.
#[tokio::test]
async fn no_telegram_is_dispatched_while_a_table_is_reloaded() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = ets_style_config("", "");
    let mut dev = start_device(&cfg, tmp.path().join("knx-memory.bin")).await;
    let mut ets = connect(&dev).await;
    ets_download(&mut ets, &ets_links()).await;
    drain(&mut ets).await;

    // ETS starts to reload the association table (object 2): start loading, no data yet.
    property_write(&mut ets, 2, 5, vec![1]).await;
    ets.group_write_value(ga("1/1/1"), DPT_SWITCH, &true.into())
        .await
        .unwrap();
    expect_no_command(dev.zone_rx.get_mut(&1).unwrap()).await;

    ets_download(&mut ets, &ets_links()).await;
    drain(&mut ets).await;
    ets.group_write_value(ga("1/1/1"), DPT_SWITCH, &true.into())
        .await
        .unwrap();
    let cmd = timeout(T, dev.zone_rx.get_mut(&1).unwrap().recv()).await;
    assert!(matches!(cmd, Ok(Some(ZoneCommand::Play))), "{cmd:?}");
}

// ── Controls: TOML-configured addresses ───────────────────────

/// Control for the ETS tests above: the same ETS programming works when the configuration
/// repeats the programmed addresses as strings (what device mode needed before dispatch
/// followed the group object identity). Proves the tables, the tunnel and the simulated
/// download are sound and that the ETS-only failures come from the missing strings.
#[tokio::test]
async fn control_toml_strings_matching_the_programming_work() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = ets_style_config(
        "",
        r#"
[zone.knx]
play = "1/1/1"
volume_status = "1/1/3"
"#,
    );
    let mut dev = start_device(&cfg, tmp.path().join("knx-memory.bin")).await;
    let mut ets = connect(&dev).await;
    ets_download(&mut ets, &ets_links()).await;
    drain(&mut ets).await;

    ets.group_write_value(ga("1/1/1"), DPT_SWITCH, &true.into())
        .await
        .unwrap();
    let cmd = timeout(T, dev.zone_rx.get_mut(&1).unwrap().recv()).await;
    assert!(
        matches!(cmd, Ok(Some(ZoneCommand::Play))),
        "zone 1 play: {cmd:?}"
    );

    set_zone_volume(&dev, 1, 30, false).await;
    let vol = expect_group_write(&mut ets, "1/1/3").await;
    assert_eq!(
        dpt::decode(DPT_SCALING, &vol)
            .unwrap()
            .as_f64()
            .map(f64::round),
        Some(30.0)
    );
}

/// Device mode that ETS never programmed takes its tables from the TOML addresses.
#[tokio::test]
async fn toml_only_device_dispatches_and_sends() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = ets_style_config(
        "",
        r#"
[zone.knx]
play = "1/1/1"
volume_status = "1/1/3"
"#,
    );
    let mut dev = start_device(&cfg, tmp.path().join("knx-memory.bin")).await;
    let mut ets = connect(&dev).await;

    ets.group_write_value(ga("1/1/1"), DPT_SWITCH, &true.into())
        .await
        .unwrap();
    let cmd = timeout(T, dev.zone_rx.get_mut(&1).unwrap().recv()).await;
    assert!(
        matches!(cmd, Ok(Some(ZoneCommand::Play))),
        "zone 1 play: {cmd:?}"
    );

    set_zone_volume(&dev, 1, 30, false).await;
    let vol = expect_group_write(&mut ets, "1/1/3").await;
    assert_eq!(
        dpt::decode(DPT_SCALING, &vol)
            .unwrap()
            .as_f64()
            .map(f64::round),
        Some(30.0)
    );
}

// ── Server supervision ────────────────────────────────────────

/// Open a tunnel by hand and then read and acknowledge nothing.
async fn connect_and_go_silent(server: SocketAddr) -> tokio::net::UdpSocket {
    use knx_rs_core::knxip::{Hpai, KnxIpFrame, ServiceType};
    let socket = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let port = socket.local_addr().unwrap().port();
    let request = KnxIpFrame::connect_request(
        Hpai::nat_udp(port),
        Hpai::nat_udp(port),
        &[0x04, 0x04, 0x02, 0x00],
    );
    socket.send_to(&request.to_bytes(), server).await.unwrap();
    let mut buf = [0u8; 512];
    let (n, _) = timeout(T, socket.recv_from(&mut buf))
        .await
        .expect("connect response")
        .unwrap();
    assert_eq!(
        KnxIpFrame::parse(&buf[..n]).unwrap().service_type,
        ServiceType::ConnectResponse
    );
    socket
}

/// From now on the silent client acknowledges every tunnelling request it receives.
fn spawn_acker(socket: std::sync::Arc<tokio::net::UdpSocket>) -> tokio::task::JoinHandle<()> {
    use knx_rs_core::knxip::{KnxIpFrame, ServiceType};
    tokio::spawn(async move {
        let mut buf = [0u8; 512];
        while let Ok(Ok((n, from))) =
            timeout(Duration::from_secs(60), socket.recv_from(&mut buf)).await
        {
            if let Ok(f) = KnxIpFrame::parse(&buf[..n])
                && f.service_type == ServiceType::TunnelingRequest
            {
                let ack = KnxIpFrame::tunneling_ack(f.body[1], f.body[2], 0);
                let _ = socket.send_to(&ack.to_bytes(), from).await;
            }
        }
    })
}

/// A tunnel client that stops acknowledging (a hung visualisation, a half-open NAT mapping), a burst
/// of status changes and a flood of incoming telegrams used to wedge the device for good: the
/// KNXnet/IP server waited for the device task to take events while the device task waited for the
/// server to take frames. Now the device task never waits for the server, so the management calls
/// answer and shutdown returns while the client is still dead, and the device is alive again soon
/// after the client acknowledges.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_dead_tunnel_client_does_not_wedge_the_device() {
    use knx_rs_core::knxip::KnxIpFrame;
    use tokio::net::UdpSocket;

    let tmp = tempfile::tempdir().unwrap();
    let cfg = ets_style_config("", "");
    let mut dev = start_device(&cfg, tmp.path().join("knx-memory.bin")).await;
    {
        let mut ets = connect(&dev).await;
        ets_download(&mut ets, &ets_links()).await;
        drain(&mut ets).await;
    }
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Count what reaches zone 1.
    let plays = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    {
        let mut rx1 = dev.zone_rx.remove(&1).unwrap();
        let plays = plays.clone();
        tokio::spawn(async move {
            while rx1.recv().await.is_some() {
                plays.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            }
        });
    }

    // The dead client: it connects and then reads and acknowledges nothing.
    let server = dev.handle.local_addr().expect("server up");
    let dead = std::sync::Arc::new(connect_and_go_silent(server).await);

    // A burst of status changes, every one of them a telegram for the dead client...
    for v in 0..90 {
        set_zone_volume(&dev, 1, 10 + v, v % 2 == 0).await;
    }
    tokio::time::sleep(Duration::from_millis(1500)).await;
    // ... and a flood of group writes through a router, 300 datagrams.
    let router = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let play = CemiFrame::new_l_data(
        knx_rs_core::message::MessageCode::LDataInd,
        knx_rs_core::address::IndividualAddress::from_raw(0x1101),
        knx_rs_core::address::DestinationAddress::Group(ga("1/1/1")),
        knx_rs_core::types::Priority::Low,
        &[0x00, 0x81],
    );
    let routing = KnxIpFrame::routing_indication(play.as_bytes()).to_bytes();
    for _ in 0..300 {
        router.send_to(&routing, server).await.unwrap();
    }

    // The client is still dead. The device task is not stuck with it: the management call answers
    // and the zone gets the commands the router sent.
    let t = std::time::Instant::now();
    let prog = timeout(Duration::from_secs(4), dev.handle.get_prog_mode())
        .await
        .expect("the programming-mode call returns");
    assert!(prog.is_ok(), "and answers: {prog:?}");
    assert!(
        t.elapsed() < Duration::from_secs(3),
        "took {:?}",
        t.elapsed()
    );

    // After a while the client acknowledges again, as in the case that wedged the server.
    tokio::time::sleep(Duration::from_secs(8)).await;
    let acker = spawn_acker(dead.clone());

    // The device is alive: a fresh client connects, and its command arrives.
    let started = std::time::Instant::now();
    let mut fresh = timeout(Duration::from_secs(40), async {
        loop {
            if let Some(addr) = dev.handle.local_addr()
                && let Ok(Ok(c)) =
                    timeout(Duration::from_secs(3), TunnelConnection::connect(addr)).await
            {
                return c;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    })
    .await
    .expect("a new client can connect after the old one acknowledges again");
    eprintln!(
        "alive again {:?} after the client recovered",
        started.elapsed()
    );
    // A knx-rs client acknowledges the server's frames only while it is receiving.
    drain(&mut fresh).await;
    let before = plays.load(std::sync::atomic::Ordering::SeqCst);
    timeout(Duration::from_secs(20), async {
        loop {
            fresh
                .group_write_value(ga("1/1/1"), DPT_SWITCH, &true.into())
                .await
                .ok();
            if plays.load(std::sync::atomic::Ordering::SeqCst) > before {
                return;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    })
    .await
    .expect("commands are dispatched again");
    assert!(
        plays.load(std::sync::atomic::Ordering::SeqCst) > 0,
        "the router's commands were not all lost"
    );

    let t = std::time::Instant::now();
    dev.bridge.shutdown().await;
    assert!(
        t.elapsed() < Duration::from_secs(3),
        "shutdown took {:?}",
        t.elapsed()
    );
    acker.abort();
}

/// A port that is taken when the device starts (knxd on the host, a second instance, no
/// multicast route yet at boot) must not disable KNX for good: the device keeps trying, answers
/// the programming-mode API meanwhile, and works once the server is up.
#[tokio::test]
async fn device_starts_serving_once_its_port_frees_up() {
    let blocker = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    let addr = blocker.local_addr().unwrap();

    let tmp = tempfile::tempdir().unwrap();
    let cfg = ets_style_config("", "");
    let mut dev = start_device_on(&cfg, tmp.path().join("knx-memory.bin"), addr).await;
    assert_eq!(
        dev.handle.local_addr(),
        None,
        "no server while the port is taken"
    );

    // The management API does not wait for the server.
    assert!(!dev.handle.get_prog_mode().await.unwrap());
    dev.handle.set_prog_mode(true).await.unwrap();
    assert!(dev.handle.get_prog_mode().await.unwrap());

    drop(blocker);
    timeout(T, async {
        while dev.handle.local_addr().is_none() {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("the device server started after the port was released");

    let mut ets = connect(&dev).await;
    ets_download(&mut ets, &ets_links()).await;
    drain(&mut ets).await;
    ets.group_write_value(ga("1/1/1"), DPT_SWITCH, &true.into())
        .await
        .unwrap();
    let cmd = timeout(T, dev.zone_rx.get_mut(&1).unwrap().recv()).await;
    assert!(matches!(cmd, Ok(Some(ZoneCommand::Play))), "{cmd:?}");
}

// ── Individual address ────────────────────────────────────────

/// The individual address of the device, as it answers `A_IndividualAddress_Read` in
/// programming mode (the source of the response).
async fn individual_address_of(dev: &Device, ets: &mut TunnelConnection) -> u16 {
    dev.handle.set_prog_mode(true).await.unwrap();
    let [hi, lo] = apci_hdr(ApduType::IndividualAddressRead);
    ets.send(broadcast_frame(&[hi, lo])).await.unwrap();
    timeout(T, async {
        loop {
            let frame = ets.recv().await.expect("tunnel open");
            if frame.as_group_write().is_none() {
                return frame.source_address().raw();
            }
        }
    })
    .await
    .expect("individual address response")
}

/// ETS assigns an individual address in programming mode and downloads to it. After the
/// restart that follows the download, the device must still answer to that address.
#[tokio::test]
async fn individual_address_assigned_by_ets_survives_a_restart() {
    const ASSIGNED: u16 = 0x1201; // 1.2.1

    let tmp = tempfile::tempdir().unwrap();
    let memory = tmp.path().join("knx-memory.bin");
    let cfg = ets_style_config("", "");
    {
        let dev = start_device(&cfg, memory.clone()).await;
        let mut ets = connect(&dev).await;
        assert_eq!(individual_address_of(&dev, &mut ets).await, DEVICE_IA);

        dev.handle.set_prog_mode(true).await.unwrap();
        let [hi, lo] = apci_hdr(ApduType::IndividualAddressWrite);
        ets.send(broadcast_frame(&[hi, lo, 0x12, 0x01]))
            .await
            .unwrap();
        assert_eq!(individual_address_of(&dev, &mut ets).await, ASSIGNED);

        // The download that follows addresses the new address.
        MANAGED_IA.with(|ia| ia.set(ASSIGNED));
        ets_download(&mut ets, &ets_links()).await;
        drain(&mut ets).await;
        dev.bridge.shutdown().await;
    }
    timeout(T, async {
        while !memory.exists() {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("ETS programming persisted");

    // Restart with the same configuration: the configured address is still 1.1.100.
    let dev = start_device(&cfg, memory).await;
    let mut ets = connect(&dev).await;
    assert_eq!(
        individual_address_of(&dev, &mut ets).await,
        ASSIGNED,
        "the address ETS assigned is restored"
    );
}
