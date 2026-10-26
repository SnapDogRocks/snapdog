// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! Device-mode KNX transport — runs as ETS-programmable KNX/IP device.
//!
//! Wraps `DeviceServer` + `Bau`. The BAU runs in a dedicated task;
//! the transport communicates via channels.

use std::net::Ipv4Addr;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;

use anyhow::{Context, Result};
use knx_rs_core::address::{GroupAddress, IndividualAddress};
use knx_rs_core::cemi::CemiFrame;
use knx_rs_core::dpt::{Dpt, DptValue};
use knx_rs_device::bau::Bau;
use knx_rs_device::device_object;
use knx_rs_ip::tunnel_server::{DeviceServer, ServerEvent};
use tokio::sync::{Notify, mpsc, oneshot, watch};

use super::group_objects::mem;
use super::outbound::{Outbound, Throttle};
use super::supervisor::{
    BACKOFF_BASE, BACKOFF_CAP, Backoff, delay_after_session, stop_requested, warn_on_failure,
};
use super::transport::{DeviceUnavailable, Incoming, Target};

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

/// Buffer size in bytes of a group object before its DPT is set (the third argument of
/// `Bau::new`). Every object gets its DPT in [`build_bau`], which grows the buffer as needed.
const DEFAULT_GO_SIZE: usize = 1;

/// Channel buffer size for BAU command/update channels.
const BAU_CHANNEL_CAPACITY: usize = 64;

/// Debounce delay after ETS memory changes before persisting to disk.
const ETS_PERSIST_DEBOUNCE: std::time::Duration = std::time::Duration::from_secs(2);

/// Path for persisted ETS memory.
const ETS_MEMORY_PATH: &str = "knx-memory.bin";

// ── Persistence format ────────────────────────────────────────
//
// Version 2: [magic 4B] [version 1B] [length 2B (LE)] [payload …] [configured IA 2B (LE)]
//            [assigned IA 2B (LE)] [crc32 4B (LE) over payload and both addresses]
// Version 1 (read only): the same without the two addresses; the CRC covers the payload.
// Atomic write via temp file + rename.

/// Magic bytes identifying a `SnapDog` KNX memory file.
const PERSIST_MAGIC: &[u8; 4] = b"SDKM";

/// Current persistence format version. Bump when memory layout changes.
const PERSIST_VERSION: u8 = 2;

/// The first format, without the individual address record.
const PERSIST_VERSION_1: u8 = 1;

/// Header size: magic (4) + version (1) + length (2).
const PERSIST_HEADER: usize = 7;

/// Size of the individual address record in version 2.
const PERSIST_IA_RECORD: usize = 4;

/// The individual address a device has, and the one it was configured with when it got it.
///
/// ETS assigns an individual address over the bus (`A_IndividualAddress_Write` in
/// programming mode) and then addresses every management service to it. The device stack
/// keeps it in RAM only, so it is saved with the programming: without it the device would
/// come back after the restart that ends a download under its configured address, and the
/// next ETS session could not reach it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct IaRecord {
    /// Raw address from the configuration (or `--knx-address`) at the time of saving.
    configured: u16,
    /// Raw address the device had at the time of saving.
    assigned: u16,
}

/// What a memory file holds.
struct Persisted {
    /// The saved BAU state.
    payload: Vec<u8>,
    /// The individual address record (version 2 files).
    ia: Option<IaRecord>,
}

/// Numbers the snapshots in the order they were taken, so that a slow writer holding an older
/// snapshot cannot replace a newer file.
static PERSIST_GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// The newest generation written per file.
static PERSIST_WRITTEN: std::sync::Mutex<Vec<(PathBuf, u64)>> = std::sync::Mutex::new(Vec::new());

/// Take the number for a snapshot about to be persisted.
fn next_generation() -> u64 {
    PERSIST_GENERATION.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
}

/// Write ETS memory to disk with integrity envelope (atomic).
fn persist_memory(path: &std::path::Path, payload: &[u8], ia: IaRecord) -> std::io::Result<()> {
    persist_generation(path, payload, ia, next_generation())
}

/// [`persist_memory`] for a snapshot taken as `generation`: written unless a newer snapshot of
/// the same file went to disk already.
fn persist_generation(
    path: &std::path::Path,
    payload: &[u8],
    ia: IaRecord,
    generation: u64,
) -> std::io::Result<()> {
    let len = u16::try_from(payload.len()).map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!(
                "BAU state of {} bytes does not fit the memory file",
                payload.len()
            ),
        )
    })?;
    let mut record = Vec::with_capacity(PERSIST_IA_RECORD);
    record.extend_from_slice(&ia.configured.to_le_bytes());
    record.extend_from_slice(&ia.assigned.to_le_bytes());
    let mut crc = crc32fast::Hasher::new();
    crc.update(payload);
    crc.update(&record);

    let mut buf = Vec::with_capacity(PERSIST_HEADER + payload.len() + PERSIST_IA_RECORD + 4);
    buf.extend_from_slice(PERSIST_MAGIC);
    buf.push(PERSIST_VERSION);
    buf.extend_from_slice(&len.to_le_bytes());
    buf.extend_from_slice(payload);
    buf.extend_from_slice(&record);
    buf.extend_from_slice(&crc.finalize().to_le_bytes());

    // One writer at a time per process; a snapshot older than the file is dropped.
    let mut written = PERSIST_WRITTEN
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let newer = written.iter().any(|(p, g)| p == path && *g > generation);
    if newer {
        tracing::debug!(path = %path.display(), generation, "ETS state snapshot is older than the file, not written");
        return Ok(());
    }
    write_atomically(path, &buf)?;
    if let Some(entry) = written.iter_mut().find(|(p, _)| p == path) {
        entry.1 = generation;
    } else {
        written.push((path.to_path_buf(), generation));
    }
    drop(written);
    Ok(())
}

/// Replace `path` with `buf` so that a crash leaves either the old or the new file: a temp file
/// of its own (two writers never share one), written with owner-only permissions (the file holds
/// the MQTT, `AirPlay` and Subsonic passwords, the PSK and the API keys ETS parameters carry),
/// flushed to disk, renamed over the target, and the rename flushed through the directory.
fn write_atomically(path: &std::path::Path, buf: &[u8]) -> std::io::Result<()> {
    use std::io::Write as _;

    static TEMP_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let tmp = path.with_extension(format!(
        "tmp.{}.{}",
        std::process::id(),
        TEMP_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let result = (|| {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        let mut file = options.open(&tmp)?;
        file.write_all(buf)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&tmp, path)?;
        // Make the rename itself durable. Not every platform can open a directory.
        #[cfg(unix)]
        if let Some(dir) = path.parent().map(|d| {
            if d.as_os_str().is_empty() {
                std::path::Path::new(".")
            } else {
                d
            }
        }) && let Ok(dir) = std::fs::File::open(dir)
        {
            let _ = dir.sync_all();
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// Load ETS memory from disk, verifying magic, version, length, and CRC32.
fn load_memory(path: &std::path::Path) -> Result<Persisted> {
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
    let version = data[4];
    anyhow::ensure!(
        version == PERSIST_VERSION || version == PERSIST_VERSION_1,
        "unsupported version {version} (expected {PERSIST_VERSION})"
    );
    let record_len = if version == PERSIST_VERSION {
        PERSIST_IA_RECORD
    } else {
        0
    };

    let len = u16::from_le_bytes([data[5], data[6]]) as usize;
    anyhow::ensure!(
        data.len() == PERSIST_HEADER + len + record_len + 4,
        "length mismatch: header says {len} bytes, file has {}",
        data.len().saturating_sub(PERSIST_HEADER + record_len + 4)
    );

    let covered = &data[PERSIST_HEADER..PERSIST_HEADER + len + record_len];
    let crc_at = PERSIST_HEADER + len + record_len;
    let stored_crc = u32::from_le_bytes([
        data[crc_at],
        data[crc_at + 1],
        data[crc_at + 2],
        data[crc_at + 3],
    ]);
    let actual_crc = crc32(covered);
    anyhow::ensure!(
        stored_crc == actual_crc,
        "CRC32 mismatch (file corrupted): stored {stored_crc:#010x}, computed {actual_crc:#010x}"
    );

    let ia = (record_len > 0).then(|| {
        let r = &data[PERSIST_HEADER + len..PERSIST_HEADER + len + PERSIST_IA_RECORD];
        IaRecord {
            configured: u16::from_le_bytes([r[0], r[1]]),
            assigned: u16::from_le_bytes([r[2], r[3]]),
        }
    });
    Ok(Persisted {
        payload: data[PERSIST_HEADER..PERSIST_HEADER + len].to_vec(),
        ia,
    })
}

/// CRC32 (ISO 3309 / ITU-T V.42 — same as zlib/PNG).
fn crc32(data: &[u8]) -> u32 {
    crc32fast::hash(data)
}

use super::group_objects::{
    self, CGO_CONNECTED, CGO_LATENCY, CGO_LATENCY_STATUS, CGO_MUTE, CGO_MUTE_STATUS,
    CGO_MUTE_TOGGLE, CGO_VOLUME, CGO_VOLUME_DIM, CGO_VOLUME_STATUS, CGO_ZONE, CGO_ZONE_STATUS,
    CLIENT_GOS, GGO_ALL_MUTE, GGO_ALL_STOP, GGO_KNX_TIME, GGO_SERVER_ONLINE, GGO_SYSTEM_FAULT,
    MAX_API_KEYS, MAX_CLIENTS, MAX_ZONES, TOTAL_GO_COUNT, ZGO_CONTROL_STATUS, ZGO_MUTE,
    ZGO_MUTE_STATUS, ZGO_MUTE_TOGGLE, ZGO_PAUSE, ZGO_PLAY, ZGO_PLAYLIST, ZGO_PLAYLIST_NEXT,
    ZGO_PLAYLIST_PREVIOUS, ZGO_PLAYLIST_STATUS, ZGO_PRESENCE, ZGO_PRESENCE_ENABLE,
    ZGO_PRESENCE_TIMER_ACTIVE, ZGO_REPEAT, ZGO_REPEAT_STATUS, ZGO_REPEAT_TOGGLE, ZGO_SHUFFLE,
    ZGO_SHUFFLE_STATUS, ZGO_SHUFFLE_TOGGLE, ZGO_STOP, ZGO_TRACK_ALBUM, ZGO_TRACK_ARTIST,
    ZGO_TRACK_NEXT, ZGO_TRACK_PLAYING, ZGO_TRACK_PREVIOUS, ZGO_TRACK_PROGRESS, ZGO_TRACK_REPEAT,
    ZGO_TRACK_REPEAT_STATUS, ZGO_TRACK_REPEAT_TOGGLE, ZGO_TRACK_TITLE, ZGO_VOLUME, ZGO_VOLUME_DIM,
    ZGO_VOLUME_STATUS, ZONE_GOS,
};

/// Fixed-size array of `MAX_RADIOS` values. A newtype because Rust only derives
/// `Default` for arrays up to length 32, and `MAX_RADIOS` is 50. `Deref`/`DerefMut`
/// to the inner array make indexing, `iter()` and assignment transparent.
#[derive(Debug)]
pub struct Radios<T>([T; mem::MAX_RADIOS]);

impl<T: Default> Default for Radios<T> {
    fn default() -> Self {
        Self(std::array::from_fn(|_| T::default()))
    }
}
impl<T> std::ops::Deref for Radios<T> {
    type Target = [T; mem::MAX_RADIOS];
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl<T> std::ops::DerefMut for Radios<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

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
    /// Global Snapcast output sample rate in Hz (decoded from the ETS enum index).
    pub sample_rate: u32,
    /// Global Snapcast output bit depth (decoded from the ETS enum index).
    pub bit_depth: u16,
    /// Global Snapcast codec index (0=pcm, 1=flac, 2=f32lz4, 3=f32lz4e).
    pub codec: u8,
    /// Global source-conflict policy index (0 = `last_wins`, 1 = `receiver_wins`).
    pub source_conflict: u8,
    /// Global zone-switch fade duration in ms.
    pub zone_switch_fade: u16,
    /// Global source-switch fade duration in ms.
    pub source_switch_fade: u16,
    /// Interval of the cyclic "Server Online" object in minutes (1, 3, 5, 10, 15, 30, 45 or
    /// 60; the ETS dropdown stores the minutes themselves). 5 when the memory holds none.
    pub heartbeat_minutes: u16,
    /// Per-zone presence default-source index (0 = none, 1..=`MAX_RADIOS` = radio index).
    pub zone_presence_source: [u8; MAX_ZONES],
    pub radio_active: Radios<bool>,
    // Strings
    pub zone_names: [String; MAX_ZONES],
    /// Per-zone `AirPlay` device names (empty = derive from zone name).
    pub zone_airplay_names: [String; MAX_ZONES],
    /// Per-zone Spotify device names (empty = derive from zone name).
    pub zone_spotify_names: [String; MAX_ZONES],
    pub client_names: [String; MAX_CLIENTS],
    pub client_macs: [String; MAX_CLIENTS],
    /// Per-client icons (empty = default icon).
    pub client_icons: [String; MAX_CLIENTS],
    pub subsonic_url: String,
    pub subsonic_user: String,
    pub subsonic_pass: String,
    pub mqtt_broker: String,
    pub mqtt_topic: String,
    /// MQTT password (secret, plaintext in ETS memory by design).
    pub mqtt_pass: String,
    /// `AirPlay` password (secret, plaintext in ETS memory by design).
    pub airplay_pass: String,
    /// Snapcast encryption PSK (secret, plaintext in ETS memory by design).
    pub psk: String,
    /// HTTP API keys (secrets, plaintext in ETS memory by design; empty slots dropped).
    pub api_keys: Vec<String>,
    pub radio_names: Radios<String>,
    pub radio_urls: Radios<String>,
    /// Per-radio cover-art URLs.
    pub radio_covers: Radios<String>,
}

/// Decode ISO 8859-15 (Latin-9), the text parameter encoding the product database declares
/// (`TextParameterEncoding`). It is Latin-1 except for eight positions.
fn decode_latin9(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|&b| match b {
            0xA4 => '\u{20AC}', // €
            0xA6 => '\u{0160}', // Š
            0xA8 => '\u{0161}', // š
            0xB4 => '\u{017D}', // Ž
            0xB8 => '\u{017E}', // ž
            0xBC => '\u{0152}', // Œ
            0xBD => '\u{0153}', // œ
            0xBE => '\u{0178}', // Ÿ
            _ => char::from(b),
        })
        .collect()
}

/// Read a null-terminated or fixed-length string from a byte slice.
fn read_string(data: &[u8], offset: usize, max_len: usize) -> String {
    if offset + max_len > data.len() {
        return String::new();
    }
    let bytes = &data[offset..offset + max_len];
    // Find null terminator or use full length
    let len = bytes.iter().position(|&b| b == 0).unwrap_or(max_len);
    decode_latin9(&bytes[..len])
}

/// A point-to-point `L_Data.ind` management frame from [`INSTALL_SOURCE`] to the device.
fn management_frame(bau: &Bau, apdu: &[u8]) -> Option<CemiFrame> {
    let dst = bau.individual_address().raw();
    let mut f = vec![
        0x29,
        0x00,
        0xB0,
        0x60,
        (INSTALL_SOURCE >> 8) as u8,
        (INSTALL_SOURCE & 0xFF) as u8,
        (dst >> 8) as u8,
        (dst & 0xFF) as u8,
        u8::try_from(apdu.len().checked_sub(1)?).ok()?,
    ];
    f.extend_from_slice(apdu);
    CemiFrame::parse(&f).ok()
}

/// Whether the BAU holds a programming ETS downloaded (as opposed to none, or tables snapdog
/// installed from the TOML addresses, which leave the memory area empty).
fn ets_programmed(bau: &Bau) -> bool {
    bau.configured() && !bau.memory_area().is_empty()
}

/// The BAU state to persist.
///
/// A device ETS never programmed saves an empty state: its tables are installed from the
/// configuration at every boot, and saved table states would make the next boot take empty
/// tables for an ETS programming. After a download the state is compacted (see
/// [`compact_state`]).
fn persisted_state(bau: &mut Bau) -> Vec<u8> {
    if bau.memory_area().is_empty() {
        return build_bau(bau.individual_address()).save();
    }
    compact_state(bau).unwrap_or_else(|| bau.save())
}

/// The state of a fully programmed BAU, with the memory laid out as a first download would
/// leave it: the application parameters, then the address table, then the association table.
///
/// The device stack appends the segments of every download and never reclaims the old ones, so
/// the live memory grows by about 12 KB per download. Saved as it is, the file would grow with
/// it (and its 16-bit length field wrap after a handful of downloads). Only the segments the
/// tables currently use are kept, and boot-time parsing finds the parameters at offset 0 again.
/// Returns `None` when the BAU is not fully programmed; the caller saves it as it is.
///
/// The record layout is the one documented at [`Bau::save`]: three table object records of
/// state, offset and size, the five program version bytes, the memory length and the memory.
fn compact_state(bau: &mut Bau) -> Option<Vec<u8>> {
    /// A table object record: `LoadState::Loaded`, offset and size.
    fn record(offset: usize, size: usize) -> Option<[u8; 9]> {
        let mut r = [0u8; 9];
        r[0] = 1;
        r[1..5].copy_from_slice(&u32::try_from(offset).ok()?.to_le_bytes());
        r[5..9].copy_from_slice(&u32::try_from(size).ok()?.to_le_bytes());
        Some(r)
    }
    const RECORDS: usize = 3 * 9;
    const VERSION: usize = 5;

    let app_at = app_segment(bau)?;
    let memory = bau.memory_area();
    let app = memory.get(app_at..app_at.checked_add(mem::TOTAL)?)?;
    let addr = bau.addr_table_object().data(memory);
    let assoc = bau.assoc_table_object().data(memory);
    if addr.is_empty() || assoc.is_empty() {
        return None;
    }
    let full = bau.save();
    let version = full.get(RECORDS..RECORDS + VERSION)?;

    let mut compact = Vec::with_capacity(app.len() + addr.len() + assoc.len());
    compact.extend_from_slice(app);
    compact.extend_from_slice(addr);
    compact.extend_from_slice(assoc);

    let mut blob = Vec::with_capacity(RECORDS + VERSION + 4 + compact.len());
    blob.extend_from_slice(&record(mem::TOTAL, addr.len())?); // address table object
    blob.extend_from_slice(&record(mem::TOTAL + addr.len(), assoc.len())?); // association table object
    blob.extend_from_slice(&record(0, mem::TOTAL)?); // application program object
    blob.extend_from_slice(version);
    blob.extend_from_slice(&u32::try_from(compact.len()).ok()?.to_le_bytes());
    blob.extend_from_slice(&compact);
    Some(blob)
}

/// Where the application program's parameter block starts in the BAU memory.
///
/// The device stack hands out memory by appending: every download, and every
/// parameter-only download, allocates its segment behind everything allocated before. The
/// parameters therefore start at offset 0 after the first download only. ETS reads the
/// segment address with `PID_TABLE_REFERENCE` of the application program object before it
/// writes there, and so does this, through the device stack's own management service.
fn app_segment(bau: &mut Bau) -> Option<usize> {
    use knx_rs_core::message::ApduType;

    const OBJ_APP_PROGRAM: u8 = 4;
    const PID_TABLE_REFERENCE: u8 = 7;

    if !bau.configured() {
        return None;
    }
    // Between events the outbox is empty; the response is the only frame after this.
    while bau.next_outgoing_frame().is_some() {}
    let [hi, lo] = {
        let b = (ApduType::PropertyValueRead as u16).to_be_bytes();
        [b[0] & 0x03, b[1]]
    };
    let frame = management_frame(
        bau,
        &[hi, lo, OBJ_APP_PROGRAM, PID_TABLE_REFERENCE, 0x10, 0x01],
    )?;
    bau.process_frame(&frame, now_ms());
    let response = bau.next_outgoing_frame()?;
    while bau.next_outgoing_frame().is_some() {}
    let payload = response.payload();
    let at = payload.len().checked_sub(4)?;
    let reference = u32::from_be_bytes(payload[at..].try_into().ok()?);
    usize::try_from(reference).ok()
}

/// The ETS parameters of the programming the BAU holds, read from the segment the last
/// download wrote.
fn app_parameters(bau: &mut Bau) -> Option<EtsParams> {
    let at = app_segment(bau)?;
    let block = bau.memory_area().get(at..at.checked_add(mem::TOTAL)?)?;
    Some(parse_ets_memory(block))
}

/// Parse ETS parameters from BAU memory area.
#[allow(clippy::too_many_lines)]
pub fn parse_ets_memory(data: &[u8]) -> EtsParams {
    let mut p = EtsParams::default();
    if data.len() < mem::TOTAL {
        return p;
    }
    // Numeric — zones. A single "number of zones" byte replaces the former per-zone active
    // flags: zones 1..=NUM_ZONES are active (contiguous), the rest are off.
    let num_zones = usize::from(data[mem::NUM_ZONES]).min(MAX_ZONES);
    for i in 0..MAX_ZONES {
        p.zone_active[i] = i < num_zones;
        p.zone_default_volume[i] = data[mem::ZONE_DEF_VOL + i];
        p.zone_max_volume[i] = data[mem::ZONE_MAX_VOL + i];
        p.zone_airplay[i] = data[mem::ZONE_AIRPLAY + i] != 0;
        p.zone_spotify[i] = data[mem::ZONE_SPOTIFY + i] != 0;
        p.zone_presence_enabled[i] = data[mem::ZONE_PRESENCE_EN + i] != 0;
        let to_off = mem::ZONE_PRESENCE_TO + i * 2;
        p.zone_presence_timeout[i] = u16::from_be_bytes([data[to_off], data[to_off + 1]]);
        p.zone_presence_source[i] = data[mem::ZONE_PRES_SRC + i];
    }
    // Numeric — clients. Like zones, a single count byte replaces the former per-client
    // active flags: clients 1..=NUM_CLIENTS are active (contiguous), the rest are off.
    let num_clients = usize::from(data[mem::NUM_CLIENTS]).min(MAX_CLIENTS);
    for i in 0..MAX_CLIENTS {
        p.client_active[i] = i < num_clients;
        p.client_default_zone[i] = data[mem::CLIENT_DEF_ZONE + i];
        p.client_default_volume[i] = data[mem::CLIENT_DEF_VOL + i];
        p.client_max_volume[i] = data[mem::CLIENT_MAX_VOL + i];
        p.client_default_latency[i] = data[mem::CLIENT_DEF_LAT + i];
    }
    // Numeric — global
    p.http_port =
        u16::from_be_bytes([data[mem::GLOBAL_HTTP_PORT], data[mem::GLOBAL_HTTP_PORT + 1]]);
    p.log_level = data[mem::GLOBAL_LOG_LVL];
    // Global audio format (server-wide Snapcast output), decoded from the ETS enum index.
    p.sample_rate = match data[mem::GLOBAL_SRATE] {
        0 => 44_100,
        2 => 88_200,
        3 => 96_000,
        4 => 176_400,
        5 => 192_000,
        _ => 48_000,
    };
    p.bit_depth = match data[mem::GLOBAL_BITD] {
        1 => 24,
        2 => 32,
        _ => 16,
    };
    p.codec = data[mem::GLOBAL_CODEC];
    p.source_conflict = data[mem::GLOBAL_SRC_CONFLICT];
    p.zone_switch_fade =
        u16::from_be_bytes([data[mem::GLOBAL_ZONE_FADE], data[mem::GLOBAL_ZONE_FADE + 1]]);
    p.source_switch_fade =
        u16::from_be_bytes([data[mem::GLOBAL_SRC_FADE], data[mem::GLOBAL_SRC_FADE + 1]]);
    p.heartbeat_minutes = match data[mem::GLOBAL_HEARTBEAT] {
        0 => 5, // an unprogrammed byte; the ETS dropdown has no 0
        m => u16::from(m),
    };
    // Radios: a single count byte replaces the former per-radio active flags —
    // radios 1..=NUM_RADIOS are active (contiguous), the rest are off.
    let num_radios = usize::from(data[mem::NUM_RADIOS]).min(mem::MAX_RADIOS);
    for i in 0..mem::MAX_RADIOS {
        p.radio_active[i] = i < num_radios;
    }
    // Strings
    for i in 0..MAX_ZONES {
        p.zone_names[i] = read_string(
            data,
            mem::ZONE_NAME + i * mem::ZONE_NAME_SIZE,
            mem::ZONE_NAME_SIZE,
        );
        p.zone_airplay_names[i] = read_string(
            data,
            mem::ZONE_AIRPLAY_NAME + i * mem::ZONE_AIRPLAY_NAME_SIZE,
            mem::ZONE_AIRPLAY_NAME_SIZE,
        );
        p.zone_spotify_names[i] = read_string(
            data,
            mem::ZONE_SPOTIFY_NAME + i * mem::ZONE_SPOTIFY_NAME_SIZE,
            mem::ZONE_SPOTIFY_NAME_SIZE,
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
        p.client_icons[i] = read_string(
            data,
            mem::CLIENT_ICON + i * mem::CLIENT_ICON_SIZE,
            mem::CLIENT_ICON_SIZE,
        );
    }
    p.subsonic_url = read_string(data, mem::GLOBAL_SUB_URL, mem::GLOBAL_SUB_URL_SIZE);
    p.subsonic_user = read_string(data, mem::GLOBAL_SUB_USER, mem::GLOBAL_SUB_USER_SIZE);
    p.subsonic_pass = read_string(data, mem::GLOBAL_SUB_PASS, mem::GLOBAL_SUB_PASS_SIZE);
    p.mqtt_broker = read_string(data, mem::GLOBAL_MQTT_BROKER, mem::GLOBAL_MQTT_BROKER_SIZE);
    p.mqtt_topic = read_string(data, mem::GLOBAL_MQTT_TOPIC, mem::GLOBAL_MQTT_TOPIC_SIZE);
    // Secrets (plaintext in ETS memory by design — see RFC / product decision).
    p.mqtt_pass = read_string(data, mem::GLOBAL_MQTT_PASS, mem::GLOBAL_MQTT_PASS_SIZE);
    p.airplay_pass = read_string(
        data,
        mem::GLOBAL_AIRPLAY_PASS,
        mem::GLOBAL_AIRPLAY_PASS_SIZE,
    );
    p.psk = read_string(data, mem::GLOBAL_PSK, mem::GLOBAL_PSK_SIZE);
    // API keys 1..=count are active (mirrors zone/client counts); empty slots are dropped.
    let num_api_keys = usize::from(data[mem::GLOBAL_NUM_API_KEYS]).min(MAX_API_KEYS);
    p.api_keys = (0..num_api_keys)
        .map(|i| {
            read_string(
                data,
                mem::GLOBAL_API_KEYS + i * mem::GLOBAL_API_KEY_SIZE,
                mem::GLOBAL_API_KEY_SIZE,
            )
        })
        .filter(|k| !k.is_empty())
        .collect();
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
        p.radio_covers[i] = read_string(
            data,
            mem::RADIO_COVER + i * mem::RADIO_COVER_SIZE,
            mem::RADIO_COVER_SIZE,
        );
    }
    p
}

/// Command sent to the BAU task.
enum BauCmd {
    /// Write a value to a group object. The programmed associations decide the address.
    WriteObject { asap: u16, value: DptValue },
    /// Set programming mode on/off.
    SetProgMode { enabled: bool },
    /// Get current programming mode state.
    GetProgMode { reply: oneshot::Sender<bool> },
    /// Let the KNXnet/IP server end, as if its socket had failed (supervision tests).
    #[cfg(test)]
    RestartServer,
}

/// Device-mode publisher: sends values to the BAU task.
#[derive(Clone)]
pub struct DevicePublisher {
    cmd_tx: mpsc::Sender<BauCmd>,
    local_addr: watch::Receiver<Option<std::net::SocketAddr>>,
    resync: Arc<Notify>,
}

/// How long a management call (programming mode) and a status write wait for the device task.
const DEVICE_CALL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

impl DevicePublisher {
    /// UDP address the KNXnet/IP device server is bound to, `None` while it is not up.
    #[must_use]
    pub fn local_addr(&self) -> Option<std::net::SocketAddr> {
        *self.local_addr.borrow()
    }

    /// Queue a command for the device task; `false` when it does not take it in time.
    async fn send_cmd(&self, cmd: BauCmd) -> bool {
        matches!(
            tokio::time::timeout(DEVICE_CALL_TIMEOUT, self.cmd_tx.send(cmd)).await,
            Ok(Ok(()))
        )
    }
}

impl super::transport::KnxDeviceControl for DevicePublisher {
    async fn set_prog_mode(&self, enabled: bool) -> Result<(), DeviceUnavailable> {
        if self.send_cmd(BauCmd::SetProgMode { enabled }).await {
            Ok(())
        } else {
            Err(DeviceUnavailable)
        }
    }

    async fn get_prog_mode(&self) -> Result<bool, DeviceUnavailable> {
        let (tx, rx) = oneshot::channel();
        if !self.send_cmd(BauCmd::GetProgMode { reply: tx }).await {
            return Err(DeviceUnavailable);
        }
        match tokio::time::timeout(DEVICE_CALL_TIMEOUT, rx).await {
            Ok(Ok(enabled)) => Ok(enabled),
            _ => Err(DeviceUnavailable),
        }
    }
}

/// Device-mode listener: receives group object updates from the BAU task.
pub struct DeviceListener {
    update_rx: mpsc::Receiver<(u16, Vec<u8>)>,
}

/// Overrides for the device transport's environment.
///
/// Production uses the defaults (UDP 3671 plus the KNX multicast group, memory file in the
/// working directory). Tests bind a loopback port and keep the memory file in a temp dir.
#[derive(Debug, Clone, Default)]
pub struct DeviceOptions {
    /// Bind the KNXnet/IP server to this unicast address instead of `0.0.0.0:3671`.
    /// No multicast group is joined in that case.
    pub bind: Option<std::net::SocketAddr>,
    /// Path of the persisted ETS memory (default: `knx-memory.bin` in the working directory).
    pub memory_path: Option<PathBuf>,
}

/// Start the KNXnet/IP server (UDP 3671 plus multicast, or the test address).
async fn open_server(bind: Option<std::net::SocketAddr>) -> Result<DeviceServer> {
    match bind {
        Some(addr) => DeviceServer::start_at(addr).await,
        None => DeviceServer::start(Ipv4Addr::UNSPECIFIED).await,
    }
    .context("Failed to start KNX device server")
}

/// What the BAU holds in its address and association tables, for the startup log.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct TableSummary {
    /// Group addresses in the address table.
    pub group_addresses: u16,
    /// Entries in the association table.
    pub associations: u16,
    /// Zone objects with at least one association.
    pub zone_objects: usize,
    /// Client objects with at least one association.
    pub client_objects: usize,
    /// Device-level objects with at least one association.
    pub global_objects: usize,
    /// Whether the BAU takes part in group communication (tables and application loaded).
    pub configured: bool,
}

impl TableSummary {
    /// Read the counts from `bau`.
    pub(super) fn of(bau: &Bau) -> Self {
        let zone_end = (MAX_ZONES * group_objects::ZONE_GO_COUNT) as u16;
        let client_end = zone_end + (MAX_CLIENTS * group_objects::CLIENT_GO_COUNT) as u16;
        let mut s = Self {
            group_addresses: bau.address_table().entry_count(),
            associations: bau.association_table().entry_count(),
            zone_objects: 0,
            client_objects: 0,
            global_objects: 0,
            configured: bau.configured(),
        };
        for asap in 1..=TOTAL_GO_COUNT as u16 {
            if bau.association_table().translate_asap(asap).is_none() {
                continue;
            }
            if asap <= zone_end {
                s.zone_objects += 1;
            } else if asap <= client_end {
                s.client_objects += 1;
            } else {
                s.global_objects += 1;
            }
        }
        s
    }

    /// Log the summary, and a warning when nothing is associated.
    pub(super) fn log(&self, origin: &str) {
        tracing::info!(
            origin,
            group_addresses = self.group_addresses,
            associations = self.associations,
            zone_objects = self.zone_objects,
            client_objects = self.client_objects,
            global_objects = self.global_objects,
            configured = self.configured,
            "KNX device: group objects associated with group addresses"
        );
        // A download in progress has loaded the address table before the associations.
        let downloading = origin == "ETS download" && !self.configured;
        if self.associations == 0 && !downloading {
            tracing::warn!(
                "KNX device: no group object is associated with a group address, so snapdog \
                 neither reacts to nor sends group telegrams. Download the .knxprod from ETS \
                 to this device or configure [[zone]].knx / [[client]].knx addresses."
            );
        }
    }
}

/// The individual address to take from a memory file, if any.
///
/// The saved address wins when ETS assigned it under the configuration that is in force now.
/// A configuration that changed since (a different `individual_address` or `--knx-address`)
/// was changed on purpose and wins, so that an operator can always move the device by
/// editing the configuration.
fn restored_individual_address(saved: Option<IaRecord>, configured: u16) -> Option<u16> {
    let saved = saved?;
    (saved.configured == configured && saved.assigned != configured).then_some(saved.assigned)
}

/// Restore the persisted ETS programming and individual address into `bau`, if there is any.
fn restore_state(bau: &mut Bau, path: &std::path::Path) {
    match load_memory(path) {
        Ok(saved) => {
            let configured = bau.individual_address().raw();
            if let Some(ia) = restored_individual_address(saved.ia, configured) {
                tracing::info!(
                    assigned = %IndividualAddress::from_raw(ia),
                    configured = %IndividualAddress::from_raw(configured),
                    "Using the individual address ETS assigned"
                );
                device_object::set_individual_address(bau.device_mut(), ia);
            }
            if bau.restore(&saved.payload).is_ok() {
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

/// What [`start_device_transport`] hands back.
pub struct DeviceTransport {
    /// Sends status values and controls programming mode.
    pub publisher: DevicePublisher,
    /// Receives the group objects the bus updated.
    pub listener: DeviceListener,
    /// The supervisor task; it persists on its way out, so shutdown waits for it.
    pub task: tokio::task::JoinHandle<()>,
}

/// Start the device server and BAU, returning a publisher/listener pair.
///
/// The server is supervised: when it cannot be bound (yet) or stops later, it is started
/// again with backoff, and the bridge re-sends all status values once it is back.
pub async fn start_device_transport(
    individual_address: &str,
    config: &crate::config::AppConfig,
    options: &DeviceOptions,
    shutdown: watch::Receiver<bool>,
) -> Result<DeviceTransport> {
    let ia = IndividualAddress::from_str(individual_address)
        .map_err(|e| anyhow::anyhow!("Invalid individual address: {e}"))?;

    let knx = config.knx.as_ref().expect("KNX config required");
    let persist = knx.persist_ets_config.unwrap_or(true);
    let restart_after_ets = knx.restart_after_ets.unwrap_or(true);
    let persist_path = if persist {
        Some(
            options
                .memory_path
                .clone()
                .unwrap_or_else(|| PathBuf::from(ETS_MEMORY_PATH)),
        )
    } else {
        None
    };

    // Build BAU and restore state before spawning the task
    let mut bau = build_bau(ia);
    if let Some(ref path) = persist_path {
        restore_state(&mut bau, path);
    }

    let origin = if ets_programmed(&bau) {
        tracing::info!(
            "Using ETS-programmed group address tables (TOML KNX addresses are not used for the device tables)"
        );
        let ets = app_parameters(&mut bau).unwrap_or_default();
        tracing::info!(
            zones = ets.zone_active.iter().filter(|&&a| a).count(),
            clients = ets.client_active.iter().filter(|&&a| a).count(),
            radios = ets.radio_active.iter().filter(|&&a| a).count(),
            subsonic = !ets.subsonic_url.is_empty(),
            mqtt = !ets.mqtt_broker.is_empty(),
            "ETS parameters loaded"
        );
        "ETS programming"
    } else if build_tables_from_config(&mut bau, config) {
        tracing::info!("No ETS programming found — using group addresses from TOML config");
        "TOML configuration"
    } else {
        tracing::info!("No ETS programming and no TOML group addresses");
        "none"
    };
    TableSummary::of(&bau).log(origin);

    let (cmd_tx, cmd_rx) = mpsc::channel(BAU_CHANNEL_CAPACITY);
    let (update_tx, update_rx) = mpsc::channel(BAU_CHANNEL_CAPACITY);

    // Activate programming mode if requested via CLI
    if config.knx.as_ref().is_some_and(|k| k.start_prog_mode) {
        knx_rs_device::device_object::set_prog_mode(bau.device_mut(), true);
        tracing::info!("KNX programming mode active (--knx-prog-mode)");
    }

    // First bind inline so a caller (and the log) sees the result right away; a failure is
    // not fatal, the supervisor keeps trying.
    let (addr_tx, addr_rx) = watch::channel(None);
    let first = match open_server(options.bind).await {
        Ok(server) => {
            let _ = addr_tx.send(Some(server.local_addr()));
            Some(server)
        }
        Err(e) => {
            tracing::warn!(error = %format!("{e:#}"), "KNX device server could not start — retrying in the background");
            None
        }
    };

    let resync = Arc::new(Notify::new());
    let task = tokio::spawn(device_supervisor(
        DeviceTask {
            bau,
            cmd_rx,
            update_tx,
            persist_path,
            restart_after_ets,
            config_ia: ia.raw(),
            bind: options.bind,
            addr_tx,
            resync: Arc::clone(&resync),
            shutdown,
            pending_persist: None,
        },
        first,
    ));

    tracing::info!(%individual_address, persist, "KNX device mode started");

    Ok(DeviceTransport {
        publisher: DevicePublisher {
            cmd_tx,
            local_addr: addr_rx,
            resync,
        },
        listener: DeviceListener { update_rx },
        task,
    })
}

impl super::transport::KnxPublisher for DevicePublisher {
    const ADDRESSED: bool = false;

    async fn write(&self, target: Target, _dpt: Dpt, value: &DptValue) -> bool {
        let Target::Object(asap) = target else {
            return false;
        };
        // A device task that does not take commands must not stop the publisher: a value that is
        // not taken is offered again with the next change.
        self.send_cmd(BauCmd::WriteObject {
            asap,
            value: value.clone(),
        })
        .await
    }

    async fn resync(&self) {
        self.resync.notified().await;
    }
}

impl super::transport::KnxListener for DeviceListener {
    async fn recv(&mut self) -> Option<Incoming> {
        let (asap, data) = self.update_rx.recv().await?;
        Some(Incoming::Object(asap, data))
    }
}

// ── BAU task ──────────────────────────────────────────────────

/// Everything the BAU task owns across server restarts.
struct DeviceTask {
    bau: Bau,
    cmd_rx: mpsc::Receiver<BauCmd>,
    update_tx: mpsc::Sender<(u16, Vec<u8>)>,
    persist_path: Option<PathBuf>,
    restart_after_ets: bool,
    /// Individual address from the configuration, before ETS assigned another.
    config_ia: u16,
    bind: Option<std::net::SocketAddr>,
    addr_tx: watch::Sender<Option<std::net::SocketAddr>>,
    resync: Arc<Notify>,
    shutdown: watch::Receiver<bool>,
    /// The debounced write that may still be running; shutdown waits for it.
    pending_persist: Option<tokio::task::JoinHandle<()>>,
}

/// Why [`bau_task_loop`] returned.
#[derive(Debug, PartialEq, Eq)]
enum LoopEnd {
    /// The KNXnet/IP server stopped; start it again.
    ServerClosed,
    /// Nobody can send commands any more, or shutdown was requested.
    Stopped,
}

/// Keep the KNXnet/IP server running for the lifetime of the BAU.
async fn device_supervisor(task: DeviceTask, first: Option<DeviceServer>) {
    device_supervisor_with(task, first, RealLoop).await;
}

/// What the supervisor runs for as long as a server is up: the BAU loop, or in a test a stand-in
/// that lives for as long as the test says.
trait ServerLoop: Send {
    fn run(
        &mut self,
        task: &mut DeviceTask,
        server: DeviceServer,
    ) -> impl std::future::Future<Output = LoopEnd> + Send;
}

struct RealLoop;

impl ServerLoop for RealLoop {
    async fn run(&mut self, task: &mut DeviceTask, server: DeviceServer) -> LoopEnd {
        bau_task_loop(task, server).await
    }
}

/// [`device_supervisor`] with the BAU loop left open.
async fn device_supervisor_with(
    mut task: DeviceTask,
    first: Option<DeviceServer>,
    mut serve: impl ServerLoop,
) {
    let mut backoff = Backoff::new(BACKOFF_BASE, BACKOFF_CAP);
    let mut server = first;
    // The first bind happened (and failed) in start_device_transport: count it, and let
    // everything the publisher offered meanwhile go out once the server is up.
    let mut restarted = server.is_none();
    if restarted {
        let delay = backoff.failure();
        if !idle(&mut task, delay).await {
            finish(&mut task).await;
            return;
        }
    }

    loop {
        let srv = match server.take() {
            Some(s) => s,
            None => match open_server(task.bind).await {
                Ok(s) => {
                    if backoff.failures() > 0 {
                        tracing::info!(attempts = backoff.failures(), "KNX device server is up");
                    }
                    // No reset here: a server that comes up and goes down again after a few
                    // seconds keeps backing off. Only a session that lasted resets it.
                    let _ = task.addr_tx.send(Some(s.local_addr()));
                    s
                }
                Err(e) => {
                    let delay = backoff.failure();
                    let error = format!("{e:#}");
                    if warn_on_failure(backoff.failures()) {
                        tracing::warn!(attempt = backoff.failures(), %error, retry_in = ?delay, "KNX device server not available");
                    } else {
                        tracing::debug!(attempt = backoff.failures(), %error, retry_in = ?delay, "KNX device server not available");
                    }
                    if !idle(&mut task, delay).await {
                        break;
                    }
                    continue;
                }
            },
        };
        if restarted {
            // The peers' view of our group objects is stale; send everything again.
            task.resync.notify_one();
        }
        let started = tokio::time::Instant::now();
        let end = serve.run(&mut task, srv).await;
        let _ = task.addr_tx.send(None);
        if end == LoopEnd::Stopped {
            break;
        }
        restarted = true;
        let delay = delay_after_session(&mut backoff, started.elapsed());
        tracing::warn!(retry_in = ?delay, "KNX device server stopped — restarting");
        if !idle(&mut task, delay).await {
            break;
        }
    }
    finish(&mut task).await;
}

/// The end of the BAU task: the debounced write that is still running finishes first.
async fn finish(task: &mut DeviceTask) {
    if let Some(write) = task.pending_persist.take() {
        let _ = write.await;
    }
    tracing::info!("KNX BAU task ended");
}

/// Wait `delay` while there is no server, still serving the commands that need none:
/// programming mode can be read and set, and values land in their group objects, from where
/// the first poll after the restart sends them. Returns `false` on shutdown.
async fn idle(task: &mut DeviceTask, delay: std::time::Duration) -> bool {
    let sleep = tokio::time::sleep(delay);
    tokio::pin!(sleep);
    loop {
        tokio::select! {
            () = &mut sleep => return !*task.shutdown.borrow(),
            () = stop_requested(&mut task.shutdown) => return false,
            cmd = task.cmd_rx.recv() => match cmd {
                None => return false,
                Some(BauCmd::WriteObject { asap, value }) => {
                    if task.bau.association_table().translate_asap(asap).is_some()
                        && let Some(go) = task.bau.group_objects_mut().get_mut(asap)
                    {
                        let _ = go.set_value(&value);
                    }
                }
                Some(cmd) => handle_management_command(&mut task.bau, cmd),
            },
        }
    }
}

/// Programming mode commands, which work with or without a server.
fn handle_management_command(bau: &mut Bau, cmd: BauCmd) {
    match cmd {
        BauCmd::SetProgMode { enabled } => {
            knx_rs_device::device_object::set_prog_mode(bau.device_mut(), enabled);
            tracing::info!(enabled, "KNX programming mode changed");
        }
        BauCmd::GetProgMode { reply } => {
            let enabled = knx_rs_device::device_object::prog_mode(bau.device());
            let _ = reply.send(enabled);
        }
        BauCmd::WriteObject { .. } => {}
        #[cfg(test)]
        BauCmd::RestartServer => {}
    }
}

#[allow(clippy::too_many_lines)] // One select loop over server events, commands and the persist timer
async fn bau_task_loop(task: &mut DeviceTask, mut server: DeviceServer) -> LoopEnd {
    let DeviceTask {
        bau,
        cmd_rx,
        update_tx,
        persist_path,
        restart_after_ets,
        config_ia,
        shutdown,
        resync,
        pending_persist,
        ..
    } = task;
    let mut memory_dirty = false;
    let persist_timer = tokio::time::sleep(ETS_PERSIST_DEBOUNCE);
    tokio::pin!(persist_timer);
    let mut persist_armed = false;
    let mut tunnel_active = false;

    // Frames for the server wait here and are handed over without ever waiting for it (see
    // `outbound`): a device task that waits for the server can deadlock with it.
    let mut outbound = Outbound::new();
    let mut drops = Throttle::default();

    // Values written while there was no server wait in their group objects.
    bau.poll(now_ms());
    drain_outbox(bau, &mut outbound);
    if outbound.flush(&server, tokio::time::Instant::now()) {
        return finish_loop(server, LoopEnd::ServerClosed).await;
    }

    let end = loop {
        let outbound_wake = outbound.next_wake();
        tokio::select! {
            event = server.recv() => {
                let Some(event) = event else { break LoopEnd::ServerClosed };
                let is_tunnel = matches!(event, ServerEvent::TunnelFrame(_));
                let frame = repair_master_reset(match event {
                    ServerEvent::TunnelFrame(f) | ServerEvent::RoutingFrame(f) => f,
                });

                // Log tunnel client connections
                if is_tunnel && !tunnel_active {
                    tunnel_active = true;
                    tracing::info!(
                        source = %frame.source_address(),
                        "KNX tunnel client connected (ETS programming session)"
                    );
                }

                // ETS programs through management services addressed to this device, over a
                // tunnel or through a router.
                let programming = is_management_for_device(bau, &frame);
                let before = programming.then(|| TableSummary::of(bau));
                let ia_before = bau.individual_address();
                match classify_group_frame(bau, &frame) {
                    GroupPolicy::Pass => bau.process_frame(&frame, now_ms()),
                    GroupPolicy::Drop => {}
                    GroupPolicy::Correct(asaps) => {
                        // A foreign write on a status address: say what the value really is.
                        for asap in asaps {
                            if let Some(go) = bau.group_objects_mut().get_mut(asap)
                                && go.initialized()
                            {
                                go.object_written();
                            }
                        }
                    }
                }
                bau.poll(now_ms());
                drain_outbox(bau, &mut outbound);
                if outbound.flush(&server, tokio::time::Instant::now()) {
                    break LoopEnd::ServerClosed;
                }

                // ETS programs via tunnel — mark dirty for persistence. So does an individual
                // address assignment, which can come before the download.
                if persist_path.is_some()
                    && ((programming && !bau.memory_area().is_empty())
                        || bau.individual_address() != ia_before)
                {
                    memory_dirty = true;
                    persist_timer.as_mut().reset(tokio::time::Instant::now() + ETS_PERSIST_DEBOUNCE);
                    persist_armed = true;
                }

                // Log (and re-publish) when a download changed what is associated.
                if let Some(before) = before {
                    let after = TableSummary::of(bau);
                    if after != before {
                        after.log("ETS download");
                        // A device that is not configured sends nothing, and what its group
                        // objects were handed meanwhile goes out when it becomes configured.
                        // Asking for a burst at every step of the download would send each
                        // status telegram once per step.
                        if after.configured {
                            resync.notify_one();
                        }
                    }
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
                    // An empty memory is no reason to skip the write when a file from an
                    // earlier programming exists: a factory reset wipes the memory, and the
                    // file must say so, or the next start brings the old programming back.
                    if let Some(path) = persist_path.as_ref()
                        && (!bau.memory_area().is_empty()
                            || bau.individual_address().raw() != *config_ia
                            || path.exists()) {
                            let state = persisted_state(bau);
                            if let Err(e) = persist_memory(path, &state, ia_record(bau, *config_ia)) {
                                tracing::warn!(error = %e, "Failed to persist ETS state before restart");
                            } else {
                                tracing::info!(bytes = state.len(), "ETS programming persisted before restart");
                            }
                        }
                    if *restart_after_ets {
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

                dispatch_updated_gos(bau, update_tx, &mut drops);
            }

            cmd = cmd_rx.recv() => {
                let Some(cmd) = cmd else { break LoopEnd::Stopped };
                match cmd {
                    BauCmd::WriteObject { asap, value } => {
                        write_object(bau, asap, &value);
                        drain_outbox(bau, &mut outbound);
                        if outbound.flush(&server, tokio::time::Instant::now()) {
                            break LoopEnd::ServerClosed;
                        }
                    }
                    #[cfg(test)]
                    BauCmd::RestartServer => break LoopEnd::ServerClosed,
                    other => handle_management_command(bau, other),
                }
            }
            // Debounced persist timer
            () = &mut persist_timer, if persist_armed => {
                persist_armed = false;
                memory_dirty = false;
                tunnel_active = false;
                if let Some(ref path) = *persist_path {
                    *pending_persist = Some(spawn_persist(
                        persisted_state(bau),
                        path.clone(),
                        ia_record(bau, *config_ia),
                    ));
                }
            }
            () = async {
                match outbound_wake {
                    Some(at) => tokio::time::sleep_until(at).await,
                    None => std::future::pending().await,
                }
            } => {
                let now = tokio::time::Instant::now();
                if outbound.stalled(now) {
                    tracing::warn!(
                        waiting = outbound.len(),
                        "KNX device server takes no frames (a tunnel client that does not acknowledge?) — restarting it"
                    );
                    break LoopEnd::ServerClosed;
                }
                if outbound.flush(&server, now) {
                    break LoopEnd::ServerClosed;
                }
                let dropped = outbound.take_dropped();
                if dropped > 0 && drops.allow(std::time::Duration::from_secs(5)) {
                    tracing::warn!(dropped, "KNX device server cannot keep up, oldest frames dropped");
                }
            }
            () = stop_requested(shutdown) => break LoopEnd::Stopped,
        }
    };

    // Persist on the way out if dirty
    if memory_dirty && let Some(ref path) = *persist_path {
        let state = persisted_state(bau);
        if let Err(e) = persist_memory(path, &state, ia_record(bau, *config_ia)) {
            tracing::warn!(error = %e, "Failed to persist ETS state on shutdown");
        } else {
            tracing::info!(path = %path.display(), "ETS state persisted on shutdown");
        }
    }
    finish_loop(server, end).await
}

/// Stop the server. It may be busy with a tunnel client that does not acknowledge, so only
/// try for a moment: dropping it ends its task in any case.
async fn finish_loop(server: DeviceServer, end: LoopEnd) -> LoopEnd {
    let _ = tokio::time::timeout(std::time::Duration::from_millis(500), server.stop()).await;
    end
}

/// Move what the device stack wants to send into the outbound queue, as indications.
fn drain_outbox(bau: &mut Bau, outbound: &mut Outbound) {
    while let Some(out) = bau.next_outgoing_frame() {
        outbound.push(to_indication(out));
    }
}

/// The individual address record to save next to the BAU state.
fn ia_record(bau: &Bau, config_ia: u16) -> IaRecord {
    IaRecord {
        configured: config_ia,
        assigned: bau.individual_address().raw(),
    }
}

/// Persist a saved BAU memory snapshot to `path` in a blocking task, logging the
/// outcome. Shared by the debounced-persist and restart-persist paths.
fn spawn_persist(state: Vec<u8>, path: PathBuf, ia: IaRecord) -> tokio::task::JoinHandle<()> {
    // The snapshot is numbered now, not when the blocking task gets to run.
    let generation = next_generation();
    tokio::task::spawn_blocking(
        move || match persist_generation(&path, &state, ia, generation) {
            Ok(()) => {
                tracing::info!(path = %path.display(), bytes = state.len(), "ETS programming persisted");
            }
            Err(e) => tracing::warn!(error = %e, "Failed to persist ETS state"),
        },
    )
}

/// Load and parse persisted ETS device memory into [`EtsParams`], or `None` if the
/// file is absent/unreadable or the device was never fully programmed.
///
/// Reads the same [`ETS_MEMORY_PATH`] the BAU task persists to, and builds a
/// throwaway BAU purely to restore the memory image and parse it — no
/// `DeviceServer` and no UDP bind — so it is safe to call at boot **before** the
/// real KNX device transport starts (KEA-0004 boot-time apply).
pub(super) fn load_persisted_ets_params(ia: IndividualAddress) -> Option<EtsParams> {
    let saved = load_memory(std::path::Path::new(ETS_MEMORY_PATH)).ok()?;
    let mut bau = build_bau(ia);
    if bau.restore(&saved.payload).is_err() || !ets_programmed(&bau) {
        return None;
    }
    app_parameters(&mut bau)
}

/// Build a BAU with all group objects configured with their DPTs and buffer sizes.
///
/// The address and association tables stay empty: they come from the persisted ETS
/// programming or, for a device ETS never programmed, from [`build_tables_from_config`].
fn build_bau(ia: IndividualAddress) -> Bau {
    let device = device_object::new_device_object(
        device_serial(),
        group_objects::HARDWARE_TYPE, // must match the .knxprod LdCtrlCompareProp (PID 78)
    );
    let mut bau = Bau::new(device, TOTAL_GO_COUNT as u16, DEFAULT_GO_SIZE);
    device_object::set_individual_address(bau.device_mut(), ia.raw());

    // The group object table (flags and sizes) is compiled in. Without it the stack skips every
    // flag check: responses would execute commands, command objects would answer reads, and a
    // foreign write would stick on a status object.
    bau.load_group_object_table(&group_objects::descriptor_table());

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

    // Configure the device-level GOs. Without a DPT a write fails with `NoDpt`, and the
    // default one-byte buffer would cut the three-byte KNX time.
    for (i, go_def) in group_objects::GLOBAL_GOS.iter().enumerate() {
        let asap = group_objects::global_asap(i);
        if let Some(go) = bau.group_objects_mut().get_mut(asap) {
            go.set_dpt(go_def.dpt);
        }
    }

    bau
}

/// Build address and association tables from TOML zone/client/global KNX addresses and
/// install them as if ETS had downloaded them. Returns `false` when no address is configured.
///
/// For an object with two addresses (a command and its status), the sending address comes
/// first: the device stack sends to the first association and treats the others as listening
/// addresses.
#[allow(clippy::too_many_lines)] // Table construction from config — linear data flow
fn build_tables_from_config(bau: &mut Bau, config: &crate::config::AppConfig) -> bool {
    // Collect all (GA string, ASAP) pairs
    let mut ga_asap_pairs: Vec<(u16, u16)> = Vec::new();

    for zone_cfg in &config.zones {
        let idx = zone_cfg.index;
        if idx > MAX_ZONES {
            tracing::warn!(
                zone = idx,
                max = MAX_ZONES,
                "KNX device mode has no objects for this zone, its addresses are ignored"
            );
            continue;
        }
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
            (&knx.presence_enable_status, ZGO_PRESENCE_ENABLE),
            (&knx.presence_enable, ZGO_PRESENCE_ENABLE),
            (&knx.presence_timer_status, ZGO_PRESENCE_TIMER_ACTIVE),
        ];
        for (ga_opt, go_idx) in zone_gas {
            if let Some(ga_str) = ga_opt
                && let Ok(ga) = GroupAddress::from_str(ga_str)
            {
                let asap = group_objects::zone_asap(idx, *go_idx);
                ga_asap_pairs.push((ga.raw(), asap));
            }
        }
    }

    for client_cfg in &config.clients {
        let idx = client_cfg.index;
        if idx > MAX_CLIENTS {
            tracing::warn!(
                client = idx,
                max = MAX_CLIENTS,
                "KNX device mode has no objects for this client, its addresses are ignored"
            );
            continue;
        }
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
            if let Some(ga_str) = ga_opt
                && let Ok(ga) = GroupAddress::from_str(ga_str)
            {
                let asap = group_objects::client_asap(idx, *go_idx);
                ga_asap_pairs.push((ga.raw(), asap));
            }
        }
    }

    if let Some(knx) = config.knx.as_ref() {
        let global_gas: &[(&Option<String>, usize)] = &[
            (&knx.server_online, GGO_SERVER_ONLINE),
            (&knx.all_stop, GGO_ALL_STOP),
            (&knx.all_mute_status, GGO_ALL_MUTE),
            (&knx.all_mute, GGO_ALL_MUTE),
            (&knx.system_fault, GGO_SYSTEM_FAULT),
            (&knx.knx_time, GGO_KNX_TIME),
        ];
        for (ga_opt, go_idx) in global_gas {
            if let Some(ga_str) = ga_opt
                && let Ok(ga) = GroupAddress::from_str(ga_str)
            {
                ga_asap_pairs.push((ga.raw(), group_objects::global_asap(*go_idx)));
            }
        }
    }

    if ga_asap_pairs.is_empty() {
        return false;
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
    install_tables(bau, &addr_data, &assoc_data);

    tracing::info!(
        gas = unique_gas.len(),
        associations = assoc_entries.len(),
        "KNX address/association tables loaded from TOML"
    );
    true
}

/// Source address of the management frames [`install_tables`] feeds the BAU (15.15.255).
const INSTALL_SOURCE: u16 = 0xFFFF;

/// Cut `data` into pieces an `A_Memory_Write` can carry.
const INSTALL_CHUNK: usize = 48;

/// Load the address and association table into the BAU through the same management
/// services ETS uses (load state machine plus memory writes), with an empty application
/// program, and leave the BAU's memory area empty.
///
/// The device stack takes part in group communication only when the address table, the
/// association table and the application program all report *loaded*, and those states are
/// not reachable from outside the crate. Filling the runtime tables directly would receive
/// telegrams but never send one.
fn install_tables(bau: &mut Bau, addr: &[u8], assoc: &[u8]) {
    use knx_rs_core::message::ApduType;

    const LOAD_STATE_CONTROL: u8 = 5;
    const OBJ_ADDR_TABLE: u8 = 1;
    const OBJ_ASSOC_TABLE: u8 = 2;
    const OBJ_APP_PROGRAM: u8 = 4;

    let header = |t: ApduType| {
        let b = (t as u16).to_be_bytes();
        [b[0] & 0x03, b[1]]
    };
    let feed = |bau: &mut Bau, apdu: &[u8]| {
        if let Some(frame) = management_frame(bau, apdu) {
            bau.process_frame(&frame, now_ms());
        }
        // The property read-backs are for a programming tool; there is none.
        while bau.next_outgoing_frame().is_some() {}
    };

    // Start from nothing: the stack allocates behind whatever the memory holds, and the tables
    // are written at offset 0. A leftover image (an interrupted download that was persisted)
    // would put the allocation elsewhere and leave the tables empty but "loaded". An object that
    // is still loading, or in error, ignores a start-loading event, so unload first.
    bau.set_memory_area(Vec::new());
    for obj in [OBJ_ADDR_TABLE, OBJ_ASSOC_TABLE, OBJ_APP_PROGRAM] {
        let [hi, lo] = header(ApduType::PropertyValueWrite);
        feed(bau, &[hi, lo, obj, LOAD_STATE_CONTROL, 0x10, 0x01, 4]); // unload
    }

    let mut offset = 0usize;
    for (obj, data) in [
        (OBJ_ADDR_TABLE, addr),
        (OBJ_ASSOC_TABLE, assoc),
        (OBJ_APP_PROGRAM, &[][..]),
    ] {
        let load_control = |payload: &[u8]| {
            let [hi, lo] = header(ApduType::PropertyValueWrite);
            let mut a = vec![hi, lo, obj, LOAD_STATE_CONTROL, 0x10, 0x01];
            a.extend_from_slice(payload);
            a
        };
        feed(bau, &load_control(&[1])); // start loading
        let mut alloc = vec![0x03, 0x0B];
        alloc.extend_from_slice(&u32::try_from(data.len()).unwrap_or(0).to_be_bytes());
        alloc.extend_from_slice(&[0x01, 0x00]); // relative allocation, filled with zeros
        feed(bau, &load_control(&alloc));
        for (i, chunk) in data.chunks(INSTALL_CHUNK).enumerate() {
            let [hi, lo] = header(ApduType::MemoryWrite);
            let at = u16::try_from(offset + i * INSTALL_CHUNK).unwrap_or(0);
            let mut a = vec![
                hi,
                lo | u8::try_from(chunk.len()).unwrap_or(0),
                (at >> 8) as u8,
                (at & 0xFF) as u8,
            ];
            a.extend_from_slice(chunk);
            feed(bau, &a);
        }
        feed(bau, &load_control(&[2])); // load completed
        offset += data.len();
    }

    // The tables now live in the runtime lookup structures. Forget the memory image: it is
    // not ETS programming and must not be persisted as such, and a later ETS download
    // allocates the application parameters at offset 0, where `parse_ets_memory` expects them.
    bau.set_memory_area(Vec::new());
}

/// Write a value to a group object. The associations decide the address; an object that is
/// not associated with any address keeps the value (a read of it is answered once it has one)
/// and sends nothing. The frame the poll produces is the caller's to queue.
fn write_object(bau: &mut Bau, asap: u16, value: &DptValue) {
    // A value for an object nobody listens to, or one handed over while the device does not
    // send (no download yet, or one in progress), would stay queued in the object and go out
    // as soon as the device is configured, in front of the burst that the end of the download
    // asks for anyway: every status telegram twice, in ASAP order and in publishing order.
    if !bau.configured() || bau.association_table().translate_asap(asap).is_none() {
        return;
    }
    let Some(go) = bau.group_objects_mut().get_mut(asap) else {
        return;
    };
    if let Err(e) = go.set_value(value) {
        tracing::warn!(asap, error = %e, "KNX group object write failed");
        return;
    }
    bau.poll(now_ms());
}

/// Forward every group object the bus updated to the listener channel, without waiting: a
/// listener that is slow (a zone that does not take commands) must not stop the device task.
/// What does not fit is dropped, and said once in a while.
fn dispatch_updated_gos(
    bau: &mut Bau,
    update_tx: &mpsc::Sender<(u16, Vec<u8>)>,
    drops: &mut Throttle,
) {
    // While ETS reloads a table the tables disagree: a new address table against the old
    // associations would hand a telegram to another object. Nothing is dispatched until the
    // device is configured again.
    let configured = bau.configured();
    while let Some(asap) = bau.group_objects().next_updated() {
        let data = bau
            .group_objects()
            .get(asap)
            .map(|go| go.value_ref().to_vec());
        // Acknowledge first: a closed channel must not keep this loop spinning.
        if let Some(go) = bau.group_objects_mut().get_mut(asap) {
            go.set_comm_flag(knx_rs_device::group_object::ComFlag::Ok);
        }
        if let Some(data) = data
            && configured
            && update_tx.try_send((asap, data)).is_err()
            && drops.allow(std::time::Duration::from_secs(5))
        {
            tracing::warn!(
                asap,
                "KNX command queue is full or closed, telegram dropped"
            );
        }
    }
}

/// Whether `frame` is a point-to-point frame to this device: what ETS (a programming tool) sends,
/// over a tunnel or through a router. Group telegrams, even a burst of them from a visualisation,
/// are not programming.
fn is_management_for_device(bau: &Bau, frame: &CemiFrame) -> bool {
    frame.address_type() == knx_rs_core::types::AddressType::Individual
        && frame.destination_address_raw() == bau.individual_address().raw()
}

/// An `A_Restart` with the master reset type (`0x0381`) as the specification has it: the APCI,
/// the erase code and the channel, four bytes. knx-rs 0.11 strips both APCI bytes and then reads
/// the erase code at `data[1]`, the layout of the C++ stack's pointer, which still has the low
/// APCI byte in front. It rejects the four-byte form as too short, so a master reset from a real
/// ETS (a factory reset, "reset links", "reset application") would never take effect. The low
/// APCI byte is repeated in front of the erase code, which gives the stack the layout it reads.
///
/// Other frames, and a frame that already has the longer form, are returned as they are.
fn repair_master_reset(frame: CemiFrame) -> CemiFrame {
    const MASTER_RESET: u16 = 0x0381;
    let bytes = frame.as_bytes();
    // msg code, additional info length, ctrl1, ctrl2, source (2), destination (2), length
    let length_at = 8 + bytes.get(1).map_or(0, |n| usize::from(*n));
    let tpdu = length_at + 1;
    let [hi, lo] = match bytes.get(tpdu..tpdu + 2) {
        Some([hi, lo]) => [*hi, *lo],
        _ => return frame,
    };
    let apci = u16::from_be_bytes([hi & 0x03, lo]);
    if apci != MASTER_RESET || bytes.len() != tpdu + 4 || usize::from(bytes[length_at]) != 3 {
        return frame;
    }
    let mut patched = bytes.to_vec();
    patched[length_at] = 4;
    patched.insert(tpdu + 2, lo);
    CemiFrame::parse(&patched).unwrap_or(frame)
}

/// A frame the device stack produced, as the KNXnet/IP server must deliver it.
///
/// The stack builds its outgoing frames as `L_Data.req` (0x11), a request from an application to
/// the data link layer. A server tells its tunnel clients, and the routing multicast group, about
/// a telegram with `L_Data.ind` (0x29). Clients and routers that follow the specification may
/// ignore a request that arrives where an indication belongs.
fn to_indication(frame: CemiFrame) -> CemiFrame {
    use knx_rs_core::message::MessageCode;
    if frame.message_code_raw() != MessageCode::LDataReq as u8 {
        return frame;
    }
    let mut bytes = frame.as_bytes().to_vec();
    bytes[0] = MessageCode::LDataInd as u8;
    CemiFrame::parse(&bytes).unwrap_or(frame)
}

/// What the BAU task does with a telegram before the device stack sees it.
#[derive(Debug, PartialEq, Eq)]
enum GroupPolicy {
    /// Hand it to the device stack (which applies the object flags).
    Pass,
    /// Ignore it.
    Drop,
    /// A foreign write on a status address: send these objects' actual values again.
    Correct(Vec<u16>),
}

/// Decide what to do with a group telegram.
///
/// - While the device is not configured (ETS is reloading a table) group telegrams are dropped.
/// - Group value responses are dropped. snapdog never reads, so a response answers somebody
///   else's read, and it must not execute a command.
/// - A write to an address that only status objects listen to is not applied (the objects are
///   not writable) but corrected: the status object sends its actual value, so the bus does
///   not keep a value snapdog does not have. An address that also has a writable object is
///   that object's business: its command changes the state, and the new status follows.
fn classify_group_frame(bau: &Bau, frame: &CemiFrame) -> GroupPolicy {
    use knx_rs_core::message::ApduType;
    use knx_rs_core::types::AddressType;

    let dst = frame.destination_address_raw();
    if frame.address_type() != AddressType::Group || dst == 0 {
        return GroupPolicy::Pass;
    }
    let Some(apdu) = frame.tpdu().and_then(|t| t.apdu().cloned()) else {
        return GroupPolicy::Pass;
    };
    if !matches!(
        apdu.apdu_type,
        ApduType::GroupValueWrite | ApduType::GroupValueResponse | ApduType::GroupValueRead
    ) {
        return GroupPolicy::Pass;
    }
    if !bau.configured() || apdu.apdu_type == ApduType::GroupValueResponse {
        return GroupPolicy::Drop;
    }
    if apdu.apdu_type == ApduType::GroupValueRead {
        return GroupPolicy::Pass;
    }
    let Some(tsap) = bau.address_table().get_tsap(dst) else {
        return GroupPolicy::Pass;
    };
    let asaps = bau.association_table().asaps_for_tsap(tsap);
    let flags = |asap: u16| group_objects::definition_of(asap).map(|d| &d.flags);
    if asaps.iter().any(|a| flags(*a).is_some_and(|f| f.write)) {
        return GroupPolicy::Pass;
    }
    // Only objects that send to this very address can correct it.
    let senders: Vec<u16> = asaps
        .into_iter()
        .filter(|a| flags(*a).is_some_and(|f| f.transmit))
        .filter(|a| {
            bau.association_table()
                .translate_asap(*a)
                .and_then(|t| bau.address_table().get_group_address(t))
                == Some(dst)
        })
        .collect();
    if senders.is_empty() {
        GroupPolicy::Pass
    } else {
        GroupPolicy::Correct(senders)
    }
}

#[cfg(test)]
mod tests {
    use super::group_objects::zone_asap;
    use super::*;

    /// A point-to-point frame from 15.15.177 to 1.1.100 carrying `tpdu` (TPCI and APDU).
    fn p2p(tpdu: &[u8]) -> CemiFrame {
        let mut bytes = vec![0x11, 0x00, 0xB0, 0x60, 0xFF, 0xB1, 0x11, 0x64];
        bytes.push(u8::try_from(tpdu.len() - 1).unwrap());
        bytes.extend_from_slice(tpdu);
        CemiFrame::parse(&bytes).unwrap()
    }

    #[test]
    fn a_master_reset_in_the_specified_form_reaches_the_device_stack_unnumbered() {
        let mut bau = build_bau(IndividualAddress::from_raw(0x1164));
        // erase code 2 (factory reset), channel 0: APCI 0x0381 and two bytes.
        let frame = p2p(&[0x03, 0x81, 0x02, 0x00]);
        bau.process_frame(&frame, 0);
        assert!(!bau.take_restart_pending(), "knx-rs 0.11 rejects the form");
        bau.process_frame(&repair_master_reset(frame), 0);
        assert!(bau.take_restart_pending());
    }

    #[test]
    fn a_master_reset_in_the_specified_form_reaches_the_device_stack_connected() {
        let mut bau = build_bau(IndividualAddress::from_raw(0x1164));
        bau.process_frame(&p2p(&[0x80, 0x00]), 0); // T_Connect
        // Numbered data, sequence 0.
        let frame = repair_master_reset(p2p(&[0x43, 0x81, 0x02, 0x00]));
        bau.process_frame(&frame, 1);
        bau.poll(2);
        assert!(bau.take_restart_pending());
    }

    #[test]
    fn only_a_four_byte_master_reset_is_repaired() {
        let same = |tpdu: &[u8]| {
            let frame = p2p(tpdu);
            repair_master_reset(frame.clone()).as_bytes() == frame.as_bytes()
        };
        assert!(same(&[0x03, 0x80]), "a basic restart");
        assert!(
            same(&[0x03, 0x81, 0x02, 0x00, 0x00]),
            "already the longer form"
        );
        assert!(same(&[0x03, 0x82, 0x02, 0x00]), "another service");
        assert!(same(&[0x00, 0x80]), "a group write");
        let repaired = repair_master_reset(p2p(&[0x03, 0x81, 0x07, 0x00]));
        assert_eq!(
            &repaired.as_bytes()[8..],
            &[4, 0x03, 0x81, 0x81, 0x07, 0x00]
        );
    }

    #[test]
    fn text_parameters_are_decoded_as_iso_8859_15() {
        // "Küche", "Bücher", "5 €", "Œuvre" as ETS stores them.
        assert_eq!(decode_latin9(b"K\xFCche"), "Küche");
        assert_eq!(decode_latin9(b"B\xFCcher"), "Bücher");
        assert_eq!(decode_latin9(b"5 \xA4"), "5 €");
        assert_eq!(decode_latin9(b"\xBCuvre"), "Œuvre");
        assert_eq!(decode_latin9(b"plain ASCII"), "plain ASCII");

        let mut mem = vec![0u8; 32];
        mem[4..9].copy_from_slice(b"K\xFCche");
        assert_eq!(read_string(&mem, 4, 16), "Küche");
    }

    #[test]
    fn parse_ets_memory_defaults() {
        let data = vec![0u8; mem::TOTAL];
        let p = parse_ets_memory(&data);
        assert_eq!(
            p.heartbeat_minutes, 5,
            "unprogrammed memory keeps the default"
        );
        assert!(!p.zone_active[0]);
        assert_eq!(p.zone_max_volume[0], 0);
        assert!(!p.client_active[0]);
    }

    #[test]
    fn parse_ets_memory_values() {
        let mut data = vec![0u8; mem::TOTAL];
        data[mem::NUM_ZONES] = 3; // zones 1..=3 active (contiguous)
        data[mem::ZONE_MAX_VOL] = 80;
        data[mem::NUM_CLIENTS] = 2; // clients 1..=2 active (contiguous)
        data[mem::CLIENT_MAX_VOL] = 60;
        data[mem::CLIENT_DEF_ZONE] = 3;
        data[mem::CLIENT_DEF_LAT] = 50;
        data[mem::GLOBAL_SRATE] = 3; // 96000 Hz
        data[mem::GLOBAL_BITD] = 1; // 24 bit
        data[mem::GLOBAL_HEARTBEAT] = 15;
        let p = parse_ets_memory(&data);
        assert_eq!(p.heartbeat_minutes, 15);
        assert!(p.zone_active[0]);
        assert!(p.zone_active[1]);
        assert!(p.zone_active[2]);
        assert!(!p.zone_active[3]);
        assert_eq!(p.zone_max_volume[0], 80);
        assert!(p.client_active[0]);
        assert!(p.client_active[1]);
        assert!(!p.client_active[2]);
        assert_eq!(p.client_max_volume[0], 60);
        assert_eq!(p.client_default_zone[0], 3);
        assert_eq!(p.client_default_latency[0], 50);
        assert_eq!(p.sample_rate, 96_000);
        assert_eq!(p.bit_depth, 24);
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
        let bau = bau_from_toml(&config);

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

    const IA: IaRecord = IaRecord {
        configured: 0x1164,
        assigned: 0x1201,
    };

    #[test]
    fn persist_roundtrip() {
        let dir = std::env::temp_dir().join("snapdog_test_persist");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("test.bin");

        let payload = vec![0xDE, 0xAD, 0xBE, 0xEF, 0x42];
        super::persist_memory(&path, &payload, IA).unwrap();
        let loaded = super::load_memory(&path).unwrap();
        assert_eq!(loaded.payload, payload);
        assert_eq!(loaded.ia, Some(IA));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn persist_detects_corruption() {
        let dir = std::env::temp_dir().join("snapdog_test_corrupt");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("corrupt.bin");

        let payload = vec![1, 2, 3, 4, 5];
        super::persist_memory(&path, &payload, IA).unwrap();

        // Flip a byte in the payload area
        let mut raw = std::fs::read(&path).unwrap();
        raw[super::PERSIST_HEADER] ^= 0xFF;
        std::fs::write(&path, &raw).unwrap();

        let err = super::load_memory(&path).err().unwrap();
        assert!(
            err.to_string().contains("CRC32 mismatch"),
            "expected CRC error, got: {err}"
        );

        // So does a flipped byte in the individual address record.
        super::persist_memory(&path, &payload, IA).unwrap();
        let mut raw = std::fs::read(&path).unwrap();
        raw[super::PERSIST_HEADER + payload.len()] ^= 0x01;
        std::fs::write(&path, &raw).unwrap();
        let err = super::load_memory(&path).err().unwrap();
        assert!(err.to_string().contains("CRC32 mismatch"), "got: {err}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn persist_rejects_wrong_version() {
        let dir = std::env::temp_dir().join("snapdog_test_version");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("version.bin");

        super::persist_memory(&path, &[0; 10], IA).unwrap();

        let mut raw = std::fs::read(&path).unwrap();
        raw[4] = 99;
        std::fs::write(&path, &raw).unwrap();

        let err = super::load_memory(&path).err().unwrap();
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
        let err = super::load_memory(&path).err().unwrap();
        assert!(
            err.to_string().contains("too short"),
            "expected too-short error, got: {err}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn persist_envelope_byte_layout() {
        // Pin the on-disk format (the IT-T43 §9.2 Bau.save drift risk):
        // ["SDKM"][1 version][2 len LE][payload][2 configured IA LE][2 assigned IA LE]
        // [4 crc32 LE over payload and both addresses].
        let dir = std::env::temp_dir().join("snapdog_test_envelope");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("envelope.bin");
        let payload = vec![0xDE_u8, 0xAD, 0xBE, 0xEF, 0x42];
        super::persist_memory(&path, &payload, IA).unwrap();
        let raw = std::fs::read(&path).unwrap();
        let n = payload.len();

        assert_eq!(&raw[0..4], b"SDKM", "magic");
        assert_eq!(raw[4], 2, "version byte");
        assert_eq!(
            u16::from_le_bytes([raw[5], raw[6]]),
            n as u16,
            "length is little-endian u16 and counts the payload only"
        );
        let h = super::PERSIST_HEADER;
        assert_eq!(&raw[h..h + n], &payload[..], "payload verbatim");
        assert_eq!(
            &raw[h + n..h + n + 2],
            &[0x64, 0x11],
            "configured address, LE"
        );
        assert_eq!(
            &raw[h + n + 2..h + n + 4],
            &[0x01, 0x12],
            "assigned address, LE"
        );
        let crc = u32::from_le_bytes(raw[h + n + 4..h + n + 8].try_into().unwrap());
        assert_eq!(
            crc,
            super::crc32(&raw[h..h + n + 4]),
            "trailing CRC32 is little-endian and covers payload and addresses"
        );
        assert_eq!(
            raw.len(),
            h + n + 4 + 4,
            "header + payload + addresses + CRC"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn version_1_files_still_load() {
        // Written by releases before the individual address was persisted.
        let dir = std::env::temp_dir().join("snapdog_test_v1");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("v1.bin");
        let payload = [9u8, 8, 7];
        let mut raw = b"SDKM".to_vec();
        raw.push(1);
        raw.extend_from_slice(&3u16.to_le_bytes());
        raw.extend_from_slice(&payload);
        raw.extend_from_slice(&crc32fast::hash(&payload).to_le_bytes());
        std::fs::write(&path, &raw).unwrap();

        let loaded = super::load_memory(&path).unwrap();
        assert_eq!(loaded.payload, payload);
        assert_eq!(loaded.ia, None);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn assigned_individual_address_wins_only_under_the_same_configuration() {
        let rec = |configured, assigned| {
            Some(IaRecord {
                configured,
                assigned,
            })
        };
        // ETS assigned 1.2.1 while 1.1.100 was configured, and it still is.
        assert_eq!(
            restored_individual_address(rec(0x1164, 0x1201), 0x1164),
            Some(0x1201)
        );
        // The operator changed the configured address since: the configuration wins.
        assert_eq!(
            restored_individual_address(rec(0x1164, 0x1201), 0x1165),
            None
        );
        // Nothing assigned, or nothing saved (version 1 file).
        assert_eq!(
            restored_individual_address(rec(0x1164, 0x1164), 0x1164),
            None
        );
        assert_eq!(restored_individual_address(None, 0x1164), None);
    }

    #[test]
    fn asap_resolves_to_the_toml_group_address() {
        let bau = bau_from_toml(&toml_config());
        let ga_of = |asap: u16| {
            let tsap = bau.association_table().translate_asap(asap)?;
            bau.address_table().get_group_address(tsap)
        };
        // ASAP 1 = zone 1 Play.
        assert_eq!(
            ga_of(zone_asap(1, ZGO_PLAY)),
            Some(GroupAddress::from_str("1/0/1").unwrap().raw())
        );
        // A non-existent ASAP resolves to nothing.
        assert_eq!(ga_of(60000), None);
    }

    fn toml_config() -> crate::config::AppConfig {
        let raw: crate::config::FileConfig = toml::from_str(
            r#"
            [knx]
            role = "device"
            individual_address = "1.1.100"
            all_stop = "0/0/1"
            all_mute = "0/0/2"
            all_mute_status = "0/0/3"
            knx_time = "0/0/4"

            [[zone]]
            name = "Test"
            [zone.knx]
            play = "1/0/1"
            volume = "1/0/2"
            volume_status = "1/0/3"
            presence_enable = "1/0/4"
            presence_enable_status = "1/0/5"

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
        crate::config::load_raw(raw).unwrap()
    }

    /// A BAU the way `start_device_transport` builds it for a device ETS never programmed.
    fn bau_from_toml(config: &crate::config::AppConfig) -> Bau {
        let mut bau = build_bau(IndividualAddress::from_raw(0x1164));
        assert!(!bau.configured(), "no tables yet");
        assert!(build_tables_from_config(&mut bau, config));
        bau
    }

    #[test]
    fn every_group_object_has_its_dpt_and_a_big_enough_buffer() {
        let bau = build_bau(IndividualAddress::from_raw(0x1164));
        let check = |asap: u16, def: &group_objects::GoDefinition| {
            let go = bau.group_objects().get(asap).unwrap();
            assert_eq!(go.dpt(), Some(def.dpt), "{} dpt", def.name);
            assert!(
                go.value_size() >= def.dpt.data_length() as usize,
                "{} buffer {} < {}",
                def.name,
                go.value_size(),
                def.dpt.data_length()
            );
        };
        for (i, def) in group_objects::GLOBAL_GOS.iter().enumerate() {
            check(group_objects::global_asap(i), def);
        }
        for (i, def) in ZONE_GOS.iter().enumerate() {
            check(zone_asap(MAX_ZONES, i), def);
        }
        for (i, def) in CLIENT_GOS.iter().enumerate() {
            check(group_objects::client_asap(MAX_CLIENTS, i), def);
        }
        // The KNX time needs all three bytes.
        let time = bau
            .group_objects()
            .get(group_objects::global_asap(GGO_KNX_TIME))
            .unwrap();
        assert_eq!(time.value_size(), 3);
    }

    #[test]
    fn toml_tables_configure_the_device_so_it_sends() {
        let mut bau = bau_from_toml(&toml_config());
        assert!(
            bau.configured(),
            "tables installed through the load state machine"
        );
        assert_eq!(
            bau.next_outgoing_frame().map(|_| ()),
            None,
            "read-backs discarded"
        );

        // A status object sends on its TOML address.
        let go = bau
            .group_objects_mut()
            .get_mut(zone_asap(1, ZGO_VOLUME_STATUS))
            .unwrap();
        go.set_value(&DptValue::Float(40.0)).unwrap();
        bau.poll(0);
        let frame = bau.next_outgoing_frame().expect("status telegram");
        assert_eq!(
            frame.destination_address_raw(),
            GroupAddress::from_str("1/0/3").unwrap().raw()
        );

        // A global status object does as well.
        let go = bau
            .group_objects_mut()
            .get_mut(group_objects::global_asap(GGO_ALL_MUTE))
            .unwrap();
        go.set_value(&DptValue::Bool(true)).unwrap();
        bau.poll(0);
        let frame = bau.next_outgoing_frame().expect("all-mute status telegram");
        assert_eq!(
            frame.destination_address_raw(),
            GroupAddress::from_str("0/0/3").unwrap().raw(),
            "the status address sends, the command address only listens"
        );
    }

    #[test]
    fn object_with_command_and_status_address_sends_on_the_status_address() {
        let mut bau = bau_from_toml(&toml_config());
        let go = bau
            .group_objects_mut()
            .get_mut(zone_asap(1, ZGO_PRESENCE_ENABLE))
            .unwrap();
        go.set_value(&DptValue::Bool(true)).unwrap();
        bau.poll(0);
        let frame = bau.next_outgoing_frame().expect("presence-enable status");
        assert_eq!(
            frame.destination_address_raw(),
            GroupAddress::from_str("1/0/5").unwrap().raw()
        );
    }

    #[test]
    fn table_summary_counts_associated_objects_by_kind() {
        let bau = bau_from_toml(&toml_config());
        let s = TableSummary::of(&bau);
        assert!(s.configured);
        // zone: play, volume, volume_status, presence_enable (x2 addresses, one object)
        assert_eq!(s.zone_objects, 4);
        assert_eq!(s.client_objects, 2);
        // all_stop, all_mute (x2), knx_time
        assert_eq!(s.global_objects, 3);
        assert_eq!(s.group_addresses, 11);
        assert_eq!(s.associations, 11);

        let empty = TableSummary::of(&build_bau(IndividualAddress::from_raw(0x1164)));
        assert!(!empty.configured);
        assert_eq!((empty.associations, empty.zone_objects), (0, 0));
    }

    // ── ETS downloads against the device stack ──────────────────────

    fn apdu_header(t: knx_rs_core::message::ApduType) -> [u8; 2] {
        let b = (t as u16).to_be_bytes();
        [b[0] & 0x03, b[1]]
    }

    /// Send a management APDU from the programming tool and return what the device answers.
    fn send_management(bau: &mut Bau, apdu: &[u8]) -> Vec<CemiFrame> {
        let frame = management_frame(bau, apdu).unwrap();
        bau.process_frame(&frame, 0);
        std::iter::from_fn(|| bau.next_outgoing_frame()).collect()
    }

    /// Allocate a segment for `obj` the way ETS does (start loading, allocate, read the
    /// table reference) and return where it is.
    fn allocate_segment(bau: &mut Bau, obj: u8, size: usize) -> usize {
        use knx_rs_core::message::ApduType;
        let load_control = |payload: &[u8]| {
            let [hi, lo] = apdu_header(ApduType::PropertyValueWrite);
            let mut a = vec![hi, lo, obj, 5, 0x10, 0x01];
            a.extend_from_slice(payload);
            a
        };
        send_management(bau, &load_control(&[1]));
        let mut alc = vec![0x03, 0x0B];
        alc.extend_from_slice(&u32::try_from(size).unwrap().to_be_bytes());
        alc.extend_from_slice(&[0x01, 0x00]);
        send_management(bau, &load_control(&alc));
        let [hi, lo] = apdu_header(ApduType::PropertyValueRead);
        let response = send_management(bau, &[hi, lo, obj, 7, 0x10, 0x01]);
        let p = response[0].payload();
        u32::from_be_bytes(p[p.len() - 4..].try_into().unwrap()) as usize
    }

    fn write_segment(bau: &mut Bau, at: usize, data: &[u8]) {
        use knx_rs_core::message::ApduType;
        for (i, chunk) in data.chunks(48).enumerate() {
            let [hi, lo] = apdu_header(ApduType::MemoryWrite);
            let to = u16::try_from(at + i * 48).unwrap();
            let mut a = vec![
                hi,
                lo | u8::try_from(chunk.len()).unwrap(),
                (to >> 8) as u8,
                (to & 0xFF) as u8,
            ];
            a.extend_from_slice(chunk);
            send_management(bau, &a);
        }
    }

    fn complete(bau: &mut Bau, obj: u8) {
        use knx_rs_core::message::ApduType;
        let [hi, lo] = apdu_header(ApduType::PropertyValueWrite);
        send_management(bau, &[hi, lo, obj, 5, 0x10, 0x01, 2]);
    }

    fn params_with_zones(zones: u8) -> Vec<u8> {
        let mut params = vec![0u8; mem::TOTAL];
        params[mem::NUM_ZONES] = zones;
        params
    }

    fn zones_of(p: &EtsParams) -> usize {
        p.zone_active.iter().filter(|a| **a).count()
    }

    /// The download order of the generated product: application segment first, then the tables,
    /// then the parameters. `tables` is `false` for a parameter-only download.
    fn ets_download(bau: &mut Bau, zones: u8, tables: Option<(&[u8], &[u8])>) {
        let params = params_with_zones(zones);
        let app_at = allocate_segment(bau, 4, params.len());
        if let Some((addr, assoc)) = tables {
            for (obj, data) in [(1u8, addr), (2u8, assoc)] {
                let at = allocate_segment(bau, obj, data.len());
                write_segment(bau, at, data);
                complete(bau, obj);
            }
        }
        write_segment(bau, app_at, &params);
        complete(bau, 4);
    }

    fn toml_tables() -> (Vec<u8>, Vec<u8>) {
        // 1/0/1 and 1/0/2; zone 1 Play and Volume.
        let addr = vec![0, 2, 0x08, 0x01, 0x08, 0x02];
        let assoc = vec![0, 2, 0, 1, 0, 1, 0, 2, 0, 6];
        (addr, assoc)
    }

    #[test]
    fn a_parameter_only_download_replaces_the_parameters() {
        let (addr, assoc) = toml_tables();
        let mut bau = build_bau(IndividualAddress::from_raw(0x1164));
        ets_download(&mut bau, 3, Some((&addr, &assoc)));
        assert!(ets_programmed(&bau));
        assert_eq!(zones_of(&app_parameters(&mut bau).unwrap()), 3);

        // ETS changes a parameter and downloads only the parameters. The device stack
        // allocates a second segment behind the first; the first still holds the old values.
        ets_download(&mut bau, 1, None);
        assert_eq!(
            bau.memory_area().len(),
            2 * mem::TOTAL + addr.len() + assoc.len()
        );
        assert_eq!(
            zones_of(&parse_ets_memory(bau.memory_area())),
            3,
            "the start of the memory is the stale first download"
        );
        assert_eq!(
            zones_of(&app_parameters(&mut bau).unwrap()),
            1,
            "the parameters come from the segment the last download wrote"
        );
    }

    #[test]
    fn nothing_is_programmed_without_a_download() {
        let mut bau = bau_from_toml(&toml_config());
        assert!(bau.configured());
        assert!(
            !ets_programmed(&bau),
            "TOML tables are not an ETS programming"
        );
        assert!(app_parameters(&mut bau).is_none());
        let mut blank = build_bau(IndividualAddress::from_raw(0x1164));
        assert!(app_segment(&mut blank).is_none());
    }

    #[test]
    fn repeated_downloads_persist_a_compact_state() {
        let (addr, assoc) = toml_tables();
        let mut bau = build_bau(IndividualAddress::from_raw(0x1164));
        ets_download(&mut bau, 3, Some((&addr, &assoc)));
        ets_download(&mut bau, 2, None);
        ets_download(&mut bau, 1, None);
        assert!(
            bau.memory_area().len() > 3 * mem::TOTAL,
            "the live memory grew"
        );

        let state = persisted_state(&mut bau);
        assert_eq!(
            state.len(),
            27 + 5 + 4 + mem::TOTAL + addr.len() + assoc.len(),
            "one parameter block and the two tables"
        );
        assert!(state.len() < usize::from(u16::MAX));

        // A restore of the compact state is the same programming.
        let mut restored = build_bau(IndividualAddress::from_raw(0x1164));
        restored.restore(&state).unwrap();
        assert!(ets_programmed(&restored));
        assert_eq!(zones_of(&app_parameters(&mut restored).unwrap()), 1);
        assert_eq!(
            zones_of(&parse_ets_memory(restored.memory_area())),
            1,
            "the parameters are at offset 0 again"
        );
        let (a, b) = (TableSummary::of(&bau), TableSummary::of(&restored));
        assert_eq!(a, b);
        assert_eq!((b.group_addresses, b.associations), (2, 2));
        assert_eq!(
            restored.association_table().translate_asap(1),
            bau.association_table().translate_asap(1)
        );
        assert_eq!(
            restored.address_table().get_tsap(0x0802),
            bau.address_table().get_tsap(0x0802)
        );

        // Saving the restored state again changes nothing.
        assert_eq!(persisted_state(&mut restored), state);
    }

    #[test]
    fn a_device_without_ets_programming_persists_an_unprogrammed_state() {
        // Only the individual address is saved for a TOML-configured device. The next boot must
        // rebuild its tables from the configuration instead of finding "loaded" tables that
        // are empty.
        let mut bau = bau_from_toml(&toml_config());
        let state = persisted_state(&mut bau);
        let mut restored = build_bau(IndividualAddress::from_raw(0x1164));
        restored.restore(&state).unwrap();
        assert!(!restored.configured());
        assert!(!ets_programmed(&restored));
    }

    #[test]
    fn a_state_too_big_for_the_file_is_an_error_not_a_wrapped_length() {
        let dir = std::env::temp_dir().join("snapdog_test_too_big");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("big.bin");
        let err = persist_memory(&path, &vec![0u8; 70_000], IA).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
        assert!(!path.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_partly_programmed_device_is_saved_as_it_is() {
        let mut bau = build_bau(IndividualAddress::from_raw(0x1164));
        let (addr, _) = toml_tables();
        let at = allocate_segment(&mut bau, 1, addr.len());
        write_segment(&mut bau, at, &addr);
        complete(&mut bau, 1);
        assert!(!bau.configured());
        assert!(compact_state(&mut bau).is_none());
    }

    #[test]
    fn toml_addresses_of_zone_11_are_not_installed_on_client_1() {
        // zone_asap(11, i) == client_asap(1, i): without the bound, zone 11's play address
        // would be associated with a client object.
        let mut toml = String::new();
        for z in 1..=11 {
            let _ =
                std::fmt::Write::write_fmt(&mut toml, format_args!("[[zone]]\nname = \"Z{z}\"\n"));
            if z == 11 {
                toml.push_str("[zone.knx]\nplay = \"4/0/1\"\n");
            }
        }
        toml.push_str("[[client]]\nname = \"C\"\nmac = \"02:42:ac:11:00:10\"\nzone = \"Z1\"\n");
        let config = crate::config::load_raw(toml::from_str(&toml).unwrap()).unwrap();
        let mut bau = build_bau(IndividualAddress::from_raw(0x1164));
        assert!(
            !build_tables_from_config(&mut bau, &config),
            "nothing to install"
        );
        assert_eq!(TableSummary::of(&bau).associations, 0);
    }

    // ── Group telegram policy ───────────────────────────────────────

    #[derive(Clone, Copy)]
    enum Kind {
        Write,
        Response,
        Read,
    }

    fn group_frame(dst: &str, kind: Kind, value: u8) -> CemiFrame {
        use knx_rs_core::address::DestinationAddress;
        use knx_rs_core::message::MessageCode;
        use knx_rs_core::types::Priority;
        let payload = match kind {
            Kind::Write => [0x00, 0x80 | (value & 0x3F)],
            Kind::Response => [0x00, 0x40 | (value & 0x3F)],
            Kind::Read => [0x00, 0x00],
        };
        CemiFrame::new_l_data(
            MessageCode::LDataInd,
            IndividualAddress::from_raw(0x1101),
            DestinationAddress::Group(GroupAddress::from_str(dst).unwrap()),
            Priority::Low,
            &payload,
        )
    }

    /// 1/0/1 zone 1 Play, 1/0/2 Volume (command), 1/0/3 Volume Status, 1/0/4 presence enable
    /// status and 1/0/5 presence enable command, plus a command and a status object that share
    /// 1/0/9.
    fn policy_bau() -> Bau {
        let raw: crate::config::FileConfig = toml::from_str(
            r#"
            [[zone]]
            name = "Test"
            [zone.knx]
            play = "1/0/1"
            volume = "1/0/9"
            volume_status = "1/0/9"
            mute_status = "1/0/3"
            [[client]]
            name = "Speaker"
            mac = "00:00:00:00:00:00"
            zone = "Test"
        "#,
        )
        .unwrap();
        bau_from_toml(&crate::config::load_raw(raw).unwrap())
    }

    #[test]
    fn responses_never_reach_the_objects() {
        let bau = policy_bau();
        for dst in ["1/0/1", "1/0/9", "1/0/3"] {
            assert_eq!(
                classify_group_frame(&bau, &group_frame(dst, Kind::Response, 1)),
                GroupPolicy::Drop,
                "{dst}"
            );
        }
    }

    #[test]
    fn writes_to_command_addresses_pass() {
        let bau = policy_bau();
        assert_eq!(
            classify_group_frame(&bau, &group_frame("1/0/1", Kind::Write, 1)),
            GroupPolicy::Pass
        );
        // A command and a status object share 1/0/9: the command decides.
        assert_eq!(
            classify_group_frame(&bau, &group_frame("1/0/9", Kind::Write, 1)),
            GroupPolicy::Pass
        );
    }

    #[test]
    fn a_foreign_write_on_a_status_address_is_corrected() {
        let bau = policy_bau();
        let mute_status = zone_asap(1, ZGO_MUTE_STATUS);
        assert_eq!(
            classify_group_frame(&bau, &group_frame("1/0/3", Kind::Write, 1)),
            GroupPolicy::Correct(vec![mute_status])
        );
    }

    #[test]
    fn reads_unknown_addresses_and_management_pass_to_the_stack() {
        let bau = policy_bau();
        assert_eq!(
            classify_group_frame(&bau, &group_frame("1/0/3", Kind::Read, 0)),
            GroupPolicy::Pass
        );
        assert_eq!(
            classify_group_frame(&bau, &group_frame("7/7/7", Kind::Write, 1)),
            GroupPolicy::Pass
        );
        let mgmt = management_frame(&bau, &[0x03, 0x00]).unwrap();
        assert_eq!(classify_group_frame(&bau, &mgmt), GroupPolicy::Pass);
    }

    #[test]
    fn group_telegrams_are_dropped_while_a_table_is_being_loaded() {
        let mut bau = policy_bau();
        assert_eq!(
            classify_group_frame(&bau, &group_frame("1/0/1", Kind::Write, 1)),
            GroupPolicy::Pass
        );
        // ETS starts reloading the association table: the runtime tables disagree from now on.
        send_management(&mut bau, &[0x03, 0xD7, 2, 5, 0x10, 0x01, 1]);
        assert!(!bau.configured());
        for kind in [Kind::Write, Kind::Read, Kind::Response] {
            assert_eq!(
                classify_group_frame(&bau, &group_frame("1/0/1", kind, 1)),
                GroupPolicy::Drop
            );
        }
    }

    #[test]
    fn the_table_makes_the_stack_enforce_the_flags() {
        let mut bau = policy_bau();
        let play = zone_asap(1, ZGO_PLAY);
        // Responses update only objects with the A flag: Play is a plain command object.
        bau.process_frame(&group_frame("1/0/1", Kind::Response, 1), 0);
        assert!(bau.group_objects().next_updated().is_none());
        // Writes update writable objects only.
        bau.process_frame(&group_frame("1/0/3", Kind::Write, 1), 0);
        assert!(
            bau.group_objects().next_updated().is_none(),
            "status object"
        );
        bau.process_frame(&group_frame("1/0/1", Kind::Write, 1), 0);
        assert_eq!(bau.group_objects().next_updated(), Some(play));
    }

    #[test]
    fn dispatch_waits_for_the_device_to_be_configured() {
        let mut bau = policy_bau();
        bau.process_frame(&group_frame("1/0/1", Kind::Write, 1), 0);
        send_management(&mut bau, &[0x03, 0xD7, 2, 5, 0x10, 0x01, 1]);
        assert!(!bau.configured());
        let (tx, mut rx) = mpsc::channel(4);
        dispatch_updated_gos(&mut bau, &tx, &mut Throttle::default());
        assert!(
            rx.try_recv().is_err(),
            "nothing is dispatched while unconfigured"
        );
        assert!(
            bau.group_objects().next_updated().is_none(),
            "but it is acknowledged"
        );
    }

    #[test]
    fn tables_installed_over_a_leftover_memory_image_hold_the_addresses() {
        // An interrupted download that was persisted leaves a memory image and an association
        // table that is still loading.
        let mut bau = build_bau(IndividualAddress::from_raw(0x1164));
        bau.set_memory_area(vec![0xAA; 100]);
        send_management(&mut bau, &[0x03, 0xD7, 2, 5, 0x10, 0x01, 1]);
        assert!(build_tables_from_config(&mut bau, &toml_config()));
        let s = TableSummary::of(&bau);
        assert!(s.configured);
        assert_eq!((s.group_addresses, s.associations), (11, 11));
        assert_eq!(bau.memory_area().len(), 0);
    }

    // ── Persisting ──────────────────────────────────────────────────

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("snapdog_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[cfg(unix)]
    #[test]
    fn the_memory_file_is_readable_by_its_owner_only() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = temp_dir("mode");
        let path = dir.join("knx-memory.bin");
        // A file left by an older release, with the umask's permissions.
        std::fs::write(&path, b"old").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();

        persist_memory(&path, &[1, 2, 3], IA).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "the file holds passwords, keys and the PSK");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn concurrent_writers_never_leave_a_mixed_file_or_a_temp_file() {
        // Two snapdog processes (the old one still shutting down, the new one starting) have
        // no lock in common: each writer needs a temp file of its own.
        let dir = temp_dir("writers");
        let path = dir.join("knx-memory.bin");
        let payloads = [vec![0xAA_u8; 30_000], vec![0x55_u8; 30_001]];
        let handles: Vec<_> = payloads
            .iter()
            .cloned()
            .map(|payload| {
                let path = path.clone();
                std::thread::spawn(move || {
                    for _ in 0..100 {
                        write_atomically(&path, &payload).unwrap();
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        let written = std::fs::read(&path).unwrap();
        assert!(payloads.contains(&written), "a complete file of one writer");
        let leftovers: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .filter(|n| n != "knx-memory.bin")
            .collect();
        assert!(
            leftovers.is_empty(),
            "temp files left behind: {leftovers:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_older_snapshot_does_not_replace_a_newer_file() {
        let dir = temp_dir("generation");
        let path = dir.join("knx-memory.bin");
        let newer = next_generation();
        let older = next_generation() - 2; // numbered before `newer`
        assert!(older < newer);
        persist_generation(&path, &[2, 2, 2], IA, newer).unwrap();
        persist_generation(&path, &[1, 1, 1], IA, older).unwrap();
        assert_eq!(load_memory(&path).unwrap().payload, vec![2, 2, 2]);
        // Another file is not affected by that bookkeeping.
        let other = dir.join("other.bin");
        persist_generation(&other, &[3], IA, older).unwrap();
        assert_eq!(load_memory(&other).unwrap().payload, vec![3]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn only_frames_to_this_device_count_as_programming() {
        let bau = policy_bau();
        let mgmt = management_frame(&bau, &[0x03, 0x00]).unwrap();
        assert!(is_management_for_device(&bau, &mgmt));
        // The same frame to another device, and group telegrams, do not.
        let mut other = Bau::new(
            device_object::new_device_object(device_serial(), group_objects::HARDWARE_TYPE),
            TOTAL_GO_COUNT as u16,
            DEFAULT_GO_SIZE,
        );
        device_object::set_individual_address(other.device_mut(), 0x1165);
        assert!(!is_management_for_device(&other, &mgmt));
        assert!(!is_management_for_device(
            &bau,
            &group_frame("1/0/1", Kind::Write, 1)
        ));
    }

    // ── Supervision ─────────────────────────────────────────────────

    fn supervised_task(
        bind: std::net::SocketAddr,
    ) -> (
        DeviceTask,
        mpsc::Sender<BauCmd>,
        Arc<Notify>,
        watch::Sender<bool>,
    ) {
        let (cmd_tx, cmd_rx) = mpsc::channel(8);
        let (update_tx, _update_rx) = mpsc::channel(8);
        let (addr_tx, _addr_rx) = watch::channel(None);
        let (shutdown_tx, shutdown) = watch::channel(false);
        let resync = Arc::new(Notify::new());
        let task = DeviceTask {
            bau: build_bau(IndividualAddress::from_raw(0x1164)),
            cmd_rx,
            update_tx,
            persist_path: None,
            restart_after_ets: false,
            config_ia: 0x1164,
            bind: Some(bind),
            addr_tx,
            resync: Arc::clone(&resync),
            shutdown,
            pending_persist: None,
        };
        (task, cmd_tx, resync, shutdown_tx)
    }

    #[tokio::test]
    async fn the_status_is_published_again_after_the_server_restarted() {
        let bind = "127.0.0.1:0".parse().unwrap();
        let (task, cmd_tx, resync, shutdown_tx) = supervised_task(bind);
        let first = open_server(Some(bind)).await.unwrap();
        let supervisor = tokio::spawn(device_supervisor(task, Some(first)));

        // The first server is not a restart: nobody has anything to republish yet.
        let quiet =
            tokio::time::timeout(std::time::Duration::from_millis(300), resync.notified()).await;
        assert!(quiet.is_err());

        cmd_tx.send(BauCmd::RestartServer).await.unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(5), resync.notified())
            .await
            .expect("the peers' view is stale after a restart: the publisher is told to send everything");

        shutdown_tx.send(true).unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(5), supervisor)
            .await
            .expect("the supervisor ends on shutdown")
            .unwrap();
    }

    /// The debounced write that is still running when the BAU task ends is waited for: the
    /// bridge's shutdown returns only once the file is complete.
    #[tokio::test]
    async fn the_end_of_the_bau_task_waits_for_the_write_that_is_running() {
        let (mut task, _cmd_tx, _resync, _shutdown_tx) =
            supervised_task("127.0.0.1:0".parse().unwrap());
        let written = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = Arc::clone(&written);
        task.pending_persist = Some(tokio::task::spawn_blocking(move || {
            std::thread::sleep(std::time::Duration::from_millis(300));
            flag.store(true, std::sync::atomic::Ordering::SeqCst);
        }));
        finish(&mut task).await;
        assert!(
            written.load(std::sync::atomic::Ordering::SeqCst),
            "finish() returned before the write was done"
        );
    }

    /// A stand-in for the BAU loop: each server lives for as long as the script says (a
    /// restart follows), and when the script is used up the loop ends for good.
    struct Lives {
        script: std::vec::IntoIter<std::time::Duration>,
        started: Arc<std::sync::Mutex<Vec<tokio::time::Instant>>>,
    }

    impl ServerLoop for Lives {
        async fn run(&mut self, _task: &mut DeviceTask, server: DeviceServer) -> LoopEnd {
            self.started
                .lock()
                .unwrap()
                .push(tokio::time::Instant::now());
            let Some(lives) = self.script.next() else {
                return LoopEnd::Stopped;
            };
            tokio::time::sleep(lives).await;
            drop(server);
            LoopEnd::ServerClosed
        }
    }

    /// The times at which the server was started, for servers that live as the script says.
    async fn start_times(script: Vec<std::time::Duration>) -> Vec<tokio::time::Instant> {
        let bind = "127.0.0.1:0".parse().unwrap();
        let (task, _cmd_tx, _resync, _shutdown_tx) = supervised_task(bind);
        let first = open_server(Some(bind)).await.unwrap();
        let started = Arc::new(std::sync::Mutex::new(Vec::new()));
        let serve = Lives {
            script: script.into_iter(),
            started: Arc::clone(&started),
        };
        device_supervisor_with(task, Some(first), serve).await;
        started.lock().unwrap().clone()
    }

    /// A server that stayed up for 30 s or more was healthy, so the delay before the next one
    /// starts over; a server that goes down again after a few seconds keeps the delays growing.
    /// (The mutation that replaces the call in the supervisor with a plain `backoff.failure()`
    /// is killed by the last assertion.)
    #[tokio::test(start_paused = true)]
    async fn a_device_server_that_lasted_starts_the_backoff_over() {
        use std::time::Duration;
        let short = Duration::from_secs(2);
        let long = super::super::supervisor::STABLE_AFTER + Duration::from_secs(1);
        let t = start_times(vec![short, short, short, long]).await;
        assert_eq!(t.len(), 5);
        let pause = |i: usize, lived: Duration| t[i + 1].duration_since(t[i] + lived);
        // 1st loss: 0.5 to 1 s, 2nd: 1 to 2 s, 3rd: 2 to 4 s.
        assert!(
            pause(0, short) <= Duration::from_millis(1050),
            "{:?}",
            pause(0, short)
        );
        assert!(
            pause(1, short) >= Duration::from_secs(1),
            "{:?}",
            pause(1, short)
        );
        assert!(
            pause(2, short) >= Duration::from_secs(2),
            "{:?}",
            pause(2, short)
        );
        // After a healthy server the next delay is the first one again.
        assert!(
            pause(3, long) <= Duration::from_millis(1050),
            "waited {:?} after a healthy server",
            pause(3, long)
        );
    }

    #[tokio::test]
    async fn a_server_that_could_not_bind_at_first_publishes_when_it_comes_up() {
        let blocker = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        let bind = blocker.local_addr().unwrap();
        let (task, _cmd_tx, resync, shutdown_tx) = supervised_task(bind);
        let supervisor = tokio::spawn(device_supervisor(task, None));
        let quiet =
            tokio::time::timeout(std::time::Duration::from_millis(300), resync.notified()).await;
        assert!(quiet.is_err(), "no server yet");
        drop(blocker);
        tokio::time::timeout(std::time::Duration::from_secs(10), resync.notified())
            .await
            .expect("everything offered while there was no server goes out once it is up");
        shutdown_tx.send(true).unwrap();
        supervisor.await.unwrap();
    }

    #[test]
    fn frames_leave_the_server_as_indications() {
        let mut bau = bau_from_toml(&toml_config());
        let go = bau
            .group_objects_mut()
            .get_mut(zone_asap(1, ZGO_VOLUME_STATUS))
            .unwrap();
        go.set_value(&DptValue::Float(40.0)).unwrap();
        bau.poll(0);
        let req = bau.next_outgoing_frame().unwrap();
        assert_eq!(req.message_code_raw(), 0x11, "the stack builds requests");
        let ind = to_indication(req.clone());
        assert_eq!(ind.message_code_raw(), 0x29);
        // Nothing else about the telegram changes.
        assert_eq!(ind.destination_address_raw(), req.destination_address_raw());
        assert_eq!(ind.source_address(), req.source_address());
        assert_eq!(ind.as_group_write(), req.as_group_write());
        // A frame that is not a request is left alone.
        assert_eq!(to_indication(ind.clone()).as_bytes(), ind.as_bytes());
    }
}
