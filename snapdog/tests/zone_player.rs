// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! IT-T50 / IT-T55 / IT-T82 — deterministic zone-player tests (transitions,
//! presence auto-off, F32 send path).
//!
//! Drives REAL `spawn_zone_players` runner tasks (receivers disabled via
//! `start_receivers = false`, no-op [`MockBackend`]) and observes each transition
//! through the WebSocket notification broadcast (the sync barrier) plus the shared
//! store. No sockets, no mDNS, no timing assumptions — the notification *proves*
//! the async command was processed before the store is read.

mod common;

use common::{spawn_zone_harness, test_config};
use snapdog::player::ZoneCommand;
use snapdog::state::{PlaybackState, SourceType};
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

/// IT-T82 — presence auto-off timer fires under a paused clock and stops the zone.
///
/// The auto-off timer is retriggerable: each `SetPresence(true)` (re)arms it, and the
/// zone stops when it expires with no further trigger. We seed a presence-started source
/// currently Playing DIRECTLY in the store, rather than driving a real radio: the
/// presence-default path spawns an HTTP decode whose failure (connection-refused, on real
/// wall-clock under `start_paused`) could reset playback→Stopped before the arm, and emits
/// a `stopped` `zone_changed` that is indistinguishable from the auto-off one. Direct
/// seeding makes it fully deterministic. The fire barrier keys on
/// `zone_presence_changed{timer_active: false}` because only the auto-off fire broadcasts
/// it after the timer expires.
#[tokio::test(start_paused = true)]
async fn presence_auto_off_stops_zone_after_delay() {
    let mut h = spawn_zone_harness(test_config()).await;
    let zone = 1usize;

    {
        let mut s = h.store.write().await;
        let z = s.zones.get_mut(&zone).unwrap();
        z.presence_enabled = true; // default, made explicit
        z.presence_source = true; // playback was started by presence
        z.playback = PlaybackState::Playing;
        z.auto_off_delay = 10; // seconds; magnitude irrelevant under start_paused
        drop(s);
    }

    // A presence trigger (re)arms the retriggerable auto-off timer.
    h.senders[&zone]
        .send(ZoneCommand::SetPresence(true))
        .await
        .unwrap();
    // Linchpin barrier: emitted only AFTER `.reset()` + `auto_off_armed = true`, so the
    // pinned sleep is live and the task is parked back in `select!` before we move the clock.
    h.await_notification(|v| {
        v["type"] == "zone_presence_changed"
            && v["zone"] == zone
            && v["presence"] == true
            && v["timer_active"] == true
    })
    .await;

    // Advance virtual time past the delay with no further trigger → the timer fires.
    tokio::time::advance(std::time::Duration::from_secs(11)).await;

    // Fire barrier: race-free vs the decode-error `stopped` path (which never calls
    // notify_presence).
    h.await_notification(|v| {
        v["type"] == "zone_presence_changed" && v["zone"] == zone && v["timer_active"] == false
    })
    .await;

    let (playback, source, presence_source, auto_off_active, track_none) = {
        let s = h.store.read().await;
        (
            s.zones[&zone].playback.clone(),
            s.zones[&zone].source.clone(),
            s.zones[&zone].presence_source,
            s.zones[&zone].auto_off_active,
            s.zones[&zone].track.is_none(),
        )
    };
    assert_eq!(playback, PlaybackState::Stopped);
    assert_eq!(source, SourceType::Idle);
    assert!(!presence_source);
    assert!(!auto_off_active);
    assert!(track_none);
}

/// IT-T55 (contract) — compile-time signature guard for `SnapcastBackend::send_audio`.
/// The nested fn is type-checked (drift in arity/arg-types/return breaks the build)
/// but never executed. Runs in the normal suite (no feature). The embedded
/// `F32AudioSender` is guarded transitively: `embedded.rs` forwards into it, so an
/// upstream signature change fails the `snapcast-embedded` build.
#[test]
fn send_audio_signature_is_stable() {
    #[allow(dead_code)]
    fn contract<B: snapdog::snapcast::backend::SnapcastBackend>(b: &B) {
        let _fut: snapdog::snapcast::backend::BoxFuture<'_, anyhow::Result<()>> =
            b.send_audio(0usize, &[0.0f32][..], 0u32, 0u16);
    }
}

/// IT-T55 (behavioral) — a PCM frame injected into the real decode arm flows through
/// resample→EQ and reaches `SnapcastBackend::send_audio` with the right shape.
/// Gated on `test-harness` (the `test_pcm_rx` seam). `test_config()` audio rates
/// match → passthrough resampler + flat EQ preserve the sample count.
#[cfg(feature = "test-harness")]
#[tokio::test]
async fn injected_pcm_reaches_backend_send_audio() {
    let h = common::spawn_zone_harness_capturing(test_config()).await;

    // 4 stereo frames = 8 interleaved f32 samples.
    h.test_pcm_tx[&1]
        .send(snapdog::audio::PcmMessage::Audio(vec![0.25_f32; 8]))
        .await
        .unwrap();

    // The audio arm emits no notification — poll the backend under a bounded timeout.
    let call = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if let Some(c) = h.backend.calls().into_iter().next() {
                break c;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("send_audio was called");

    assert_eq!(call.zone_index, 1);
    assert_eq!(
        call.len, 8,
        "passthrough resampler + flat EQ preserve sample count"
    );
    assert_eq!(call.sample_rate, snapdog_common::DEFAULT_SAMPLE_RATE);
    assert_eq!(call.channels, 2);
}
