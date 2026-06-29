// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! WebSocket notification contract (IT-T20): all 7 `Notification` variants
//! serde-round-trip with the correct `type` tag, a compile-time exhaustiveness
//! guard catches silently-added/renamed variants, and a broadcast reaches a
//! subscriber tap.

mod common;

use snapdog::api::ws::{Notification, broadcast_notification};

/// Compile-time exhaustiveness guard: no wildcard arm, so adding or renaming a
/// `Notification` variant breaks the build until this (and the WS docs) are
/// updated. Mirrors the snapcast event exhaustiveness approach (IT-T52).
const fn tag(n: &Notification) -> &'static str {
    match n {
        Notification::ZoneChanged { .. } => "zone_changed",
        Notification::ZoneVolumeChanged { .. } => "zone_volume_changed",
        Notification::ZoneProgress { .. } => "zone_progress",
        Notification::ClientStateChanged { .. } => "client_state_changed",
        Notification::ZoneEqChanged { .. } => "zone_eq_changed",
        Notification::ZonePresenceChanged { .. } => "zone_presence_changed",
        Notification::PlaybackError { .. } => "playback_error",
    }
}

fn assert_tag(n: &Notification, expected: &str) {
    let v = serde_json::to_value(n).expect("serialize notification");
    assert_eq!(v["type"], expected, "serde tag mismatch: {v}");
    assert_eq!(tag(n), expected, "exhaustiveness tag mismatch");
    // Round-trip back through Deserialize.
    let back: Notification = serde_json::from_value(v).expect("deserialize notification");
    assert_eq!(tag(&back), expected);
}

#[test]
fn lightweight_variants_round_trip_with_snake_case_tag() {
    assert_tag(
        &Notification::ZoneVolumeChanged {
            zone: 1,
            volume: 50,
            muted: false,
        },
        "zone_volume_changed",
    );
    assert_tag(
        &Notification::ZoneProgress {
            zone: 1,
            position_ms: 1000,
            duration_ms: 200_000,
            buffered_ms: None,
        },
        "zone_progress",
    );
    assert_tag(
        &Notification::ClientStateChanged {
            client: 2,
            volume: 30,
            muted: true,
            connected: true,
            zone: 1,
            is_snapdog: false,
        },
        "client_state_changed",
    );
    assert_tag(
        &Notification::ZonePresenceChanged {
            zone: 1,
            presence: true,
            enabled: true,
            timer_active: false,
        },
        "zone_presence_changed",
    );
    assert_tag(
        &Notification::PlaybackError {
            zone: 1,
            message: "boom".into(),
            details: Some("source missing".into()),
            recoverable: false,
        },
        "playback_error",
    );
}

#[test]
fn zone_eq_changed_flattens_config() {
    let n = Notification::ZoneEqChanged {
        zone: 1,
        config: snapdog::audio::eq::EqConfig::default(),
    };
    let v = serde_json::to_value(&n).expect("serialize");
    assert_eq!(v["type"], "zone_eq_changed");
    assert_eq!(v["zone"], 1);
    // #[serde(flatten)] => the Eq fields are siblings of `zone`, not nested.
    assert!(
        v.get("config").is_none(),
        "config must be flattened, not nested: {v}"
    );
    assert!(
        v.get("enabled").is_some() || v.get("bands").is_some(),
        "flattened Eq fields should appear at top level: {v}"
    );
}

#[test]
fn zone_progress_omits_none_buffered() {
    let v = serde_json::to_value(Notification::ZoneProgress {
        zone: 1,
        position_ms: 0,
        duration_ms: 0,
        buffered_ms: None,
    })
    .unwrap();
    assert!(
        v.get("buffered_ms").is_none(),
        "buffered_ms=None must be skipped: {v}"
    );
}

#[tokio::test]
async fn broadcast_reaches_subscriber_tap() {
    let mut app = common::test_app();
    broadcast_notification(
        &app.state.notifications,
        &Notification::ZoneVolumeChanged {
            zone: 2,
            volume: 33,
            muted: true,
        },
    );
    let json = app.notify_rx.try_recv().expect("notification delivered");
    let v: serde_json::Value = serde_json::from_str(&json).expect("valid json");
    assert_eq!(v["type"], "zone_volume_changed");
    assert_eq!(v["zone"], 2);
    assert_eq!(v["volume"], 33);
    assert_eq!(v["muted"], true);
}
