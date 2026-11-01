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

/// Object numbers reserved per zone. A zone's objects sit in groups of ten (see
/// [`ZONE_GO_SLOTS`]), so the stride is larger than [`ZONE_GO_COUNT`] and leaves room to add objects
/// without moving any number.
pub const ZONE_GO_STRIDE: usize = 60;

/// Object numbers reserved per client (three groups of ten, see [`CLIENT_GO_SLOTS`]).
pub const CLIENT_GO_STRIDE: usize = 30;

/// Object numbers reserved for the device-level objects (two groups of ten, see
/// [`GLOBAL_GO_SLOTS`]).
pub const GLOBAL_GO_STRIDE: usize = 20;

/// Total number of group object numbers, defined or reserved (the size of the group object table).
pub const TOTAL_GO_COUNT: usize =
    MAX_ZONES * ZONE_GO_STRIDE + MAX_CLIENTS * CLIENT_GO_STRIDE + GLOBAL_GO_STRIDE;

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
pub const KNXPROD_APP_VERSION: u32 = 13;
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

/// DPT 1.007 — Step (0 = decrease, 1 = increase; here previous and next).
pub const DPT_STEP: Dpt = Dpt::new(1, 7);

/// DPT 1.008 — Up/Down (0 = up, 1 = down). For a blind-style rocker: down is next, up is previous,
/// like scrolling a list.
pub const DPT_UPDOWN: Dpt = Dpt::new(1, 8);

/// DPT 3.007 — Controlled dimming.
pub const DPT_CONTROL_DIMMING: Dpt = Dpt::new(3, 7);

/// DPT 1.018 — Occupancy (presence sensor).
const DPT_OCCUPANCY: Dpt = Dpt::new(1, 18);

/// DPT 7.005 — Time period in seconds (`UInt16`).
pub const DPT_TIME_PERIOD_SEC: Dpt = Dpt::new(7, 5);

/// DPT 1.011 — State (used for the cyclic "server online" heartbeat).
pub const DPT_STATE: Dpt = Dpt::new(1, 11);

/// DPT 1.017 — Trigger (the global "all stop" command, and next and previous track and playlist).
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

// ── Zone group objects (39 per zone, in six groups) ───────────

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

/// Zone group object definitions (41 per zone).
///
/// Listed in object-number order: the position here is the object's *index* (`ZGO_*`), its number
/// inside the zone comes from [`ZONE_GO_SLOTS`]. Six groups of ten numbers: transport (0), volume and
/// mute (1), play modes (2), playlist (3), track info (4), presence (5). Numbers left over in a group
/// are reserved.
pub const ZONE_GOS: &[GoDefinition] = &[
    // ── Group 0: Transport (numbers 1 to 10 of the zone) ──
    go_recv("Play", "Play", "Play", DPT_SWITCH, "DPST-1-1", "1 Bit"),
    go_recv("Pause", "Pause", "Pause", DPT_SWITCH, "DPST-1-1", "1 Bit"),
    go_recv("Stop", "Stop", "Stop", DPT_SWITCH, "DPST-1-1", "1 Bit"),
    go_recv(
        "Play/Pause",
        "Wiedergabe/Pause",
        "Play/Pause",
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
        "Track Next",
        "Nächster Titel",
        "Next Track",
        DPT_TRIGGER,
        "DPST-1-17",
        "1 Bit",
    ),
    go_recv(
        "Track Previous",
        "Vorheriger Titel",
        "Previous Track",
        DPT_TRIGGER,
        "DPST-1-17",
        "1 Bit",
    ),
    go_recv(
        "Track Step",
        "Titel Schritt",
        "Track Step",
        DPT_STEP,
        "DPST-1-7",
        "1 Bit",
    ),
    go_recv(
        "Track Step Up/Down",
        "Titel Schritt Auf/Ab",
        "Track Step Up/Down",
        DPT_UPDOWN,
        "DPST-1-8",
        "1 Bit",
    ),
    // ── Group 1: Volume and mute (numbers 11 to 20 of the zone) ──
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
    // ── Group 2: Play modes (numbers 21 to 30 of the zone) ──
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
    // ── Group 3: Playlist (numbers 31 to 40 of the zone) ──
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
        DPT_TRIGGER,
        "DPST-1-17",
        "1 Bit",
    ),
    go_recv(
        "Playlist Previous",
        "Vorherige Playlist",
        "Previous Playlist",
        DPT_TRIGGER,
        "DPST-1-17",
        "1 Bit",
    ),
    go_recv(
        "Playlist Step",
        "Playlist Schritt",
        "Playlist Step",
        DPT_STEP,
        "DPST-1-7",
        "1 Bit",
    ),
    go_send(
        "Playlist Name",
        "Playlist Name",
        "Playlist Name",
        DPT_STRING_8859_1,
        "DPST-16-1",
        "14 Bytes",
    ),
    go_recv(
        "Playlist Step Up/Down",
        "Playlist Schritt Auf/Ab",
        "Playlist Step Up/Down",
        DPT_UPDOWN,
        "DPST-1-8",
        "1 Bit",
    ),
    // ── Group 4: Track info (numbers 41 to 50 of the zone) ──
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
    // Jump to a track of the active playlist by its number (1 is the first track).
    go_recv(
        "Track Number",
        "Titelnummer",
        "Track Number",
        DPT_VALUE_1_UCOUNT,
        "DPST-5-10",
        "1 Byte",
    ),
    go_send(
        "Track Number Status",
        "Titelnummer Status",
        "Track Number Status",
        DPT_VALUE_1_UCOUNT,
        "DPST-5-10",
        "1 Byte",
    ),
    // ── Group 5: Presence (numbers 51 to 60 of the zone) ──
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
    // ── Group 0: System status (numbers 1501 to 1510) ──
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
    // ── Group 1: All zones (numbers 1511 to 1520) ──
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
];

/// Number of global group objects.
pub const GLOBAL_GO_COUNT: usize = GLOBAL_GOS.len();

// ── Named GO indices (global) ─────────────────────────────────

/// Global GO index — cyclic "server online" heartbeat (send).
pub const GGO_SERVER_ONLINE: usize = 0;
/// Global GO index — "system fault" alarm (send).
pub const GGO_SYSTEM_FAULT: usize = 1;
/// Global GO index — KNX time-of-day clock input (receive).
pub const GGO_KNX_TIME: usize = 2;
/// Global GO index — "all stop" trigger (receive).
pub const GGO_ALL_STOP: usize = 3;
/// Global GO index — "all mute" switch (bidirectional).
pub const GGO_ALL_MUTE: usize = 4;

/// Compute the 1-based ASAP for a global group object (0-based `go_index` within
/// [`GLOBAL_GOS`]). Globals follow every zone and client GO.
#[must_use]
pub const fn global_asap(go_index: usize) -> u16 {
    (MAX_ZONES * ZONE_GO_STRIDE + MAX_CLIENTS * CLIENT_GO_STRIDE + GLOBAL_GO_SLOTS[go_index] + 1)
        as u16
}

/// Compute the 1-based ASAP for a zone group object.
///
/// Zone `zone_index` (1-based), GO `go_index` (0-based within `ZONE_GOS`, the `ZGO_*` constants).
#[must_use]
pub const fn zone_asap(zone_index: usize, go_index: usize) -> u16 {
    ((zone_index - 1) * ZONE_GO_STRIDE + ZONE_GO_SLOTS[go_index] + 1) as u16
}

// ── Named GO indices (zone) ───────────────────────────────────
// Use these instead of magic numbers when mapping GAs to GOs. The index is the position in
// `ZONE_GOS`, not the object number (see `zone_asap`).

/// Zone GO index.
pub const ZGO_PLAY: usize = 0;
/// Zone GO index.
pub const ZGO_PAUSE: usize = 1;
/// Zone GO index.
pub const ZGO_STOP: usize = 2;
/// Zone GO index.
pub const ZGO_PLAY_PAUSE: usize = 3;
/// Zone GO index.
pub const ZGO_CONTROL_STATUS: usize = 4;
/// Zone GO index.
pub const ZGO_TRACK_PLAYING: usize = 5;
/// Zone GO index.
pub const ZGO_TRACK_NEXT: usize = 6;
/// Zone GO index.
pub const ZGO_TRACK_PREVIOUS: usize = 7;
/// Zone GO index.
pub const ZGO_TRACK_STEP: usize = 8;
/// Zone GO index.
pub const ZGO_TRACK_STEP_UPDOWN: usize = 9;
/// Zone GO index.
pub const ZGO_VOLUME: usize = 10;
/// Zone GO index.
pub const ZGO_VOLUME_STATUS: usize = 11;
/// Zone GO index.
pub const ZGO_VOLUME_DIM: usize = 12;
/// Zone GO index.
pub const ZGO_MUTE: usize = 13;
/// Zone GO index.
pub const ZGO_MUTE_STATUS: usize = 14;
/// Zone GO index.
pub const ZGO_MUTE_TOGGLE: usize = 15;
/// Zone GO index.
pub const ZGO_SHUFFLE: usize = 16;
/// Zone GO index.
pub const ZGO_SHUFFLE_STATUS: usize = 17;
/// Zone GO index.
pub const ZGO_SHUFFLE_TOGGLE: usize = 18;
/// Zone GO index.
pub const ZGO_REPEAT: usize = 19;
/// Zone GO index.
pub const ZGO_REPEAT_STATUS: usize = 20;
/// Zone GO index.
pub const ZGO_REPEAT_TOGGLE: usize = 21;
/// Zone GO index.
pub const ZGO_TRACK_REPEAT: usize = 22;
/// Zone GO index.
pub const ZGO_TRACK_REPEAT_STATUS: usize = 23;
/// Zone GO index.
pub const ZGO_TRACK_REPEAT_TOGGLE: usize = 24;
/// Zone GO index.
pub const ZGO_PLAYLIST: usize = 25;
/// Zone GO index.
pub const ZGO_PLAYLIST_STATUS: usize = 26;
/// Zone GO index.
pub const ZGO_PLAYLIST_NEXT: usize = 27;
/// Zone GO index.
pub const ZGO_PLAYLIST_PREVIOUS: usize = 28;
/// Zone GO index.
pub const ZGO_PLAYLIST_STEP: usize = 29;
/// Zone GO index.
pub const ZGO_PLAYLIST_NAME: usize = 30;
/// Zone GO index.
pub const ZGO_PLAYLIST_STEP_UPDOWN: usize = 31;
/// Zone GO index.
pub const ZGO_TRACK_TITLE: usize = 32;
/// Zone GO index.
pub const ZGO_TRACK_ARTIST: usize = 33;
/// Zone GO index.
pub const ZGO_TRACK_ALBUM: usize = 34;
/// Zone GO index.
pub const ZGO_TRACK_PROGRESS: usize = 35;
/// Zone GO index.
pub const ZGO_TRACK_NUMBER: usize = 36;
/// Zone GO index.
pub const ZGO_TRACK_NUMBER_STATUS: usize = 37;
/// Zone GO index.
pub const ZGO_PRESENCE: usize = 38;
/// Zone GO index.
pub const ZGO_PRESENCE_ENABLE: usize = 39;
/// Zone GO index.
pub const ZGO_PRESENCE_TIMER_ACTIVE: usize = 40;

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

// ── Object numbers: slots inside a zone or client block ───────

/// Slot of group `group` (0-based) at position `position` (0 to 9): groups are ten numbers wide.
const fn slot(group: usize, position: usize) -> usize {
    group * 10 + position
}

/// Slot (offset inside the zone's block of [`ZONE_GO_STRIDE`] numbers) of each zone object, in the
/// order of [`ZONE_GOS`]. Numbers that no slot names are reserved.
pub const ZONE_GO_SLOTS: [usize; ZONE_GO_COUNT] = [
    slot(0, 0),
    slot(0, 1),
    slot(0, 2),
    slot(0, 3),
    slot(0, 4),
    slot(0, 5),
    slot(0, 6),
    slot(0, 7),
    slot(0, 8),
    slot(0, 9),
    slot(1, 0),
    slot(1, 1),
    slot(1, 2),
    slot(1, 3),
    slot(1, 4),
    slot(1, 5),
    slot(2, 0),
    slot(2, 1),
    slot(2, 2),
    slot(2, 3),
    slot(2, 4),
    slot(2, 5),
    slot(2, 6),
    slot(2, 7),
    slot(2, 8),
    slot(3, 0),
    slot(3, 1),
    slot(3, 2),
    slot(3, 3),
    slot(3, 4),
    slot(3, 5),
    slot(3, 6),
    slot(4, 0),
    slot(4, 1),
    slot(4, 2),
    slot(4, 3),
    slot(4, 4),
    slot(4, 5),
    slot(5, 0),
    slot(5, 1),
    slot(5, 2),
];

/// Slot of each client object, in the order of [`CLIENT_GOS`]: volume and mute (group 0), latency and
/// zone (group 1), connection (group 2).
pub const CLIENT_GO_SLOTS: [usize; CLIENT_GO_COUNT] = [
    slot(0, 0),
    slot(0, 1),
    slot(0, 2),
    slot(0, 3),
    slot(0, 4),
    slot(0, 5),
    slot(1, 0),
    slot(1, 1),
    slot(1, 2),
    slot(1, 3),
    slot(2, 0),
];

/// Slot of each device-level object, in the order of [`GLOBAL_GOS`]: system status (group 0),
/// all zones (group 1).
pub const GLOBAL_GO_SLOTS: [usize; GLOBAL_GO_COUNT] =
    [slot(0, 0), slot(0, 1), slot(0, 2), slot(1, 0), slot(1, 1)];

/// Whether `slots` is strictly increasing and every slot is below `stride`.
const fn slots_valid(slots: &[usize], stride: usize) -> bool {
    let mut i = 0;
    while i < slots.len() {
        if slots[i] >= stride || (i > 0 && slots[i] <= slots[i - 1]) {
            return false;
        }
        i += 1;
    }
    true
}

const _: () = assert!(slots_valid(&ZONE_GO_SLOTS, ZONE_GO_STRIDE));
const _: () = assert!(slots_valid(&CLIENT_GO_SLOTS, CLIENT_GO_STRIDE));
const _: () = assert!(slots_valid(&GLOBAL_GO_SLOTS, GLOBAL_GO_STRIDE));

/// Where an object number points.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GoLocation {
    /// Zone `zone` (1-based), object index `go` in [`ZONE_GOS`].
    Zone {
        /// Zone number, 1-based.
        zone: usize,
        /// Index in [`ZONE_GOS`] (the `ZGO_*` constants).
        go: usize,
    },
    /// Client `client` (1-based), object index `go` in [`CLIENT_GOS`].
    Client {
        /// Client number, 1-based.
        client: usize,
        /// Index in [`CLIENT_GOS`] (the `CGO_*` constants).
        go: usize,
    },
    /// Device-level object, index in [`GLOBAL_GOS`].
    Global(usize),
}

/// What the object number `asap` is, `None` for a reserved number or one outside the table.
#[must_use]
pub fn locate(asap: u16) -> Option<GoLocation> {
    let n = usize::from(asap).checked_sub(1)?;
    let zones_end = MAX_ZONES * ZONE_GO_STRIDE;
    let clients_end = zones_end + MAX_CLIENTS * CLIENT_GO_STRIDE;
    if n < zones_end {
        let go = ZONE_GO_SLOTS
            .iter()
            .position(|s| *s == n % ZONE_GO_STRIDE)?;
        Some(GoLocation::Zone {
            zone: n / ZONE_GO_STRIDE + 1,
            go,
        })
    } else if n < clients_end {
        let n = n - zones_end;
        let go = CLIENT_GO_SLOTS
            .iter()
            .position(|s| *s == n % CLIENT_GO_STRIDE)?;
        Some(GoLocation::Client {
            client: n / CLIENT_GO_STRIDE + 1,
            go,
        })
    } else {
        let go = GLOBAL_GO_SLOTS.iter().position(|s| *s == n - clients_end)?;
        Some(GoLocation::Global(go))
    }
}

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
    (MAX_ZONES * ZONE_GO_STRIDE
        + (client_index - 1) * CLIENT_GO_STRIDE
        + CLIENT_GO_SLOTS[go_index]
        + 1) as u16
}

/// The definition of the object with this ASAP number, `None` for a reserved number or one outside
/// `1..=TOTAL_GO_COUNT`.
#[must_use]
pub fn definition_of(asap: u16) -> Option<&'static GoDefinition> {
    match locate(asap)? {
        GoLocation::Zone { go, .. } => ZONE_GOS.get(go),
        GoLocation::Client { go, .. } => CLIENT_GOS.get(go),
        GoLocation::Global(go) => GLOBAL_GOS.get(go),
    }
}

/// The descriptor size code for a value of `len` bytes (the inverse of the stack's
/// `value_size_bytes`; codes 0 to 6 are the sub-byte types, which this product does not use).
const fn size_code(len: usize) -> u8 {
    match len {
        0 | 1 => 7,
        5 => 15,
        6 => 11,
        7 => 16,
        8 => 12,
        9 => 17,
        10 => 13,
        11 => 18,
        12 => 19,
        13 => 20,
        14 => 14,
        n => (n + 6) as u8,
    }
}

/// The group object table of this product, one descriptor per object in ASAP order.
///
/// Flags and sizes are the ones the `.knxprod` declares. The table is compiled in (ETS never
/// downloads it), and the device stack needs it to enforce the flags: without it every object
/// accepts writes and responses and answers reads.
#[must_use]
pub fn descriptor_table() -> Vec<u8> {
    let mut table = Vec::with_capacity(2 + TOTAL_GO_COUNT * 2);
    table.extend_from_slice(&(TOTAL_GO_COUNT as u16).to_be_bytes());
    for asap in 1..=TOTAL_GO_COUNT as u16 {
        let bits = definition_of(asap).map_or(0, |d| {
            d.flags
                .to_descriptor_bits(size_code(usize::from(d.dpt.data_length())))
        });
        table.extend_from_slice(&bits.to_be_bytes());
    }
    table
}

#[cfg(test)]
#[allow(clippy::assertions_on_constants)]
mod tests {
    use super::*;

    #[test]
    fn zone_go_count() {
        assert_eq!(ZONE_GOS.len(), 41);
        assert_eq!(ZONE_GO_SLOTS.len(), 41);
    }

    #[test]
    fn client_go_count() {
        assert_eq!(CLIENT_GOS.len(), 11);
        assert_eq!(CLIENT_GO_SLOTS.len(), 11);
    }

    #[test]
    fn total_go_count() {
        // 10 zones x 60 + 30 clients x 30 + 20 global; defined or reserved.
        assert_eq!(TOTAL_GO_COUNT, 10 * 60 + 30 * 30 + 20);
        assert_eq!(TOTAL_GO_COUNT, 1520);
    }

    #[test]
    fn zone_asap_layout() {
        // Zone 1, Play: group 0, position 0 -> number 1.
        assert_eq!(zone_asap(1, ZGO_PLAY), 1);
        // Group 0 is full: ten objects, positions 0 to 9.
        assert_eq!(zone_asap(1, ZGO_TRACK_STEP), 9);
        assert_eq!(zone_asap(1, ZGO_TRACK_STEP_UPDOWN), 10);
        // Group 1 starts at the tenth number after the group before it: Volume is number 11.
        assert_eq!(zone_asap(1, ZGO_VOLUME), 11);
        assert_eq!(zone_asap(1, ZGO_MUTE_TOGGLE), 16);
        // Group 3 (playlist) starts at 31, group 4 (track info) at 41, group 5 (presence) at 51.
        assert_eq!(zone_asap(1, ZGO_PLAYLIST), 31);
        assert_eq!(zone_asap(1, ZGO_TRACK_TITLE), 41);
        assert_eq!(zone_asap(1, ZGO_PRESENCE), 51);
        assert_eq!(zone_asap(1, ZGO_PRESENCE_TIMER_ACTIVE), 53);
        // Zone 2 starts one stride later, zone 10 last object is 9 x 60 + 53.
        assert_eq!(zone_asap(2, ZGO_PLAY), 61);
        assert_eq!(zone_asap(10, ZGO_PRESENCE_TIMER_ACTIVE), 593);
    }

    #[test]
    fn client_asap_layout() {
        // Client 1 starts after the 600 zone numbers.
        assert_eq!(client_asap(1, CGO_VOLUME), 601);
        assert_eq!(client_asap(1, CGO_MUTE_TOGGLE), 606);
        // Group 1 (latency and zone) starts ten numbers after group 0.
        assert_eq!(client_asap(1, CGO_LATENCY), 611);
        assert_eq!(client_asap(1, CGO_ZONE_STATUS), 614);
        // Group 2 (connection).
        assert_eq!(client_asap(1, CGO_CONNECTED), 621);
        assert_eq!(client_asap(2, CGO_VOLUME), 631);
        assert_eq!(client_asap(MAX_CLIENTS, CGO_CONNECTED), 1491);
    }

    #[test]
    fn global_asap_layout() {
        // Globals follow all zone and client numbers: 600 + 900 = 1500. System status first,
        // then the objects for all zones, each group starting at a multiple of ten plus one.
        assert_eq!(global_asap(GGO_SERVER_ONLINE), 1501);
        assert_eq!(global_asap(GGO_SYSTEM_FAULT), 1502);
        assert_eq!(global_asap(GGO_KNX_TIME), 1503);
        assert_eq!(global_asap(GGO_ALL_STOP), 1511);
        assert_eq!(global_asap(GGO_ALL_MUTE), 1512);
    }

    #[test]
    fn blocks_do_not_overlap() {
        let last_zone = zone_asap(MAX_ZONES, ZONE_GO_COUNT - 1);
        let first_client = client_asap(1, 0);
        assert!(last_zone < first_client);
        let last_client = client_asap(MAX_CLIENTS, CLIENT_GO_COUNT - 1);
        assert!(last_client < global_asap(0));
    }

    #[test]
    fn slots_follow_the_groups_of_ten() {
        // Each group is ten numbers wide, so the tens digit of a slot is its group.
        let group = |slot: usize| slot / 10;
        for (name, go) in [
            ("Play", ZGO_PLAY),
            ("Play/Pause", ZGO_PLAY_PAUSE),
            ("Track Step", ZGO_TRACK_STEP),
        ] {
            assert_eq!(group(ZONE_GO_SLOTS[go]), 0, "{name} is a transport object");
        }
        for go in [
            ZGO_VOLUME,
            ZGO_VOLUME_STATUS,
            ZGO_VOLUME_DIM,
            ZGO_MUTE,
            ZGO_MUTE_STATUS,
            ZGO_MUTE_TOGGLE,
        ] {
            assert_eq!(group(ZONE_GO_SLOTS[go]), 1);
        }
        for go in [ZGO_SHUFFLE, ZGO_REPEAT, ZGO_TRACK_REPEAT_TOGGLE] {
            assert_eq!(group(ZONE_GO_SLOTS[go]), 2);
        }
        for go in [ZGO_PLAYLIST, ZGO_PLAYLIST_STEP, ZGO_PLAYLIST_NAME] {
            assert_eq!(group(ZONE_GO_SLOTS[go]), 3);
        }
        for go in [
            ZGO_TRACK_TITLE,
            ZGO_TRACK_ARTIST,
            ZGO_TRACK_ALBUM,
            ZGO_TRACK_PROGRESS,
        ] {
            assert_eq!(group(ZONE_GO_SLOTS[go]), 4);
        }
        for go in [ZGO_PRESENCE, ZGO_PRESENCE_ENABLE, ZGO_PRESENCE_TIMER_ACTIVE] {
            assert_eq!(group(ZONE_GO_SLOTS[go]), 5);
        }
        for go in [CGO_VOLUME, CGO_MUTE_TOGGLE] {
            assert_eq!(group(CLIENT_GO_SLOTS[go]), 0);
        }
        for go in [CGO_LATENCY, CGO_ZONE_STATUS] {
            assert_eq!(group(CLIENT_GO_SLOTS[go]), 1);
        }
        assert_eq!(group(CLIENT_GO_SLOTS[CGO_CONNECTED]), 2);
    }

    #[test]
    fn a_status_object_follows_its_command() {
        for (command, status) in [
            (ZGO_VOLUME, ZGO_VOLUME_STATUS),
            (ZGO_MUTE, ZGO_MUTE_STATUS),
            (ZGO_SHUFFLE, ZGO_SHUFFLE_STATUS),
            (ZGO_REPEAT, ZGO_REPEAT_STATUS),
            (ZGO_TRACK_REPEAT, ZGO_TRACK_REPEAT_STATUS),
            (ZGO_PLAYLIST, ZGO_PLAYLIST_STATUS),
        ] {
            assert_eq!(ZONE_GO_SLOTS[status], ZONE_GO_SLOTS[command] + 1);
        }
    }

    #[test]
    fn every_defined_object_is_found_by_its_number() {
        for zone in 1..=MAX_ZONES {
            for go in 0..ZONE_GO_COUNT {
                assert_eq!(
                    locate(zone_asap(zone, go)),
                    Some(GoLocation::Zone { zone, go })
                );
            }
        }
        for client in 1..=MAX_CLIENTS {
            for go in 0..CLIENT_GO_COUNT {
                assert_eq!(
                    locate(client_asap(client, go)),
                    Some(GoLocation::Client { client, go })
                );
            }
        }
        for go in 0..GLOBAL_GO_COUNT {
            assert_eq!(locate(global_asap(go)), Some(GoLocation::Global(go)));
        }
    }

    #[test]
    fn reserved_numbers_have_no_object() {
        // Number 10 is the rocker step of zone 1's transport group, which is full. 17 follows the
        // mute group, 30 ends the play modes, 38 follows the playlist group's last object.
        assert!(locate(10).is_some());
        assert_eq!(locate(17), None);
        assert_eq!(locate(30), None);
        assert_eq!(locate(38), None);
        // 54 to 60 are reserved at the end of every zone.
        assert_eq!(locate(54), None);
        assert_eq!(locate(60), None);
        assert_eq!(
            locate(zone_asap(MAX_ZONES, ZGO_PRESENCE_TIMER_ACTIVE) + 1),
            None
        );
        // Client 1: the tail of group 0 and group 2.
        assert_eq!(locate(607), None);
        assert_eq!(locate(622), None);
        // Outside the table.
        assert_eq!(locate(0), None);
        assert_eq!(locate(TOTAL_GO_COUNT as u16 + 1), None);
    }

    #[test]
    fn object_numbers_are_unique() {
        let mut seen = std::collections::HashSet::new();
        for zone in 1..=MAX_ZONES {
            for go in 0..ZONE_GO_COUNT {
                assert!(seen.insert(zone_asap(zone, go)));
            }
        }
        for client in 1..=MAX_CLIENTS {
            for go in 0..CLIENT_GO_COUNT {
                assert!(seen.insert(client_asap(client, go)));
            }
        }
        for go in 0..GLOBAL_GO_COUNT {
            assert!(seen.insert(global_asap(go)));
        }
        assert_eq!(
            seen.len(),
            MAX_ZONES * ZONE_GO_COUNT + MAX_CLIENTS * CLIENT_GO_COUNT + GLOBAL_GO_COUNT
        );
    }

    #[test]
    fn index_constants_name_their_definition() {
        for (go, name) in [
            (ZGO_PLAY, "Play"),
            (ZGO_PLAY_PAUSE, "Play/Pause"),
            (ZGO_TRACK_NEXT, "Track Next"),
            (ZGO_TRACK_STEP, "Track Step"),
            (ZGO_TRACK_STEP_UPDOWN, "Track Step Up/Down"),
            (ZGO_VOLUME, "Volume"),
            (ZGO_SHUFFLE_TOGGLE, "Shuffle Toggle"),
            (ZGO_PLAYLIST_STEP, "Playlist Step"),
            (ZGO_PLAYLIST_NAME, "Playlist Name"),
            (ZGO_PLAYLIST_STEP_UPDOWN, "Playlist Step Up/Down"),
            (ZGO_TRACK_PROGRESS, "Track Progress"),
            (ZGO_PRESENCE_TIMER_ACTIVE, "Presence Timer Active"),
        ] {
            assert_eq!(ZONE_GOS[go].name, name);
        }
        assert_eq!(CLIENT_GOS[CGO_ZONE].name, "Zone");
        assert_eq!(CLIENT_GOS[CGO_CONNECTED].name, "Connected");
    }

    #[test]
    fn the_new_objects_have_the_documented_dpt_and_flags() {
        let step = &ZONE_GOS[ZGO_TRACK_STEP];
        assert_eq!(step.dpt_str, "DPST-1-7");
        assert!(step.flags.write && !step.flags.read);
        assert_eq!(ZONE_GOS[ZGO_PLAYLIST_STEP].dpt_str, "DPST-1-7");
        // The rocker variants: DPT 1.008, same bit meaning as the Step objects.
        for go in [ZGO_TRACK_STEP_UPDOWN, ZGO_PLAYLIST_STEP_UPDOWN] {
            let updown = &ZONE_GOS[go];
            assert_eq!(updown.dpt_str, "DPST-1-8");
            assert!(updown.flags.write && !updown.flags.read);
        }
        // Next and previous are triggers (1.017); they react to any value.
        for go in [
            ZGO_TRACK_NEXT,
            ZGO_TRACK_PREVIOUS,
            ZGO_PLAYLIST_NEXT,
            ZGO_PLAYLIST_PREVIOUS,
        ] {
            assert_eq!(ZONE_GOS[go].dpt_str, "DPST-1-17");
        }
        let play_pause = &ZONE_GOS[ZGO_PLAY_PAUSE];
        assert_eq!(play_pause.dpt_str, "DPST-1-1");
        assert!(play_pause.flags.write);
        let name = &ZONE_GOS[ZGO_PLAYLIST_NAME];
        assert_eq!(name.dpt_str, "DPST-16-1");
        assert_eq!(name.size_str, "14 Bytes");
        assert!(name.flags.read && name.flags.transmit && !name.flags.write);
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
    fn definition_of_finds_the_object_by_asap() {
        assert_eq!(definition_of(0).map(|d| d.name), None);
        assert_eq!(definition_of(1).map(|d| d.name), Some("Play"));
        assert_eq!(
            definition_of(10).map(|d| d.name),
            Some("Track Step Up/Down")
        );
        assert_eq!(definition_of(17).map(|d| d.name), None, "reserved");
        assert_eq!(definition_of(11).map(|d| d.name), Some("Volume"));
        assert_eq!(
            definition_of(zone_asap(MAX_ZONES, ZGO_VOLUME_STATUS)).map(|d| d.name),
            Some("Volume Status")
        );
        assert_eq!(
            definition_of(client_asap(1, CGO_VOLUME)).map(|d| d.name),
            Some("Volume")
        );
        assert_eq!(
            definition_of(global_asap(GGO_KNX_TIME)).map(|d| d.name),
            Some("KNX Time")
        );
        // Reserved numbers and numbers past the table have no object.
        assert_eq!(definition_of(1504).map(|d| d.name), None);
        assert_eq!(definition_of(1513).map(|d| d.name), None);
        assert_eq!(definition_of(1521).map(|d| d.name), None);
    }

    #[test]
    fn descriptor_table_declares_flags_and_sizes_of_every_object() {
        let t = descriptor_table();
        assert_eq!(t.len(), 2 + TOTAL_GO_COUNT * 2);
        assert_eq!(
            usize::from(u16::from_be_bytes([t[0], t[1]])),
            TOTAL_GO_COUNT
        );
        let desc = |asap: u16| {
            let at = usize::from(asap) * 2;
            u16::from_be_bytes([t[at], t[at + 1]])
        };
        // Play: communicate + write, one byte. Volume status: communicate + read + transmit.
        assert_eq!(desc(zone_asap(1, ZGO_PLAY)), RECV.to_descriptor_bits(7));
        assert_eq!(
            desc(zone_asap(1, ZGO_VOLUME_STATUS)),
            SEND.to_descriptor_bits(7)
        );
        // The 14-byte strings and the 3-byte time keep their sizes.
        assert_eq!(desc(zone_asap(1, ZGO_TRACK_TITLE)) & 0xFF, 14);
        assert_eq!(desc(global_asap(GGO_KNX_TIME)) & 0xFF, 9);
        // Every defined object communicates, a reserved number has an empty descriptor.
        for asap in 1..=TOTAL_GO_COUNT as u16 {
            if definition_of(asap).is_some() {
                assert_ne!(desc(asap) & (1 << 10), 0, "communication enabled on {asap}");
            } else {
                assert_eq!(desc(asap), 0, "reserved number {asap} has no descriptor");
            }
        }
    }

    #[test]
    fn size_codes_follow_the_descriptor_format() {
        // value_size_bytes: 7 -> 1, 8..=10 -> 2..=4, 11 -> 6, 14 -> 14, 15 -> 5.
        assert_eq!(size_code(1), 7);
        assert_eq!(size_code(2), 8);
        assert_eq!(size_code(3), 9);
        assert_eq!(size_code(4), 10);
        assert_eq!(size_code(5), 15);
        assert_eq!(size_code(6), 11);
        assert_eq!(size_code(14), 14);
        assert_eq!(size_code(15), 21);
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
