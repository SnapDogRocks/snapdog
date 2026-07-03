// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! IT-T56 (tier-2) — real-`snapserver` end-to-end via the repaired `SnapserverHandle`:
//! the control path (`SnapcastClient` → `Server.GetStatus`/`GetRPCVersion`, plus
//! `sync_initial_state`/`reconcile_zone_groups` against a LIVE server) and the
//! per-zone TCP **audio source** path (snapserver's generated `tcp://…?mode=server`
//! source flips idle → playing when snapdog's `open_audio_source` producer pushes
//! PCM).
//!
//! This is the headline upgrade firewall: if `snapcast-proto`'s `ServerStatus`
//! shape or the snapserver.conf contract drifts on the 0.17 migration (IT-NG-05),
//! `server_get_status()` fails to deserialize and these tests go red.
//!
//! Tier-2: spawns a real `snapserver` child. **LOUD-SKIPs** (no panic) when the
//! binary is absent — matching the `mqtt_tier2` Docker-skip idiom. Run with:
//!   `cargo test -p snapdog --no-default-features --features snapcast-process \
//!        --test snapserver_e2e -- --test-threads=1`
//! (CI's "Integration Tests" job apt-installs snapserver, so this runs there.)

#![cfg(feature = "snapcast-process")]
#![allow(clippy::doc_markdown)] // doc mentions SnapcastClient / SnapserverHandle / snapserver.conf

use std::net::{Ipv4Addr, SocketAddr};
use std::time::Duration;

use snapdog::config::{self, FileConfig};
use snapdog::process::SnapserverHandle;
use snapdog::snapcast::types::StreamStatus;
use snapdog::snapcast::{
    SnapcastClient, open_audio_source, reconcile_zone_groups, sync_initial_state,
};
use snapdog::{api, state};
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;

/// Sine amplitude for the audio-source feed: ~24% of `i16::MAX` (≈ −12 dBFS peak),
/// a wide margin above snapcast's silence detection so the source registers as
/// `playing`. All-zero PCM is treated as digital silence and stays `idle`; if a
/// future snapserver tightens silence tuning and the audio test goes idle, raise
/// this.
const SINE_AMPLITUDE_I16: f32 = 8_000.0;

/// An ephemeral free TCP port (bound to :0, then released). Good enough for tier-2
/// serial runs; `IT-T02` will formalise a collision-proof pool.
async fn free_port() -> u16 {
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    l.local_addr().unwrap().port()
}

/// A resolved 2-zone / 1-client `AppConfig` wired for a managed snapserver on
/// unique free ports (control + streaming + per-zone TCP audio source).
async fn managed_test_config() -> config::AppConfig {
    let raw: FileConfig = toml::from_str(
        r#"
[[zone]]
name = "Living Room"

[[zone]]
name = "Kitchen"

[[client]]
name = "Speaker"
mac = "02:42:ac:11:00:20"
zone = "Living Room"
"#,
    )
    .expect("test TOML parses");
    let mut config = config::load_raw(raw).expect("config resolves");

    // Spawn a real snapserver (not the dev-mode no-op) on collision-free ports.
    config.snapcast.managed = true;
    config.snapcast.jsonrpc_tcp_port = free_port().await;
    config.snapcast.streaming_port = free_port().await;
    for zone in &mut config.zones {
        zone.tcp_source_port = free_port().await;
    }
    config
}

/// Start a real snapserver, or `None` (loud-skip) if the binary isn't installed.
/// The handle kills the child on drop, so callers just hold it for the test's life.
async fn spawn_snapserver_or_skip() -> Option<(SnapserverHandle, config::AppConfig, SnapcastClient)>
{
    let config = managed_test_config().await;
    let handle = match SnapserverHandle::start(&config) {
        Ok(h) => h,
        Err(e) => {
            eprintln!(
                "SKIP IT-T56: snapserver unavailable ({e}) — install snapcast to run this tier-2 test"
            );
            return None;
        }
    };
    // `connect` retries 10×500ms for readiness. If the control port never comes up
    // — an older snapserver that doesn't honour `[tcp-control]`, or a slow/racy start
    // — loud-skip rather than hard-fail: the tier-2 "service unavailable" contract,
    // mirroring the Docker skip in mqtt_tier2.
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, config.snapcast.jsonrpc_tcp_port));
    match SnapcastClient::connect(addr).await {
        Ok(snap) => Some((handle, config, snap)),
        Err(e) => {
            eprintln!(
                "SKIP IT-T56: snapserver control port unreachable ({e}) — incompatible/old snapserver?"
            );
            None
        }
    }
}

/// Poll `Server.GetStatus` until `stream_id` reaches `want`, or `timeout` elapses.
async fn wait_for_stream_status(
    snap: &SnapcastClient,
    stream_id: &str,
    want: StreamStatus,
    timeout: Duration,
) -> bool {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if let Ok(status) = snap.server_get_status().await {
            if let Some(s) = status.server.streams.iter().find(|s| s.id == stream_id) {
                if s.status == want {
                    return true;
                }
            }
        }
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

/// Control path: connect to a live snapserver, round-trip two RPC methods, and run
/// the repaired sync/reconcile helpers against the real server status.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn snapserver_control_e2e() {
    let Some((_handle, config, snap)) = spawn_snapserver_or_skip().await else {
        return;
    };

    // A second RPC method round-trips through the real newline-JSON framing.
    snap.server_get_rpc_version()
        .await
        .expect("Server.GetRPCVersion");

    // The headline firewall: a REAL ServerStatus deserializes through snapcast-proto.
    let status = snap
        .server_get_status()
        .await
        .expect("Server.GetStatus deserializes (fails if snapcast-proto status drifts)");

    // The generated snapserver.conf created one tcp:// source per zone, all idle
    // (no producer connected yet).
    let expected: Vec<&str> = config
        .zones
        .iter()
        .map(|z| z.stream_name.as_str())
        .collect();
    let got: Vec<&str> = status
        .server
        .streams
        .iter()
        .map(|s| s.id.as_str())
        .collect();
    for name in &expected {
        assert!(
            got.contains(name),
            "configured stream {name} present on the live server; got {got:?}"
        );
    }
    for s in &status.server.streams {
        assert_eq!(
            s.status,
            StreamStatus::Idle,
            "stream {} idle before audio",
            s.id
        );
        assert_eq!(s.uri.scheme, "tcp", "zone {} is a tcp:// source", s.id);
        assert_eq!(
            s.uri.query.get("name").map(String::as_str),
            Some(s.id.as_str()),
            "zone source name query matches the stream id"
        );
    }

    // No client has connected, so no group should exist for any configured zone
    // stream. (Asserting against the configured stream ids rather than total
    // emptiness tolerates a snapserver build that pre-creates an unrelated default
    // group.)
    let zone_groups: Vec<&str> = status
        .server
        .groups
        .iter()
        .filter(|g| config.zones.iter().any(|z| z.stream_name == g.stream_id))
        .map(|g| g.id.as_str())
        .collect();
    assert!(
        zone_groups.is_empty(),
        "no zone group before a client connects; got {zone_groups:?}"
    );

    // The repaired helpers must run against the LIVE status without panicking and
    // leave the config-derived store consistent (no groups → no snapcast_id, no
    // Group.SetClients).
    let store = state::init(&config, None).expect("state init");
    sync_initial_state(&status, &config, &snap, &store).await;
    let (notify, _notify_rx) = api::ws::notification_channel();
    reconcile_zone_groups(&snap, &config, &store, &notify).await;
    {
        let s = store.read().await;
        assert_eq!(s.zones.len(), 2, "both zones survive sync + reconcile");
        assert!(
            s.clients.values().all(|c| c.snapcast_id.is_none()),
            "no snapcast_id assigned without a connected client"
        );
    }
}

/// Audio-source path: pushing PCM to a zone's TCP source flips the snapserver
/// stream idle → playing.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn snapserver_audio_source_e2e() {
    let Some((_handle, config, snap)) = spawn_snapserver_or_skip().await else {
        return;
    };

    let zone = &config.zones[0];
    let stream_id = zone.stream_name.clone();
    let port = zone.tcp_source_port;

    // Precondition: idle before any producer connects.
    let pre = snap.server_get_status().await.expect("status");
    let pre_stream = pre
        .server
        .streams
        .iter()
        .find(|s| s.id == stream_id)
        .expect("zone stream exists in the generated conf");
    assert_eq!(pre_stream.status, StreamStatus::Idle, "idle before audio");

    // Connect as the audio producer (snapdog's own connector) and push 48000:16:2
    // PCM faster than real time so snapserver's TCP source registers a live feed
    // (backpressure paces us to the consumer rate). The signal must be NON-silent:
    // snapcast's silence detection keeps a digitally-silent (all-zero) source idle,
    // so we emit a 440 Hz sine.
    let mut src = open_audio_source(port)
        .await
        .expect("open audio source to snapserver");
    let writer = tokio::spawn(async move {
        let dphase = std::f32::consts::TAU * 440.0 / 48_000.0;
        let mut phase = 0.0_f32;
        loop {
            let mut chunk = Vec::with_capacity(3840); // 960 frames × 2ch × 2 bytes = 20ms
            for _ in 0..960 {
                let v = (SINE_AMPLITUDE_I16 * phase.sin()) as i16;
                phase = (phase + dphase) % std::f32::consts::TAU;
                let le = v.to_le_bytes();
                chunk.extend_from_slice(&le); // L
                chunk.extend_from_slice(&le); // R
            }
            if src.write_all(&chunk).await.is_err() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    });

    let playing = wait_for_stream_status(
        &snap,
        &stream_id,
        StreamStatus::Playing,
        Duration::from_secs(15),
    )
    .await;
    writer.abort();

    assert!(
        playing,
        "snapserver should report {stream_id} 'playing' once PCM flows to its TCP source"
    );
}
