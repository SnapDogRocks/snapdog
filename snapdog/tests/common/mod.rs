// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! Shared deterministic test harness ("testkit") for snapdog integration tests.
//!
//! RFC IT-0003 Phase 0. Provides:
//! - [`test_config`] — a 2-zone / 2-client resolved `AppConfig` with no file I/O.
//! - [`TestApp`] / [`build_test_app`] — an in-process `AppState` whose command
//!   channels are captured so handlers can be asserted without a real backend.
//! - [`TestApp::request`] — drive the full axum `Router` via `tower::oneshot`
//!   (no TCP socket), returning `(StatusCode, serde_json::Value)`.
//!
//! Determinism (`IT-DEC-02`): no sockets, no mDNS, `persist_path = None` (no
//! auto-save loop), `EqStore` backed by a `TempDir`.

#![allow(dead_code)] // harness is consumed selectively across test files

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use snapdog::api::{self, AppState, SharedState};
use snapdog::audio::eq::EqStore;
use snapdog::config::{self, AppConfig};
use snapdog::player::{self, SnapcastCmd, ZoneCommand, ZonePlayerContext};
use snapdog::snapcast::backend::{BoxFuture, SnapcastBackend};
use snapdog::state;
use tokio::sync::{broadcast, mpsc};
use tower::ServiceExt; // for `oneshot`

/// Build a resolved 2-zone / 2-client `AppConfig` for tests (pure, no I/O).
///
/// Zone/client names mirror `snapdog.example.toml`; MACs are lowercase per the
/// snapdog convention.
#[must_use]
pub fn test_config() -> AppConfig {
    let toml = r#"
[[zone]]
name = "Ground Floor"

[[zone]]
name = "1st Floor"

[[client]]
name = "Living Room"
mac = "02:42:ac:11:00:10"
zone = "Ground Floor"

[[client]]
name = "Kitchen"
mac = "02:42:ac:11:00:11"
zone = "1st Floor"
"#;
    let raw: config::FileConfig = toml::from_str(toml).expect("test config TOML parses");
    config::load_raw(raw).expect("test config resolves")
}

/// In-process snapdog under test, with captured command channels.
pub struct TestApp {
    /// Shared application state (what handlers see).
    pub state: SharedState,
    /// Direct handle to the in-memory store for setup/inspection.
    pub store: state::SharedState,
    /// Per-zone receivers capturing emitted [`ZoneCommand`]s (1-based index).
    pub zone_rx: HashMap<usize, mpsc::Receiver<ZoneCommand>>,
    /// Receiver capturing emitted [`SnapcastCmd`]s.
    pub snap_rx: mpsc::Receiver<SnapcastCmd>,
    /// Receiver tapping the WebSocket notification broadcast (pre-serialized JSON).
    pub notify_rx: broadcast::Receiver<Arc<str>>,
    // Kept alive so the EqStore-backing TempDir outlives the test.
    _tmp: tempfile::TempDir,
}

/// Construct an [`TestApp`] from a resolved config (no sockets, no auto-save).
#[must_use]
pub fn build_test_app(config: AppConfig) -> TestApp {
    let tmp = tempfile::tempdir().expect("tempdir");
    let store = state::init(&config, None).expect("state init");
    let covers = state::cover::new_cache();
    let (notify_tx, notify_rx) = api::ws::notification_channel();
    let eq_store = Arc::new(Mutex::new(EqStore::load(&tmp.path().join("eq.json"))));

    let mut zone_commands = HashMap::new();
    let mut zone_rx = HashMap::new();
    for z in &config.zones {
        let (tx, rx) = mpsc::channel::<ZoneCommand>(64);
        zone_commands.insert(z.index, tx);
        zone_rx.insert(z.index, rx);
    }
    let (snap_tx, snap_rx) = mpsc::channel::<SnapcastCmd>(64);

    let state: SharedState = Arc::new(AppState {
        config,
        store: store.clone(),
        zone_commands,
        snap_tx,
        covers,
        notifications: notify_tx,
        eq_store,
        knx_device_control: None,
        playlist_cache: tokio::sync::RwLock::new(None),
        speaker_db: snapdog::spinorama::SpeakerDb::new(),
    });

    TestApp {
        state,
        store,
        zone_rx,
        snap_rx,
        notify_rx,
        _tmp: tmp,
    }
}

/// Build a default [`TestApp`] (2 zones / 2 clients).
#[must_use]
pub fn test_app() -> TestApp {
    build_test_app(test_config())
}

impl TestApp {
    /// Drive the full router in-process. `body` is sent as JSON when present.
    /// Returns the status and the parsed JSON body (`Null` if empty/non-JSON).
    pub async fn request(
        &self,
        method: &str,
        uri: &str,
        body: Option<serde_json::Value>,
    ) -> (StatusCode, serde_json::Value) {
        let router = api::build_router(&self.state);
        let builder = Request::builder().method(method).uri(uri);
        let req = match body {
            Some(b) => builder
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&b).expect("serialize body")))
                .expect("build request"),
            None => builder.body(Body::empty()).expect("build request"),
        };
        let resp = router.oneshot(req).await.expect("router responds");
        let status = resp.status();
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .expect("collect body");
        let json = if bytes.is_empty() {
            serde_json::Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
        };
        (status, json)
    }

    /// Convenience: `GET` with no body.
    pub async fn get(&self, uri: &str) -> (StatusCode, serde_json::Value) {
        self.request("GET", uri, None).await
    }

    /// Drain and return all [`ZoneCommand`]s captured for a zone so far.
    pub fn drain_zone(&mut self, zone: usize) -> Vec<ZoneCommand> {
        let mut out = Vec::new();
        if let Some(rx) = self.zone_rx.get_mut(&zone) {
            while let Ok(cmd) = rx.try_recv() {
                out.push(cmd);
            }
        }
        out
    }

    /// Drain and return all [`SnapcastCmd`]s captured so far.
    pub fn drain_snap(&mut self) -> Vec<SnapcastCmd> {
        let mut out = Vec::new();
        while let Ok(cmd) = self.snap_rx.try_recv() {
            out.push(cmd);
        }
        out
    }
}

// ── Zone-player harness (IT-T50 / IT-T82): REAL `spawn_zone_players` ──────────
//
// Unlike `TestApp` (which captures the zone command channels and never runs a
// player), this spawns the real per-zone runner tasks so command→state
// transitions can be asserted end-to-end. Determinism (`IT-DEC-02`):
// `start_receivers = false` (no RAOP socket bind / mDNS), a no-op backend, empty
// group maps (no Snapcast wiring), and the WS broadcast as the sync barrier.

/// No-op [`SnapcastBackend`] for harness tests — every call succeeds, nothing is
/// bound or sent. (The crate's own mock is `#[cfg(test)]`-private, so integration
/// tests need their own copy.)
pub struct MockBackend;

impl SnapcastBackend for MockBackend {
    fn send_audio(
        &self,
        _zone_index: usize,
        _samples: &[f32],
        _sample_rate: u32,
        _channels: u16,
    ) -> BoxFuture<'_, anyhow::Result<()>> {
        Box::pin(async { Ok(()) })
    }
    fn execute(&self, _cmd: SnapcastCmd) -> BoxFuture<'_, anyhow::Result<()>> {
        Box::pin(async { Ok(()) })
    }
    fn stop(&self) -> BoxFuture<'_, anyhow::Result<()>> {
        Box::pin(async { Ok(()) })
    }
    fn get_status(&self) -> BoxFuture<'_, anyhow::Result<serde_json::Value>> {
        Box::pin(async { Ok(serde_json::Value::Null) })
    }
    fn delete_client(&self, _id: &str) -> BoxFuture<'_, anyhow::Result<()>> {
        Box::pin(async { Ok(()) })
    }
}

/// A running set of real zone players plus the channels to observe them.
pub struct ZoneHarness {
    /// 1-based zone index → command sender (drives the real runner task).
    pub senders: HashMap<usize, mpsc::Sender<ZoneCommand>>,
    /// Shared store — assert post-transition state here.
    pub store: state::SharedState,
    /// WebSocket notification tap — the deterministic sync barrier.
    pub notify_rx: broadcast::Receiver<Arc<str>>,
    /// Captured Snapcast commands (empty unless a zone has a group).
    pub snap_rx: mpsc::Receiver<SnapcastCmd>,
    // Kept alive so the EqStore-backing TempDir outlives the test.
    _tmp: tempfile::TempDir,
}

impl ZoneHarness {
    /// Await the next notification whose parsed JSON satisfies `pred`, returning it.
    ///
    /// This is the sync barrier: the runner is a concurrent task, so never poll the
    /// store immediately — await the notification that *proves* the command was
    /// processed, then assert the store. No sleeps, no timing assumptions.
    pub async fn await_notification(
        &mut self,
        pred: impl Fn(&serde_json::Value) -> bool,
    ) -> serde_json::Value {
        loop {
            let raw = self.notify_rx.recv().await.expect("notification received");
            let v: serde_json::Value =
                serde_json::from_str(&raw).expect("notification is valid JSON");
            if pred(&v) {
                return v;
            }
        }
    }
}

/// Spawn real zone players for `config` with receivers disabled (no sockets/mDNS),
/// a no-op backend, and empty group maps — so transitions are observable via the
/// store + notifications without any Snapcast group wiring.
pub async fn spawn_zone_harness(config: AppConfig) -> ZoneHarness {
    let tmp = tempfile::tempdir().expect("tempdir");
    let config = Arc::new(config);
    let store = state::init(&config, None).expect("state init");
    let covers = state::cover::new_cache();
    let (notify_tx, notify_rx) = api::ws::notification_channel();
    let (snap_tx, snap_rx) = mpsc::channel::<SnapcastCmd>(64);
    let eq_store = Arc::new(Mutex::new(EqStore::load(&tmp.path().join("eq.json"))));

    let ctx = ZonePlayerContext {
        config: config.clone(),
        store: store.clone(),
        covers,
        notify: notify_tx,
        snap_tx,
        backend: Arc::new(MockBackend),
        eq_store,
        client_mac_map: HashMap::new(),
        group_ids: Vec::new(),
        group_clients: HashMap::new(),
        start_receivers: false,
        #[cfg(feature = "test-harness")]
        test_pcm_rx: Mutex::new(HashMap::new()),
    };

    let senders = player::spawn_zone_players(ctx)
        .await
        .expect("spawn zone players");

    ZoneHarness {
        senders,
        store,
        notify_rx,
        snap_rx,
        _tmp: tmp,
    }
}

// ── Capturing harness (IT-T55): drive the real decode→send_audio path ─────────
//
// Gated on `feature = "test-harness"` (which enables the
// `ZonePlayerContext::test_pcm_rx` injection seam). Run with:
//   `cargo test -p snapdog --features test-harness`

/// One recorded `SnapcastBackend::send_audio` call.
#[cfg(feature = "test-harness")]
#[derive(Clone, Debug)]
pub struct SendAudioCall {
    pub zone_index: usize,
    pub len: usize,
    pub sample_rate: u32,
    pub channels: u16,
}

/// `SnapcastBackend` double that records every `send_audio` call (the rest are
/// no-ops, like `MockBackend`).
#[cfg(feature = "test-harness")]
#[derive(Clone, Default)]
pub struct CapturingBackend {
    calls: Arc<Mutex<Vec<SendAudioCall>>>,
}

#[cfg(feature = "test-harness")]
impl CapturingBackend {
    /// Snapshot of the `send_audio` calls recorded so far.
    pub fn calls(&self) -> Vec<SendAudioCall> {
        self.calls.lock().unwrap().clone()
    }
}

#[cfg(feature = "test-harness")]
impl SnapcastBackend for CapturingBackend {
    fn send_audio(
        &self,
        zone_index: usize,
        samples: &[f32],
        sample_rate: u32,
        channels: u16,
    ) -> BoxFuture<'_, anyhow::Result<()>> {
        self.calls.lock().unwrap().push(SendAudioCall {
            zone_index,
            len: samples.len(),
            sample_rate,
            channels,
        });
        Box::pin(async { Ok(()) })
    }
    fn execute(&self, _cmd: SnapcastCmd) -> BoxFuture<'_, anyhow::Result<()>> {
        Box::pin(async { Ok(()) })
    }
    fn stop(&self) -> BoxFuture<'_, anyhow::Result<()>> {
        Box::pin(async { Ok(()) })
    }
    fn get_status(&self) -> BoxFuture<'_, anyhow::Result<serde_json::Value>> {
        Box::pin(async { Ok(serde_json::Value::Null) })
    }
    fn delete_client(&self, _id: &str) -> BoxFuture<'_, anyhow::Result<()>> {
        Box::pin(async { Ok(()) })
    }
}

/// Real zone players wired with a [`CapturingBackend`] + per-zone PCM injection.
#[cfg(feature = "test-harness")]
pub struct CapturingHarness {
    /// 1-based zone index → command sender.
    pub senders: HashMap<usize, mpsc::Sender<ZoneCommand>>,
    /// Shared store.
    pub store: state::SharedState,
    /// WS notification tap.
    pub notify_rx: broadcast::Receiver<Arc<str>>,
    /// Captured Snapcast commands.
    pub snap_rx: mpsc::Receiver<SnapcastCmd>,
    /// 1-based zone index → PCM injection sender (feeds the real decode arm).
    pub test_pcm_tx: HashMap<usize, mpsc::Sender<snapdog::audio::PcmMessage>>,
    /// Backend handle sharing the recorded calls with the running players.
    pub backend: CapturingBackend,
    _tmp: tempfile::TempDir,
}

/// Spawn real zone players with a [`CapturingBackend`] and a per-zone PCM
/// injection channel adopted as `decode_rx`, so a test can push
/// [`snapdog::audio::PcmMessage::Audio`] straight into the real
/// resample→EQ→`send_audio` path without a network decode.
#[cfg(feature = "test-harness")]
pub async fn spawn_zone_harness_capturing(config: AppConfig) -> CapturingHarness {
    let tmp = tempfile::tempdir().expect("tempdir");
    let config = Arc::new(config);
    let store = state::init(&config, None).expect("state init");
    let covers = state::cover::new_cache();
    let (notify_tx, notify_rx) = api::ws::notification_channel();
    let (snap_tx, snap_rx) = mpsc::channel::<SnapcastCmd>(64);
    let eq_store = Arc::new(Mutex::new(EqStore::load(&tmp.path().join("eq.json"))));
    let backend = CapturingBackend::default();

    let mut test_pcm_rx = HashMap::new();
    let mut test_pcm_tx = HashMap::new();
    for z in &config.zones {
        let (tx, rx) = mpsc::channel::<snapdog::audio::PcmMessage>(64);
        test_pcm_rx.insert(z.index, rx);
        test_pcm_tx.insert(z.index, tx);
    }

    let ctx = ZonePlayerContext {
        config: config.clone(),
        store: store.clone(),
        covers,
        notify: notify_tx,
        snap_tx,
        backend: Arc::new(backend.clone()),
        eq_store,
        client_mac_map: HashMap::new(),
        group_ids: Vec::new(),
        group_clients: HashMap::new(),
        start_receivers: false,
        test_pcm_rx: Mutex::new(test_pcm_rx),
    };

    let senders = player::spawn_zone_players(ctx)
        .await
        .expect("spawn zone players");

    CapturingHarness {
        senders,
        store,
        notify_rx,
        snap_rx,
        test_pcm_tx,
        backend,
        _tmp: tmp,
    }
}

// ════════════════════════════════════════════════════════════════════════════
// Reusable testkit (IT-T02 / IT-T03 / IT-T06 / IT-T94)
//
// Deterministic primitives shared across the suite — and a stable surface other
// suites (`BT-0001`, `LI-0002`) can copy: an ephemeral-resource pool, virtual-time
// helpers, and a golden-vector harness. All pure/host-only, no mDNS, no real I/O
// beyond loopback sockets.
// ════════════════════════════════════════════════════════════════════════════

// ── Ephemeral resource pool (IT-T02) ─────────────────────────────────────────

/// An ephemeral free TCP port: bind `127.0.0.1:0`, read the kernel-assigned port,
/// release it. Each call yields a distinct port (the kernel won't re-hand one
/// that's still bound), so concurrent callers get unique ports.
pub async fn free_port() -> u16 {
    tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral port")
        .local_addr()
        .expect("local_addr")
        .port()
}

/// Bind an ephemeral loopback listener and return it together with its address —
/// the idiom for tests that hand a live listener to `api::serve` (no port race).
pub async fn bind_ephemeral() -> (tokio::net::TcpListener, std::net::SocketAddr) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral listener");
    let addr = listener.local_addr().expect("local_addr");
    (listener, addr)
}

/// Allocate `n` **mutually-distinct** ephemeral ports. All `n` listeners are bound
/// simultaneously (so the kernel can't hand the same port twice) before any are
/// released, then dropped together — the caller gets `n` free, distinct ports to
/// bind itself (e.g. a managed snapserver's control / streaming / per-zone source
/// ports). Unlike calling [`free_port`] `n` times, distinctness is guaranteed.
pub async fn alloc_ports(n: usize) -> Vec<u16> {
    let mut held = Vec::with_capacity(n);
    for _ in 0..n {
        held.push(
            tokio::net::TcpListener::bind("127.0.0.1:0")
                .await
                .expect("bind ephemeral port"),
        );
    }
    held.iter()
        .map(|l| l.local_addr().expect("local_addr").port())
        .collect()
    // `held` drops here → all ports released, still mutually distinct.
}

/// Deterministic, collision-free unique-name generator (splitmix64). Seeded, so a
/// fixed seed reproduces the exact name sequence — reproducible parallel tests
/// without global state or mDNS collisions (`IT-DEC-02`).
pub struct EphemeralNames {
    state: u64,
}

impl EphemeralNames {
    /// Create a generator with a fixed seed (same seed → same sequence).
    #[must_use]
    pub fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    fn next_u64(&mut self) -> u64 {
        // splitmix64 — small, fast, no external RNG dependency.
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Next unique `"{prefix}-XXXXXXXX"` name (32-bit hex suffix).
    pub fn next(&mut self, prefix: &str) -> String {
        format!("{prefix}-{:08x}", self.next_u64() & 0xFFFF_FFFF)
    }
}

// ── Virtual-time helpers (IT-T03) ────────────────────────────────────────────
//
// Use under `#[tokio::test(start_paused = true)]`: the clock only moves when you
// `advance` it, so a 300 s timer (§5.2: presence auto-off, WS ping cadence) fires
// in microseconds of wall time. Requires the current-thread runtime (the default
// for `#[tokio::test]`).

/// Advance the paused tokio clock by `d` (no wall-clock wait).
pub async fn advance(d: std::time::Duration) {
    tokio::time::advance(d).await;
}

/// Advance the paused tokio clock by `secs` seconds.
pub async fn advance_secs(secs: u64) {
    tokio::time::advance(std::time::Duration::from_secs(secs)).await;
}

// ── Golden-vector harness (IT-T06) ───────────────────────────────────────────
//
// Fixtures live in `tests/fixtures/<name>.json`. `UPDATE_GOLDEN=1` (re)writes the
// golden and passes; otherwise the actual value is compared against it. Two flavours:
// exact (canonical JSON) and float-tolerant (per-element ±tol for audio / float DPT).

/// Absolute path to `tests/fixtures/` for the crate under test.
#[must_use]
pub fn fixtures_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn golden_path(name: &str) -> std::path::PathBuf {
    fixtures_dir().join(format!("{name}.json"))
}

fn write_golden(path: &std::path::Path, contents: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create fixtures dir");
    }
    std::fs::write(path, contents).expect("write golden");
}

fn update_golden_enabled() -> bool {
    std::env::var_os("UPDATE_GOLDEN").is_some_and(|v| v != "0" && !v.is_empty())
}

/// Assert `actual` (canonical pretty JSON) equals `tests/fixtures/<name>.json`.
/// With `UPDATE_GOLDEN=1`, (re)writes the golden and returns.
pub fn assert_json_golden<T: serde::Serialize>(name: &str, actual: &T) {
    let path = golden_path(name);
    let mut actual_str = serde_json::to_string_pretty(actual).expect("serialize golden");
    actual_str.push('\n');
    if update_golden_enabled() {
        write_golden(&path, &actual_str);
        return;
    }
    let expected = std::fs::read_to_string(&path).unwrap_or_else(|_| {
        panic!(
            "missing golden {} — run with UPDATE_GOLDEN=1 to create it",
            path.display()
        )
    });
    assert_eq!(
        actual_str, expected,
        "golden mismatch for `{name}` — run UPDATE_GOLDEN=1 to refresh if intended"
    );
}

/// Assert each element of `actual` is within `tol` of the golden float vector at
/// `tests/fixtures/<name>.json`. With `UPDATE_GOLDEN=1`, (re)writes and returns.
/// Returns `Ok(())` on match; `Err(msg)` describing the first divergence otherwise
/// (so callers can test the comparator itself).
pub fn check_f64_golden_within(name: &str, actual: &[f64], tol: f64) -> Result<(), String> {
    let path = golden_path(name);
    if update_golden_enabled() {
        let mut s = serde_json::to_string_pretty(actual).expect("serialize golden");
        s.push('\n');
        write_golden(&path, &s);
        return Ok(());
    }
    let raw = std::fs::read_to_string(&path)
        .map_err(|_| format!("missing golden {} — run UPDATE_GOLDEN=1", path.display()))?;
    let expected: Vec<f64> =
        serde_json::from_str(&raw).map_err(|e| format!("parse golden: {e}"))?;
    cmp_f64_within(actual, &expected, tol).map_err(|e| format!("golden `{name}`: {e}"))
}

/// Pure, file-free per-element float comparison: `Ok(())` iff lengths match and
/// every element is within `tol`; else `Err` describing the first divergence. The
/// core of [`check_f64_golden_within`], exposed so the comparator can be tested
/// without touching (or being rewritten by `UPDATE_GOLDEN`) a fixture file.
pub fn cmp_f64_within(actual: &[f64], expected: &[f64], tol: f64) -> Result<(), String> {
    if actual.len() != expected.len() {
        return Err(format!("length {} != {}", actual.len(), expected.len()));
    }
    for (i, (a, e)) in actual.iter().zip(expected).enumerate() {
        if (a - e).abs() > tol {
            return Err(format!("[{i}]: {a} vs {e} (tol {tol})"));
        }
    }
    Ok(())
}

/// Panicking wrapper over [`check_f64_golden_within`].
pub fn assert_f64_golden_within(name: &str, actual: &[f64], tol: f64) {
    if let Err(e) = check_f64_golden_within(name, actual, tol) {
        panic!("{e}");
    }
}
