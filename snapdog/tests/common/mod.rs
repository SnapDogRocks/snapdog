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
use snapdog::player::{SnapcastCmd, ZoneCommand};
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
