// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! IT-T82 (partial) / IT-T50 — deterministic zone-player command→state transitions.
//!
//! Drives REAL `spawn_zone_players` runner tasks (receivers disabled via
//! `start_receivers = false`, no-op [`MockBackend`]) and observes each transition
//! through the WebSocket notification broadcast (the sync barrier) plus the shared
//! store. No sockets, no mDNS, no timing assumptions — the notification *proves*
//! the async command was processed before the store is read.

mod common;

use common::{spawn_zone_harness, test_config};
use snapdog::player::ZoneCommand;
use snapdog_common::RepeatMode;

#[tokio::test]
async fn set_volume_transitions_store_and_notifies() {
    let mut h = spawn_zone_harness(test_config()).await;

    h.senders[&1]
        .send(ZoneCommand::SetVolume(75))
        .await
        .unwrap();

    let n = h
        .await_notification(|v| v["type"] == "zone_volume_changed" && v["zone"] == 1)
        .await;
    assert_eq!(n["volume"], 75);
    assert_eq!(h.store.read().await.zones[&1].volume, 75);
}

#[tokio::test]
async fn set_volume_clamps_above_max() {
    let mut h = spawn_zone_harness(test_config()).await;

    h.senders[&2]
        .send(ZoneCommand::SetVolume(150))
        .await
        .unwrap();

    let n = h
        .await_notification(|v| v["type"] == "zone_volume_changed" && v["zone"] == 2)
        .await;
    assert_eq!(n["volume"], 100, "volume is clamped to 100");
    assert_eq!(h.store.read().await.zones[&2].volume, 100);
}

#[tokio::test]
async fn adjust_volume_applies_delta() {
    let mut h = spawn_zone_harness(test_config()).await;

    // Set a known baseline, then nudge it down.
    h.senders[&1]
        .send(ZoneCommand::SetVolume(50))
        .await
        .unwrap();
    h.await_notification(|v| {
        v["type"] == "zone_volume_changed" && v["zone"] == 1 && v["volume"] == 50
    })
    .await;

    h.senders[&1]
        .send(ZoneCommand::AdjustVolume(-20))
        .await
        .unwrap();
    let n = h
        .await_notification(|v| {
            v["type"] == "zone_volume_changed" && v["zone"] == 1 && v["volume"] == 30
        })
        .await;
    assert_eq!(n["volume"], 30);
    assert_eq!(h.store.read().await.zones[&1].volume, 30);
}

#[tokio::test]
async fn set_shuffle_and_repeat_transition_state() {
    let mut h = spawn_zone_harness(test_config()).await;

    h.senders[&1]
        .send(ZoneCommand::SetShuffle(true))
        .await
        .unwrap();
    let n = h
        .await_notification(|v| {
            v["type"] == "zone_changed" && v["zone"] == 1 && v["shuffle"] == true
        })
        .await;
    assert_eq!(n["shuffle"], true);
    assert!(h.store.read().await.zones[&1].shuffle);

    h.senders[&1]
        .send(ZoneCommand::SetRepeat(RepeatMode::Playlist))
        .await
        .unwrap();
    let n = h
        .await_notification(|v| {
            v["type"] == "zone_changed" && v["zone"] == 1 && v["repeat"] == "playlist"
        })
        .await;
    assert_eq!(n["repeat"], "playlist");
    assert_eq!(h.store.read().await.zones[&1].repeat, RepeatMode::Playlist);
}

#[tokio::test]
async fn transitions_are_isolated_per_zone() {
    let mut h = spawn_zone_harness(test_config()).await;

    // A change to zone 1 must not bleed into zone 2.
    h.senders[&1]
        .send(ZoneCommand::SetVolume(10))
        .await
        .unwrap();
    h.await_notification(|v| v["type"] == "zone_volume_changed" && v["zone"] == 1)
        .await;

    h.senders[&2]
        .send(ZoneCommand::SetVolume(90))
        .await
        .unwrap();
    h.await_notification(|v| v["type"] == "zone_volume_changed" && v["zone"] == 2)
        .await;

    let (z1, z2) = {
        let s = h.store.read().await;
        (s.zones[&1].volume, s.zones[&2].volume)
    };
    assert_eq!(z1, 10);
    assert_eq!(z2, 90);
}
