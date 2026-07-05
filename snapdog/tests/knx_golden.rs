// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! IT-T41 — KNX wire contract via the PUBLIC surface + the knx-rs-core dependency
//! contract. The GroupAddress round-trip and DPT byte encodings are exactly what a
//! knx-rs upgrade could silently break, so they're pinned here as golden bytes
//! (the in-source `group_objects` tests cover the 460-GO layout values; this also
//! confirms they're reachable through the exported module path).

#![allow(clippy::doc_markdown)] // doc mentions knx-rs / GroupAddress / DPT

use knx_rs_core::address::GroupAddress;
use knx_rs_core::dpt::{self, DPT_SCALING, DPT_SWITCH, DPT_VALUE_2_UCOUNT, Dpt, DptValue};
use snapdog::knx::group_objects::{
    CLIENT_GO_COUNT, HARDWARE_TYPE, MAX_CLIENTS, MAX_ZONES, TOTAL_GO_COUNT, ZONE_GO_COUNT,
    client_asap, zone_asap,
};
use std::str::FromStr;

#[test]
fn group_object_layout_is_exported_and_totals_460() {
    assert_eq!(
        TOTAL_GO_COUNT,
        MAX_ZONES * ZONE_GO_COUNT + MAX_CLIENTS * CLIENT_GO_COUNT
    );
    assert_eq!(TOTAL_GO_COUNT, 460);
    // Contiguous 1-based ASAP layout: zones occupy [1..=350], clients [351..=460].
    assert_eq!(zone_asap(1, 0), 1);
    assert_eq!(zone_asap(MAX_ZONES, ZONE_GO_COUNT - 1), 350);
    assert_eq!(client_asap(1, 0), 351);
    assert_eq!(client_asap(MAX_CLIENTS, CLIENT_GO_COUNT - 1), 460);
}

#[test]
fn group_address_round_trip() {
    let ga = GroupAddress::from_str("1/2/3").unwrap();
    assert_eq!(ga.raw(), 0x0A03, "3-level GA = (1<<11)|(2<<8)|3");
    assert_eq!(
        format!("{ga}"),
        "1/2/3",
        "Display is the form handle_incoming keys on"
    );
    assert_eq!(GroupAddress::from_raw(0x0A03).raw(), ga.raw());
}

#[test]
fn hardware_type_matches_knxprod_compare_gate() {
    // The device serves HARDWARE_TYPE as PID_HARDWARE_TYPE (device object, PID 78);
    // ETS compares it to the .knxprod's LdCtrlCompareProp at the start of every
    // download and aborts on mismatch. Firmware (device.rs) and the .knxprod
    // (xtask) both derive from this constant, so they can't drift — this pins the
    // value against an accidental change.
    use std::fmt::Write as _;
    let hex = HARDWARE_TYPE.iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02X}");
        s
    });
    assert_eq!(
        hex, "0000FF010100",
        "HardwareType must equal the .knxprod compare gate"
    );
}

#[test]
fn dpt_encode_byte_goldens() {
    // 1.001 switch
    assert_eq!(
        dpt::encode(DPT_SWITCH, &DptValue::Bool(true)).unwrap(),
        vec![0x01]
    );
    assert_eq!(
        dpt::encode(DPT_SWITCH, &DptValue::Bool(false)).unwrap(),
        vec![0x00]
    );
    // 5.001 scaling: round(pct * 255 / 100)
    assert_eq!(
        dpt::encode(DPT_SCALING, &DptValue::Float(0.0)).unwrap(),
        vec![0x00]
    );
    assert_eq!(
        dpt::encode(DPT_SCALING, &DptValue::Float(50.0)).unwrap(),
        vec![0x80] // round(127.5) = 128
    );
    assert_eq!(
        dpt::encode(DPT_SCALING, &DptValue::Float(60.0)).unwrap(),
        vec![0x99] // 153
    );
    assert_eq!(
        dpt::encode(DPT_SCALING, &DptValue::Float(100.0)).unwrap(),
        vec![0xFF]
    );
    // 7.x = 2-byte big-endian unsigned (7.001 decode + 7.005 publish share the wire form)
    let dpt_7_5 = Dpt::new(7, 5);
    assert_eq!(
        dpt::encode(DPT_VALUE_2_UCOUNT, &DptValue::UInt(60)).unwrap(),
        vec![0x00, 0x3C]
    );
    assert_eq!(
        dpt::encode(dpt_7_5, &DptValue::UInt(1000)).unwrap(),
        vec![0x03, 0xE8]
    );
    assert_eq!(
        dpt::encode(dpt_7_5, &DptValue::UInt(65535)).unwrap(),
        vec![0xFF, 0xFF]
    );
}
