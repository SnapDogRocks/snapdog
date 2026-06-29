// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! Snapcast seam contract (IT-T52, partial): compile-time exhaustiveness guards
//! over `SnapcastEvent` and `SnapcastCmd`. With no wildcard arm, a variant added
//! or renamed by an upgraded `snapcast-server` / refactor breaks the build —
//! exactly the silent drift the firewall (RFC §9.1) exists to catch.
//!
//! Backend-agnostic (the `SnapcastBackend` trait + these enums are always
//! compiled). The embedded `ServerEvent`→`SnapcastEvent` mapper and JSON-RPC
//! golden vectors are tracked separately (IT-T51/T54, process feature).

use snapdog::player::SnapcastCmd;
use snapdog::snapcast::backend::SnapcastEvent;

const fn event_tag(e: &SnapcastEvent) -> &'static str {
    match e {
        SnapcastEvent::ClientConnected { .. } => "client_connected",
        SnapcastEvent::ClientDisconnected { .. } => "client_disconnected",
        SnapcastEvent::ClientVolumeChanged { .. } => "client_volume_changed",
        SnapcastEvent::ClientLatencyChanged { .. } => "client_latency_changed",
        SnapcastEvent::ClientNameChanged { .. } => "client_name_changed",
        SnapcastEvent::ServerUpdated => "server_updated",
        SnapcastEvent::CustomMessage { .. } => "custom_message",
    }
}

const fn cmd_tag(c: &SnapcastCmd) -> &'static str {
    match c {
        SnapcastCmd::Group { .. } => "group",
        SnapcastCmd::Client { .. } => "client",
        SnapcastCmd::ReconcileZones => "reconcile_zones",
    }
}

#[test]
fn snapcast_event_variants_are_exhaustively_mapped() {
    assert_eq!(
        event_tag(&SnapcastEvent::ClientDisconnected { id: "x".into() }),
        "client_disconnected"
    );
    assert_eq!(event_tag(&SnapcastEvent::ServerUpdated), "server_updated");
    assert_eq!(
        event_tag(&SnapcastEvent::CustomMessage {
            client_id: "c".into(),
            type_id: 1,
            payload: vec![]
        }),
        "custom_message"
    );
}

#[test]
fn snapcast_cmd_variants_are_exhaustively_mapped() {
    assert_eq!(cmd_tag(&SnapcastCmd::ReconcileZones), "reconcile_zones");
}
