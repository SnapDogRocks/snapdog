// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! Group object definitions — single source of truth for all KNX communication objects.
//!
//! Used by:
//! - Device mode runtime (BAU `GroupObjectStore` construction)
//! - `cargo xtask generate-knxprod-xml` (`OpenKNXproducer` XML generation)

use knx_rs_core::dpt::{DPT_SCALING, DPT_STRING_8859_1, DPT_SWITCH, DPT_VALUE_1_UCOUNT, Dpt};

/// Maximum number of zones supported.
pub const MAX_ZONES: usize = 10;

/// Maximum number of clients supported.
pub const MAX_CLIENTS: usize = 30;

/// Maximum number of HTTP API keys supported. The active count is chosen in ETS via a
/// dropdown (default 1); keys `1..=count` are shown and read.
pub const MAX_API_KEYS: usize = 10;

/// Number of group objects per zone.
pub const ZONE_GO_COUNT: usize = ZONE_GOS.len();

/// Number of group objects per client.
pub const CLIENT_GO_COUNT: usize = CLIENT_GOS.len();

/// Total number of group objects.
pub const TOTAL_GO_COUNT: usize =
    MAX_ZONES * ZONE_GO_COUNT + MAX_CLIENTS * CLIENT_GO_COUNT + GLOBAL_GO_COUNT;

/// Device hardware type, served as `PID_HARDWARE_TYPE` (device object, PID 78).
///
/// **SSOT.** At the start of every download ETS reads PID 78 and compares it to
/// the `.knxprod`'s `LdCtrlCompareProp InlineData`; a mismatch aborts programming
/// with a firmware/version error before any table is touched. This constant is
/// both served by the firmware (`device.rs`) and emitted into the `.knxprod`
/// (`xtask`), so the compare can never drift. Encodes hardware order number
/// `0xFF01`, version `01` (hex `0000FF010100`).
pub const HARDWARE_TYPE: [u8; 6] = [0x00, 0x00, 0xFF, 0x01, 0x01, 0x00];

/// ETS product identity — SSOT shared by the `.knxprod` (`xtask`) and the `WebUI`
/// product-info endpoint, so the version shown to an integrator always matches the
/// downloaded database.
///
/// **Bump [`KNXPROD_APP_VERSION`] whenever the KNX memory layout changes** — ETS decides
/// its download scope by this version, so an unchanged version after a layout change
/// would leave a device mis-parameterized (see the xtask layout-lock guard).
pub const KNXPROD_APP_VERSION: u32 = 11;
/// ETS `ApplicationNumber` (== hardware order number `0xFF01`).
pub const KNXPROD_APP_NUMBER: u16 = 0xFF01;
/// Hardware revision (the version byte of [`HARDWARE_TYPE`]).
pub const KNXPROD_HW_VERSION: u8 = HARDWARE_TYPE[4];

/// KNX communication object flags (matching ETS flag bits).
pub struct GoFlags {
    /// Communication enabled (K-flag).
    pub communicate: bool,
    /// Read enabled (L-flag) — bus can read this object.
    pub read: bool,
    /// Write enabled (S-flag) — bus can write this object.
    pub write: bool,
    /// Transmit on change (Ü-flag) — send to bus when value changes.
    pub transmit: bool,
    /// Update on response (A-flag) — update value from `GroupValueResponse`.
    pub update: bool,
}

/// Shorthand flag sets.
const RECV: GoFlags = GoFlags {
    communicate: true,
    read: false,
    write: true,
    transmit: false,
    update: false,
};
const SEND: GoFlags = GoFlags {
    communicate: true,
    read: true,
    write: false,
    transmit: true,
    update: false,
};
const BIDIR: GoFlags = GoFlags {
    communicate: true,
    read: true,
    write: true,
    transmit: true,
    update: true,
};

/// DPT 3.007 — Controlled dimming.
pub const DPT_CONTROL_DIMMING: Dpt = Dpt::new(3, 7);

/// DPT 1.018 — Occupancy (presence sensor).
const DPT_OCCUPANCY: Dpt = Dpt::new(1, 18);

/// DPT 7.005 — Time period in seconds (`UInt16`).
pub const DPT_TIME_PERIOD_SEC: Dpt = Dpt::new(7, 5);

/// DPT 1.011 — State (used for the cyclic "server online" heartbeat).
pub const DPT_STATE: Dpt = Dpt::new(1, 11);

/// DPT 1.017 — Trigger (used for the global "all stop" command).
pub const DPT_TRIGGER: Dpt = Dpt::new(1, 17);

/// DPT 1.005 — Alarm (used for the global "system fault" status).
pub const DPT_ALARM: Dpt = Dpt::new(1, 5);

/// DPT 10.001 — Time of day (3 bytes; KNX clock input for presence schedules).
pub const DPT_TIME_OF_DAY: Dpt = Dpt::new(10, 1);

/// Definition of a single group object.
pub struct GoDefinition {
    /// Human-readable name (used in ETS and logs).
    pub name: &'static str,
    /// German display name (ETS Text attribute).
    pub name_de: &'static str,
    /// English display name (ETS `FunctionText` attribute).
    pub name_en: &'static str,
    /// KNX datapoint type.
    pub dpt: Dpt,
    /// ETS DPT string (e.g. "DPST-1-1").
    pub dpt_str: &'static str,
    /// ETS `ObjectSize` string (e.g. "1 Bit").
    pub size_str: &'static str,
    /// Communication flags.
    pub flags: GoFlags,
}

impl GoFlags {
    /// Encode as a 16-bit group object descriptor (upper bits).
    #[must_use]
    pub const fn to_descriptor_bits(&self, size_code: u8) -> u16 {
        let mut bits: u16 = 0;
        if self.communicate {
            bits |= 1 << 10;
        }
        if self.read {
            bits |= 1 << 11;
        }
        if self.write {
            bits |= 1 << 12;
        }
        if self.transmit {
            bits |= 1 << 14;
        }
        if self.update {
            bits |= 1 << 15;
        }
        bits | (size_code as u16)
    }
}

// ── Zone group objects (35 per zone) ──────────────────────────

/// Create a receive-only GO definition.
const fn go_recv(
    name: &'static str,
    de: &'static str,
    en: &'static str,
    dpt: Dpt,
    dpt_s: &'static str,
    size: &'static str,
) -> GoDefinition {
    GoDefinition {
        name,
        name_de: de,
        name_en: en,
        dpt,
        dpt_str: dpt_s,
        size_str: size,
        flags: RECV,
    }
}
/// Create a send-only GO definition.
const fn go_send(
    name: &'static str,
    de: &'static str,
    en: &'static str,
    dpt: Dpt,
    dpt_s: &'static str,
    size: &'static str,
) -> GoDefinition {
    GoDefinition {
        name,
        name_de: de,
        name_en: en,
        dpt,
        dpt_str: dpt_s,
        size_str: size,
        flags: SEND,
    }
}
/// Create a bidirectional GO definition.
const fn go_bidir(
    name: &'static str,
    de: &'static str,
    en: &'static str,
    dpt: Dpt,
    dpt_s: &'static str,
    size: &'static str,
) -> GoDefinition {
    GoDefinition {
        name,
        name_de: de,
        name_en: en,
        dpt,
        dpt_str: dpt_s,
        size_str: size,
        flags: BIDIR,
    }
}

/// Zone group object definitions (35 per zone).
pub const ZONE_GOS: &[GoDefinition] = &[
    go_recv("Play", "Play", "Play", DPT_SWITCH, "DPST-1-1", "1 Bit"),
    go_recv("Pause", "Pause", "Pause", DPT_SWITCH, "DPST-1-1", "1 Bit"),
    go_recv("Stop", "Stop", "Stop", DPT_SWITCH, "DPST-1-1", "1 Bit"),
    go_recv(
        "Track Next",
        "Nächster Titel",
        "Next Track",
        DPT_SWITCH,
        "DPST-1-1",
        "1 Bit",
    ),
    go_recv(
        "Track Previous",
        "Vorheriger Titel",
        "Previous Track",
        DPT_SWITCH,
        "DPST-1-1",
        "1 Bit",
    ),
    go_recv(
        "Volume",
        "Lautstärke",
        "Volume",
        DPT_SCALING,
        "DPST-5-1",
        "1 Byte",
    ),
    go_send(
        "Volume Status",
        "Lautstärke Status",
        "Volume Status",
        DPT_SCALING,
        "DPST-5-1",
        "1 Byte",
    ),
    go_recv(
        "Volume Dim",
        "Lautstärke Dimmen",
        "Volume Dim",
        DPT_CONTROL_DIMMING,
        "DPST-3-7",
        "4 Bit",
    ),
    go_recv("Mute", "Stumm", "Mute", DPT_SWITCH, "DPST-1-1", "1 Bit"),
    go_send(
        "Mute Status",
        "Stumm Status",
        "Mute Status",
        DPT_SWITCH,
        "DPST-1-1",
        "1 Bit",
    ),
    go_recv(
        "Mute Toggle",
        "Stumm Umschalten",
        "Mute Toggle",
        DPT_SWITCH,
        "DPST-1-1",
        "1 Bit",
    ),
    go_send(
        "Control Status",
        "Wiedergabe Status",
        "Playback Status",
        DPT_SWITCH,
        "DPST-1-1",
        "1 Bit",
    ),
    go_send(
        "Track Playing",
        "Titel spielt",
        "Track Playing",
        DPT_SWITCH,
        "DPST-1-1",
        "1 Bit",
    ),
    go_recv(
        "Shuffle",
        "Zufallswiedergabe",
        "Shuffle",
        DPT_SWITCH,
        "DPST-1-1",
        "1 Bit",
    ),
    go_send(
        "Shuffle Status",
        "Zufall Status",
        "Shuffle Status",
        DPT_SWITCH,
        "DPST-1-1",
        "1 Bit",
    ),
    go_recv(
        "Shuffle Toggle",
        "Zufall Umschalten",
        "Shuffle Toggle",
        DPT_SWITCH,
        "DPST-1-1",
        "1 Bit",
    ),
    go_recv(
        "Repeat",
        "Wiederholung",
        "Repeat",
        DPT_SWITCH,
        "DPST-1-1",
        "1 Bit",
    ),
    go_send(
        "Repeat Status",
        "Wiederholung Status",
        "Repeat Status",
        DPT_SWITCH,
        "DPST-1-1",
        "1 Bit",
    ),
    go_recv(
        "Repeat Toggle",
        "Wiederholung Umsch.",
        "Repeat Toggle",
        DPT_SWITCH,
        "DPST-1-1",
        "1 Bit",
    ),
    go_recv(
        "Track Repeat",
        "Titel Wiederholung",
        "Track Repeat",
        DPT_SWITCH,
        "DPST-1-1",
        "1 Bit",
    ),
    go_send(
        "Track Repeat Status",
        "Titel Wdh. Status",
        "Track Repeat Status",
        DPT_SWITCH,
        "DPST-1-1",
        "1 Bit",
    ),
    go_recv(
        "Track Repeat Toggle",
        "Titel Wdh. Umsch.",
        "Track Repeat Toggle",
        DPT_SWITCH,
        "DPST-1-1",
        "1 Bit",
    ),
    go_recv(
        "Playlist",
        "Playlist",
        "Playlist",
        DPT_VALUE_1_UCOUNT,
        "DPST-5-10",
        "1 Byte",
    ),
    go_send(
        "Playlist Status",
        "Playlist Status",
        "Playlist Status",
        DPT_VALUE_1_UCOUNT,
        "DPST-5-10",
        "1 Byte",
    ),
    go_recv(
        "Playlist Next",
        "Nächste Playlist",
        "Next Playlist",
        DPT_SWITCH,
        "DPST-1-1",
        "1 Bit",
    ),
    go_recv(
        "Playlist Previous",
        "Vorherige Playlist",
        "Previous Playlist",
        DPT_SWITCH,
        "DPST-1-1",
        "1 Bit",
    ),
    go_send(
        "Track Title",
        "Titel",
        "Track Title",
        DPT_STRING_8859_1,
        "DPST-16-1",
        "14 Bytes",
    ),
    go_send(
        "Track Artist",
        "Interpret",
        "Track Artist",
        DPT_STRING_8859_1,
        "DPST-16-1",
        "14 Bytes",
    ),
    go_send(
        "Track Album",
        "Album",
        "Track Album",
        DPT_STRING_8859_1,
        "DPST-16-1",
        "14 Bytes",
    ),
    go_send(
        "Track Progress",
        "Fortschritt",
        "Track Progress",
        DPT_SCALING,
        "DPST-5-1",
        "1 Byte",
    ),
    // Retriggerable presence trigger: each telegram (re)arms the auto-off timer and starts
    // playback of the zone's configured presence source. Timeout & source are ETS
    // parameters (Präsenz Auto-Off / Präsenz-Quelle), not group objects.
    go_recv(
        "Presence",
        "Präsenz",
        "Presence",
        DPT_OCCUPANCY,
        "DPST-1-18",
        "1 Bit",
    ),
    go_bidir(
        "Presence Enable",
        "Präsenz Aktiviert",
        "Presence Enable",
        DPT_SWITCH,
        "DPST-1-1",
        "1 Bit",
    ),
    go_send(
        "Presence Timer Active",
        "Präsenz Timer",
        "Presence Timer",
        DPT_SWITCH,
        "DPST-1-1",
        "1 Bit",
    ),
];

// ── Client group objects (11 per client) ──────────────────────

/// Client group object definitions (11 per client).
pub const CLIENT_GOS: &[GoDefinition] = &[
    go_recv(
        "Volume",
        "Lautstärke",
        "Volume",
        DPT_SCALING,
        "DPST-5-1",
        "1 Byte",
    ),
    go_send(
        "Volume Status",
        "Lautstärke Status",
        "Volume Status",
        DPT_SCALING,
        "DPST-5-1",
        "1 Byte",
    ),
    go_recv(
        "Volume Dim",
        "Lautstärke Dimmen",
        "Volume Dim",
        DPT_CONTROL_DIMMING,
        "DPST-3-7",
        "4 Bit",
    ),
    go_recv("Mute", "Stumm", "Mute", DPT_SWITCH, "DPST-1-1", "1 Bit"),
    go_send(
        "Mute Status",
        "Stumm Status",
        "Mute Status",
        DPT_SWITCH,
        "DPST-1-1",
        "1 Bit",
    ),
    go_recv(
        "Mute Toggle",
        "Stumm Umschalten",
        "Mute Toggle",
        DPT_SWITCH,
        "DPST-1-1",
        "1 Bit",
    ),
    go_recv(
        "Latency",
        "Latenz",
        "Latency",
        DPT_VALUE_1_UCOUNT,
        "DPST-5-10",
        "1 Byte",
    ),
    go_send(
        "Latency Status",
        "Latenz Status",
        "Latency Status",
        DPT_VALUE_1_UCOUNT,
        "DPST-5-10",
        "1 Byte",
    ),
    go_bidir(
        "Zone",
        "Zonenzuordnung",
        "Zone Assignment",
        DPT_VALUE_1_UCOUNT,
        "DPST-5-10",
        "1 Byte",
    ),
    go_send(
        "Zone Status",
        "Zone Status",
        "Zone Status",
        DPT_VALUE_1_UCOUNT,
        "DPST-5-10",
        "1 Byte",
    ),
    go_send(
        "Connected",
        "Verbunden",
        "Connected",
        DPT_SWITCH,
        "DPST-1-1",
        "1 Bit",
    ),
];

// ── Global group objects (device-level, not per zone/client) ──
//
// One set for the whole server. Emitted after all zone and client COs in the ComObject
// table and shown under a dedicated "System" drawer in ETS.

/// Device-level group objects, shared by the whole server.
pub const GLOBAL_GOS: &[GoDefinition] = &[
    // Cyclic heartbeat — sent every `heartbeat` interval (an ETS parameter) so the bus can
    // detect the server going offline.
    go_send(
        "Server Online",
        "Server Online",
        "Server Online",
        DPT_STATE,
        "DPST-1-11",
        "1 Bit",
    ),
    // Stop playback in every zone (e.g. a leaving-home scene).
    go_recv(
        "All Stop",
        "Alle Stopp",
        "All Stop",
        DPT_TRIGGER,
        "DPST-1-17",
        "1 Bit",
    ),
    // Mute / unmute every zone at once; the status is sent back on change.
    go_bidir(
        "All Mute",
        "Alle Stumm",
        "All Mute",
        DPT_SWITCH,
        "DPST-1-1",
        "1 Bit",
    ),
    // Set when the server enters a fault state, cleared when it recovers.
    go_send(
        "System Fault",
        "Systemstörung",
        "System Fault",
        DPT_ALARM,
        "DPST-1-5",
        "1 Bit",
    ),
    // KNX clock input — syncs the local time that drives presence schedules.
    go_recv(
        "KNX Time",
        "KNX Uhrzeit",
        "KNX Time",
        DPT_TIME_OF_DAY,
        "DPST-10-1",
        "3 Bytes",
    ),
];

/// Number of global group objects.
pub const GLOBAL_GO_COUNT: usize = GLOBAL_GOS.len();

// ── Named GO indices (global) ─────────────────────────────────

/// Global GO index — cyclic "server online" heartbeat (send).
pub const GGO_SERVER_ONLINE: usize = 0;
/// Global GO index — "all stop" trigger (receive).
pub const GGO_ALL_STOP: usize = 1;
/// Global GO index — "all mute" switch (bidirectional).
pub const GGO_ALL_MUTE: usize = 2;
/// Global GO index — "system fault" alarm (send).
pub const GGO_SYSTEM_FAULT: usize = 3;
/// Global GO index — KNX time-of-day clock input (receive).
pub const GGO_KNX_TIME: usize = 4;

/// Compute the 1-based ASAP for a global group object (0-based `go_index` within
/// [`GLOBAL_GOS`]). Globals follow every zone and client GO.
#[must_use]
pub const fn global_asap(go_index: usize) -> u16 {
    (MAX_ZONES * ZONE_GO_COUNT + MAX_CLIENTS * CLIENT_GO_COUNT + go_index + 1) as u16
}

/// Compute the 1-based ASAP for a zone group object.
///
/// Zone `zone_index` (1-based), GO `go_index` (0-based within `ZONE_GOS`).
#[must_use]
pub const fn zone_asap(zone_index: usize, go_index: usize) -> u16 {
    ((zone_index - 1) * ZONE_GO_COUNT + go_index + 1) as u16
}

// ── Named GO indices (zone) ───────────────────────────────────
// Use these instead of magic numbers when mapping GAs to GOs.

/// Zone GO index.
pub const ZGO_PLAY: usize = 0;
/// Zone GO index.
pub const ZGO_PAUSE: usize = 1;
/// Zone GO index.
pub const ZGO_STOP: usize = 2;
/// Zone GO index.
pub const ZGO_TRACK_NEXT: usize = 3;
/// Zone GO index.
pub const ZGO_TRACK_PREVIOUS: usize = 4;
/// Zone GO index.
pub const ZGO_VOLUME: usize = 5;
/// Zone GO index.
pub const ZGO_VOLUME_STATUS: usize = 6;
/// Zone GO index.
pub const ZGO_VOLUME_DIM: usize = 7;
/// Zone GO index.
pub const ZGO_MUTE: usize = 8;
/// Zone GO index.
pub const ZGO_MUTE_STATUS: usize = 9;
/// Zone GO index.
pub const ZGO_MUTE_TOGGLE: usize = 10;
/// Zone GO index.
pub const ZGO_CONTROL_STATUS: usize = 11;
/// Zone GO index.
pub const ZGO_TRACK_PLAYING: usize = 12;
/// Zone GO index.
pub const ZGO_SHUFFLE: usize = 13;
/// Zone GO index.
pub const ZGO_SHUFFLE_STATUS: usize = 14;
/// Zone GO index.
pub const ZGO_SHUFFLE_TOGGLE: usize = 15;
/// Zone GO index.
pub const ZGO_REPEAT: usize = 16;
/// Zone GO index.
pub const ZGO_REPEAT_STATUS: usize = 17;
/// Zone GO index.
pub const ZGO_REPEAT_TOGGLE: usize = 18;
/// Zone GO index.
pub const ZGO_TRACK_REPEAT: usize = 19;
/// Zone GO index.
pub const ZGO_TRACK_REPEAT_STATUS: usize = 20;
/// Zone GO index.
pub const ZGO_TRACK_REPEAT_TOGGLE: usize = 21;
/// Zone GO index.
pub const ZGO_PLAYLIST: usize = 22;
/// Zone GO index.
pub const ZGO_PLAYLIST_STATUS: usize = 23;
/// Zone GO index.
pub const ZGO_PLAYLIST_NEXT: usize = 24;
/// Zone GO index.
pub const ZGO_PLAYLIST_PREVIOUS: usize = 25;
/// Zone GO index.
pub const ZGO_TRACK_TITLE: usize = 26;
/// Zone GO index.
pub const ZGO_TRACK_ARTIST: usize = 27;
/// Zone GO index.
pub const ZGO_TRACK_ALBUM: usize = 28;
/// Zone GO index.
pub const ZGO_TRACK_PROGRESS: usize = 29;
/// Zone GO index — retriggerable presence trigger input.
pub const ZGO_PRESENCE: usize = 30;
/// Zone GO index.
pub const ZGO_PRESENCE_ENABLE: usize = 31;
/// Zone GO index — status: presence-triggered playback / auto-off timer active.
pub const ZGO_PRESENCE_TIMER_ACTIVE: usize = 32;

// ── Named GO indices (client) ─────────────────────────────────

/// Client GO index.
pub const CGO_VOLUME: usize = 0;
/// Client GO index.
pub const CGO_VOLUME_STATUS: usize = 1;
/// Client GO index.
pub const CGO_VOLUME_DIM: usize = 2;
/// Client GO index.
pub const CGO_MUTE: usize = 3;
/// Client GO index.
pub const CGO_MUTE_STATUS: usize = 4;
/// Client GO index.
pub const CGO_MUTE_TOGGLE: usize = 5;
/// Client GO index.
pub const CGO_LATENCY: usize = 6;
/// Client GO index.
pub const CGO_LATENCY_STATUS: usize = 7;
/// Client GO index.
pub const CGO_ZONE: usize = 8;
/// Client GO index.
pub const CGO_ZONE_STATUS: usize = 9;
/// Client GO index.
pub const CGO_CONNECTED: usize = 10;

// ── ETS Memory Layout (SSOT — used by xtask and device.rs) ───

/// Byte offsets for ETS parameters in BAU memory.
pub mod mem {
    use super::{MAX_API_KEYS, MAX_CLIENTS, MAX_ZONES};

    /// Number of active zones (1 byte).
    ///
    /// Zones `1..=NUM_ZONES` are active; higher zones are hidden in ETS and inactive at
    /// runtime. Replaces the former 10 per-zone active flags with a single count.
    pub const NUM_ZONES: usize = 0;
    /// Zone default volume (10 × 1 byte).
    pub const ZONE_DEF_VOL: usize = NUM_ZONES + 1;
    /// Zone max volume (10 × 1 byte).
    pub const ZONE_MAX_VOL: usize = ZONE_DEF_VOL + MAX_ZONES;
    /// Zone `AirPlay` enabled (10 × 1 byte).
    pub const ZONE_AIRPLAY: usize = ZONE_MAX_VOL + MAX_ZONES;
    /// Zone Spotify enabled (10 × 1 byte).
    pub const ZONE_SPOTIFY: usize = ZONE_AIRPLAY + MAX_ZONES;
    /// Zone presence enabled (10 × 1 byte).
    pub const ZONE_PRESENCE_EN: usize = ZONE_SPOTIFY + MAX_ZONES;
    /// Zone presence timeout (10 × 2 bytes).
    pub const ZONE_PRESENCE_TO: usize = ZONE_PRESENCE_EN + MAX_ZONES;
    /// Zone presence default source (10 × 1 byte; 0 = none, 1..=`MAX_RADIOS` = radio index).
    pub const ZONE_PRES_SRC: usize = ZONE_PRESENCE_TO + MAX_ZONES * 2;
    // Sample rate and bit depth are the global Snapcast output format, not per-zone —
    // see GLOBAL_SRATE / GLOBAL_BITD below.
    /// Clients `1..=NUM_CLIENTS` are active; higher clients are hidden in ETS and inactive
    /// at runtime. Mirrors [`NUM_ZONES`] — replaces the former 10 per-client active flags.
    pub const NUM_CLIENTS: usize = ZONE_PRES_SRC + MAX_ZONES;
    /// Client default zone (10 × 1 byte).
    pub const CLIENT_DEF_ZONE: usize = NUM_CLIENTS + 1;
    /// Client default volume (10 × 1 byte).
    pub const CLIENT_DEF_VOL: usize = CLIENT_DEF_ZONE + MAX_CLIENTS;
    /// Client max volume (10 × 1 byte).
    pub const CLIENT_MAX_VOL: usize = CLIENT_DEF_VOL + MAX_CLIENTS;
    /// Client default latency (10 × 1 byte).
    pub const CLIENT_DEF_LAT: usize = CLIENT_MAX_VOL + MAX_CLIENTS;
    /// Global HTTP port (2 bytes).
    pub const GLOBAL_HTTP_PORT: usize = CLIENT_DEF_LAT + MAX_CLIENTS;
    /// Global log level enum (1 byte).
    pub const GLOBAL_LOG_LVL: usize = GLOBAL_HTTP_PORT + 2;
    /// Global audio sample rate, server-wide (1 byte; index 0=44100, 1=48000, 2=96000 Hz).
    pub const GLOBAL_SRATE: usize = GLOBAL_LOG_LVL + 1;
    /// Global audio bit depth, server-wide (1 byte; index 0=16, 1=24, 2=32 bit).
    pub const GLOBAL_BITD: usize = GLOBAL_SRATE + 1;
    /// Global Snapcast codec (1 byte; index 0=pcm, 1=flac, 2=f32lz4, 3=f32lz4e).
    pub const GLOBAL_CODEC: usize = GLOBAL_BITD + 1;
    /// Global source-conflict policy (1 byte; index 0 = `last_wins`, 1 = `receiver_wins`).
    pub const GLOBAL_SRC_CONFLICT: usize = GLOBAL_CODEC + 1;
    /// Global zone-switch fade duration in ms (2 bytes).
    pub const GLOBAL_ZONE_FADE: usize = GLOBAL_SRC_CONFLICT + 1;
    /// Global source-switch fade duration in ms (2 bytes).
    pub const GLOBAL_SRC_FADE: usize = GLOBAL_ZONE_FADE + 2;
    /// Number of active radio stations (1 byte). Radios `1..=NUM_RADIOS` are active and
    /// shown in ETS; higher ones are hidden. Mirrors [`NUM_ZONES`].
    pub const NUM_RADIOS: usize = GLOBAL_SRC_FADE + 2;

    // ── String parameters (after numeric block) ───────────────

    /// Maximum radio stations.
    pub const MAX_RADIOS: usize = 50;

    /// End of numeric parameters.
    const NUMERIC_END: usize = NUM_RADIOS + 1;

    /// Zone names (10 × 20 bytes).
    pub const ZONE_NAME: usize = NUMERIC_END;
    /// Zone name size in bytes.
    pub const ZONE_NAME_SIZE: usize = 20;
    /// Client names (10 × 20 bytes).
    pub const CLIENT_NAME: usize = ZONE_NAME + MAX_ZONES * ZONE_NAME_SIZE;
    /// Client name size in bytes.
    pub const CLIENT_NAME_SIZE: usize = 20;
    /// Client MAC addresses (10 × 17 bytes).
    pub const CLIENT_MAC: usize = CLIENT_NAME + MAX_CLIENTS * CLIENT_NAME_SIZE;
    /// Client MAC size in bytes.
    pub const CLIENT_MAC_SIZE: usize = 17;
    /// Subsonic URL (60 bytes).
    pub const GLOBAL_SUB_URL: usize = CLIENT_MAC + MAX_CLIENTS * CLIENT_MAC_SIZE;
    /// Subsonic URL size in bytes.
    pub const GLOBAL_SUB_URL_SIZE: usize = 60;
    /// Subsonic user (20 bytes).
    pub const GLOBAL_SUB_USER: usize = GLOBAL_SUB_URL + GLOBAL_SUB_URL_SIZE;
    /// Subsonic user size in bytes.
    pub const GLOBAL_SUB_USER_SIZE: usize = 20;
    /// Subsonic password (20 bytes).
    pub const GLOBAL_SUB_PASS: usize = GLOBAL_SUB_USER + GLOBAL_SUB_USER_SIZE;
    /// Subsonic password size in bytes.
    pub const GLOBAL_SUB_PASS_SIZE: usize = 20;
    /// MQTT broker (40 bytes).
    pub const GLOBAL_MQTT_BROKER: usize = GLOBAL_SUB_PASS + GLOBAL_SUB_PASS_SIZE;
    /// MQTT broker size in bytes.
    pub const GLOBAL_MQTT_BROKER_SIZE: usize = 40;
    /// MQTT base topic (20 bytes).
    pub const GLOBAL_MQTT_TOPIC: usize = GLOBAL_MQTT_BROKER + GLOBAL_MQTT_BROKER_SIZE;
    /// MQTT base topic size in bytes.
    pub const GLOBAL_MQTT_TOPIC_SIZE: usize = 20;
    /// Radio station names (20 × 20 bytes).
    pub const RADIO_NAME: usize = GLOBAL_MQTT_TOPIC + GLOBAL_MQTT_TOPIC_SIZE;
    /// Radio name size in bytes.
    pub const RADIO_NAME_SIZE: usize = 20;
    /// Radio station URLs (20 × 80 bytes).
    pub const RADIO_URL: usize = RADIO_NAME + MAX_RADIOS * RADIO_NAME_SIZE;
    /// Radio URL size in bytes.
    pub const RADIO_URL_SIZE: usize = 80;
    /// Radio cover art URLs (20 × 80 bytes).
    pub const RADIO_COVER: usize = RADIO_URL + MAX_RADIOS * RADIO_URL_SIZE;
    /// Radio cover size in bytes.
    pub const RADIO_COVER_SIZE: usize = 80;
    /// Zone `AirPlay` device names (10 × 20 bytes).
    pub const ZONE_AIRPLAY_NAME: usize = RADIO_COVER + MAX_RADIOS * RADIO_COVER_SIZE;
    /// Zone `AirPlay` name size in bytes.
    pub const ZONE_AIRPLAY_NAME_SIZE: usize = 20;
    /// Zone Spotify device names (10 × 20 bytes).
    pub const ZONE_SPOTIFY_NAME: usize = ZONE_AIRPLAY_NAME + MAX_ZONES * ZONE_AIRPLAY_NAME_SIZE;
    /// Zone Spotify name size in bytes.
    pub const ZONE_SPOTIFY_NAME_SIZE: usize = 20;
    /// Client icons (10 × 20 bytes).
    pub const CLIENT_ICON: usize = ZONE_SPOTIFY_NAME + MAX_ZONES * ZONE_SPOTIFY_NAME_SIZE;
    /// Client icon size in bytes.
    pub const CLIENT_ICON_SIZE: usize = 20;
    /// `AirPlay` password (20 bytes). Secret — stored plaintext in ETS memory by design.
    pub const GLOBAL_AIRPLAY_PASS: usize = CLIENT_ICON + MAX_CLIENTS * CLIENT_ICON_SIZE;
    /// `AirPlay` password size in bytes.
    pub const GLOBAL_AIRPLAY_PASS_SIZE: usize = 20;
    /// MQTT password (20 bytes). Secret — stored plaintext in ETS memory by design.
    pub const GLOBAL_MQTT_PASS: usize = GLOBAL_AIRPLAY_PASS + GLOBAL_AIRPLAY_PASS_SIZE;
    /// MQTT password size in bytes.
    pub const GLOBAL_MQTT_PASS_SIZE: usize = 20;
    /// Snapcast encryption PSK (64 bytes). Secret — stored plaintext in ETS memory by design.
    pub const GLOBAL_PSK: usize = GLOBAL_MQTT_PASS + GLOBAL_MQTT_PASS_SIZE;
    /// PSK size in bytes.
    pub const GLOBAL_PSK_SIZE: usize = 64;
    /// Number of active HTTP API keys (1 byte). Keys `1..=count` are shown in ETS and read
    /// by the firmware; mirrors [`NUM_ZONES`]. Chosen via a dropdown, default 1.
    pub const GLOBAL_NUM_API_KEYS: usize = GLOBAL_PSK + GLOBAL_PSK_SIZE;
    /// HTTP API key slots (`MAX_API_KEYS` × 40 bytes). Secrets — stored plaintext in ETS
    /// memory by design. Slot `i` (0-based) is at `GLOBAL_API_KEYS + i * GLOBAL_API_KEY_SIZE`.
    pub const GLOBAL_API_KEYS: usize = GLOBAL_NUM_API_KEYS + 1;
    /// HTTP API key slot size in bytes.
    pub const GLOBAL_API_KEY_SIZE: usize = 40;
    /// Heartbeat interval index (1 byte; enum → 1/3/5/10/15/30/45/60 minutes) for the
    /// global "Server Online" cyclic send.
    pub const GLOBAL_HEARTBEAT: usize = GLOBAL_API_KEYS + MAX_API_KEYS * GLOBAL_API_KEY_SIZE;

    /// Total memory size in bytes (numeric + strings).
    pub const TOTAL: usize = GLOBAL_HEARTBEAT + 1;
}

/// Compute the 1-based ASAP for a client group object.
///
/// Client `client_index` (1-based), GO `go_index` (0-based within `CLIENT_GOS`).
#[must_use]
pub const fn client_asap(client_index: usize, go_index: usize) -> u16 {
    (MAX_ZONES * ZONE_GO_COUNT + (client_index - 1) * CLIENT_GO_COUNT + go_index + 1) as u16
}

#[cfg(test)]
#[allow(clippy::assertions_on_constants)]
mod tests {
    use super::*;

    #[test]
    fn zone_go_count() {
        assert_eq!(ZONE_GOS.len(), 33);
    }

    #[test]
    fn client_go_count() {
        assert_eq!(CLIENT_GOS.len(), 11);
    }

    #[test]
    fn total_go_count() {
        // 10 zones × 33 + 30 clients × 11 + 5 global.
        assert_eq!(TOTAL_GO_COUNT, 665);
    }

    #[test]
    fn zone_asap_layout() {
        // Zone 1, first GO → ASAP 1
        assert_eq!(zone_asap(1, 0), 1);
        // Zone 1, last GO (index 32) → ASAP 33
        assert_eq!(zone_asap(1, 32), 33);
        // Zone 2, first GO → ASAP 34
        assert_eq!(zone_asap(2, 0), 34);
        // Zone 10, last GO → ASAP 330
        assert_eq!(zone_asap(10, 32), 330);
    }

    #[test]
    fn client_asap_layout() {
        // Client 1, first GO → ASAP 331 (after 10×33 zone GOs)
        assert_eq!(client_asap(1, 0), 331);
        // Client 1, last GO → ASAP 341
        assert_eq!(client_asap(1, 10), 341);
        // Client 10, last GO → ASAP 440
        assert_eq!(client_asap(10, 10), 440);
    }

    #[test]
    fn global_asap_layout() {
        // Globals follow all zone + client GOs: 10×33 + 30×11 = 660 → first global ASAP 661.
        assert_eq!(global_asap(0), 661);
        assert_eq!(global_asap(GLOBAL_GOS.len() - 1), 665);
    }

    #[test]
    fn no_asap_overlap() {
        let last_zone = zone_asap(MAX_ZONES, ZONE_GO_COUNT - 1);
        let first_client = client_asap(1, 0);
        assert_eq!(last_zone + 1, first_client);
    }

    #[test]
    fn recv_flags() {
        assert!(RECV.write);
        assert!(RECV.communicate);
        assert!(!RECV.read);
        assert!(!RECV.transmit);
    }

    #[test]
    fn send_flags() {
        assert!(SEND.read);
        assert!(SEND.transmit);
        assert!(SEND.communicate);
        assert!(!SEND.write);
    }

    #[test]
    fn descriptor_bits_recv() {
        // RECV: communicate(10) + write(12) = 0x1400 | size
        let bits = RECV.to_descriptor_bits(1);
        assert_eq!(bits, (1 << 10) | (1 << 12) | 1);
    }

    #[test]
    fn descriptor_bits_send() {
        // SEND: communicate(10) + read(11) + transmit(14) = 0x4C00 | size
        let bits = SEND.to_descriptor_bits(1);
        assert_eq!(bits, (1 << 10) | (1 << 11) | (1 << 14) | 1);
    }
}
