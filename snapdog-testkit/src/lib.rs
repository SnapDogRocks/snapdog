// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! # snapdog-testkit
//!
//! Reusable, deterministic test primitives extracted from the SnapDog integration
//! suite (RFC IT-0003, task `IT-T94`) so sibling suites (`BT-0001`, `LI-0002`) can
//! depend on them instead of copy-pasting:
//!
//! - [`ephemeral`] — collision-free ports + seeded, reproducible names.
//! - [`time`] — virtual-time helpers for `#[tokio::test(start_paused = true)]`.
//! - [`golden`] — file-backed golden vectors (exact + float-tolerant) with
//!   `UPDATE_GOLDEN=1` regeneration; the fixtures directory is caller-supplied so
//!   each consumer keeps its own `tests/fixtures/`.
//! - [`capture`] — capture emitted events for assertion, for both channel-style
//!   ([`capture::drain`]) and callback-style ([`capture::EventSink`]) producers.
//!
//! Everything is host-only and pure — no sockets beyond loopback, no mDNS, no
//! wall-clock sleeps.

/// Ephemeral resource pool: free ports + unique, reproducible names.
pub mod ephemeral {
    use std::net::SocketAddr;

    use tokio::net::TcpListener;

    /// An ephemeral free TCP port: bind `127.0.0.1:0`, read the kernel-assigned
    /// port, release it. Distinct across concurrent callers (the kernel won't
    /// re-hand a still-bound port); for *guaranteed* mutual distinctness across a
    /// batch, prefer [`alloc_ports`].
    pub async fn free_port() -> u16 {
        TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind ephemeral port")
            .local_addr()
            .expect("local_addr")
            .port()
    }

    /// Bind an ephemeral loopback listener; return it with its address — the idiom
    /// for handing a live listener to a server under test (no port race).
    pub async fn bind_ephemeral() -> (TcpListener, SocketAddr) {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind ephemeral listener");
        let addr = listener.local_addr().expect("local_addr");
        (listener, addr)
    }

    /// Allocate `n` **mutually-distinct** ephemeral ports. All `n` listeners are
    /// bound at once (so the kernel can't hand the same port twice) before any are
    /// released, then dropped together — the caller gets `n` free, distinct ports
    /// to bind itself. Unlike calling [`free_port`] `n` times, distinctness holds.
    pub async fn alloc_ports(n: usize) -> Vec<u16> {
        let mut held = Vec::with_capacity(n);
        for _ in 0..n {
            held.push(
                TcpListener::bind("127.0.0.1:0")
                    .await
                    .expect("bind ephemeral port"),
            );
        }
        held.iter()
            .map(|l| l.local_addr().expect("local_addr").port())
            .collect()
        // `held` drops here → all ports released, still mutually distinct.
    }

    /// Deterministic, collision-free unique-name generator (splitmix64). Seeded, so
    /// a fixed seed reproduces the exact name sequence — reproducible parallel tests
    /// without global state or mDNS collisions.
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
}

/// Virtual-time helpers. Use under `#[tokio::test(start_paused = true)]`: the clock
/// only advances when you tell it to, so a 300 s timer fires in µs of wall time.
pub mod time {
    use std::time::Duration;

    /// Advance the paused tokio clock by `d` (no wall-clock wait).
    pub async fn advance(d: Duration) {
        tokio::time::advance(d).await;
    }

    /// Advance the paused tokio clock by `secs` seconds.
    pub async fn advance_secs(secs: u64) {
        tokio::time::advance(Duration::from_secs(secs)).await;
    }
}

/// Golden-vector harness. Fixtures live at `<fixtures_dir>/<name>.json`; the
/// directory is supplied by the caller (typically
/// `Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")`). Set
/// `UPDATE_GOLDEN=1` to (re)write goldens instead of comparing.
pub mod golden {
    use std::path::Path;

    use serde::Serialize;

    fn update_enabled() -> bool {
        std::env::var_os("UPDATE_GOLDEN").is_some_and(|v| v != "0" && !v.is_empty())
    }

    fn write(path: &Path, contents: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create fixtures dir");
        }
        std::fs::write(path, contents).expect("write golden");
    }

    /// Assert `actual` (canonical pretty JSON) equals `<fixtures_dir>/<name>.json`.
    /// With `UPDATE_GOLDEN=1`, (re)writes the golden and returns.
    ///
    /// # Panics
    /// Panics on mismatch, or if the golden is missing and `UPDATE_GOLDEN` is unset.
    pub fn assert_json_golden<T: Serialize>(fixtures_dir: &Path, name: &str, actual: &T) {
        let path = fixtures_dir.join(format!("{name}.json"));
        let mut actual_str = serde_json::to_string_pretty(actual).expect("serialize golden");
        actual_str.push('\n');
        if update_enabled() {
            write(&path, &actual_str);
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

    /// Compare `actual` against the golden float vector at `<fixtures_dir>/<name>.json`,
    /// each element within `tol`. With `UPDATE_GOLDEN=1`, (re)writes and returns `Ok`.
    /// Returns `Err(msg)` describing the first divergence otherwise.
    pub fn check_f64_golden_within(
        fixtures_dir: &Path,
        name: &str,
        actual: &[f64],
        tol: f64,
    ) -> Result<(), String> {
        let path = fixtures_dir.join(format!("{name}.json"));
        if update_enabled() {
            let mut s = serde_json::to_string_pretty(actual).expect("serialize golden");
            s.push('\n');
            write(&path, &s);
            return Ok(());
        }
        let raw = std::fs::read_to_string(&path)
            .map_err(|_| format!("missing golden {} — run UPDATE_GOLDEN=1", path.display()))?;
        let expected: Vec<f64> =
            serde_json::from_str(&raw).map_err(|e| format!("parse golden: {e}"))?;
        cmp_f64_within(actual, &expected, tol).map_err(|e| format!("golden `{name}`: {e}"))
    }

    /// Panicking wrapper over [`check_f64_golden_within`].
    ///
    /// # Panics
    /// Panics if `actual` diverges from the golden beyond `tol`.
    pub fn assert_f64_golden_within(fixtures_dir: &Path, name: &str, actual: &[f64], tol: f64) {
        if let Err(e) = check_f64_golden_within(fixtures_dir, name, actual, tol) {
            panic!("{e}");
        }
    }

    /// Pure, file-free per-element float comparison: `Ok(())` iff lengths match and
    /// every element is within `tol`; else `Err` describing the first divergence.
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
}

/// Event capture for assertion — channel-style ([`drain`] / [`drain_available`])
/// for `mpsc`-based producers, and callback-style ([`EventSink`]) for producers
/// that take a closure/handle. Generic over the event type `E`, so it works for
/// `snapdog::receiver::ReceiverEvent`, KNX telegrams, or any sibling suite's events.
pub mod capture {
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use tokio::sync::mpsc;
    use tokio::time::{Instant, timeout_at};

    /// Drain up to `max` events from `rx`, stopping early on channel close or when
    /// `timeout` elapses. Returns what was collected (never panics on timeout).
    pub async fn drain<E>(rx: &mut mpsc::Receiver<E>, max: usize, timeout: Duration) -> Vec<E> {
        let mut out = Vec::new();
        let deadline = Instant::now() + timeout;
        while out.len() < max {
            match timeout_at(deadline, rx.recv()).await {
                Ok(Some(e)) => out.push(e),
                Ok(None) | Err(_) => break, // channel closed or deadline
            }
        }
        out
    }

    /// Drain every event currently queued in `rx` without waiting.
    pub fn drain_available<E>(rx: &mut mpsc::Receiver<E>) -> Vec<E> {
        let mut out = Vec::new();
        while let Ok(e) = rx.try_recv() {
            out.push(e);
        }
        out
    }

    /// A cloneable sink for callback-style producers: hand out `.push`, snapshot
    /// with `.take`. Cheap to clone (shares one buffer).
    pub struct EventSink<E> {
        inner: Arc<Mutex<Vec<E>>>,
    }

    impl<E> Default for EventSink<E> {
        fn default() -> Self {
            Self::new()
        }
    }

    impl<E> Clone for EventSink<E> {
        fn clone(&self) -> Self {
            Self {
                inner: Arc::clone(&self.inner),
            }
        }
    }

    impl<E> EventSink<E> {
        /// Create an empty sink.
        #[must_use]
        pub fn new() -> Self {
            Self {
                inner: Arc::new(Mutex::new(Vec::new())),
            }
        }

        /// Record one event.
        pub fn push(&self, event: E) {
            self.inner.lock().unwrap().push(event);
        }

        /// Drain and return everything recorded so far.
        pub fn take(&self) -> Vec<E> {
            std::mem::take(&mut *self.inner.lock().unwrap())
        }

        /// Number of events recorded so far.
        #[must_use]
        pub fn len(&self) -> usize {
            self.inner.lock().unwrap().len()
        }

        /// Whether nothing has been recorded yet.
        #[must_use]
        pub fn is_empty(&self) -> bool {
            self.inner.lock().unwrap().is_empty()
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;

    #[tokio::test]
    async fn alloc_ports_are_mutually_distinct() {
        let ports = ephemeral::alloc_ports(32).await;
        let mut sorted = ports.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), ports.len(), "ports collided: {ports:?}");
        assert!(ports.iter().all(|&p| p != 0));
    }

    #[test]
    fn ephemeral_names_unique_and_seed_reproducible() {
        let mut a = ephemeral::EphemeralNames::new(7);
        let mut b = ephemeral::EphemeralNames::new(7);
        let sa: Vec<String> = (0..100).map(|_| a.next("z")).collect();
        let sb: Vec<String> = (0..100).map(|_| b.next("z")).collect();
        assert_eq!(sa, sb, "fixed seed reproduces the sequence");
        let mut u = sa.clone();
        u.sort();
        u.dedup();
        assert_eq!(u.len(), sa.len(), "no collisions");
        let mut c = ephemeral::EphemeralNames::new(8);
        let sc: Vec<String> = (0..100).map(|_| c.next("z")).collect();
        assert_ne!(sa, sc, "distinct seeds diverge");
    }

    #[tokio::test(start_paused = true)]
    async fn virtual_time_advances_instantly() {
        let wall = Instant::now();
        let h = tokio::spawn(async {
            tokio::time::sleep(Duration::from_secs(300)).await;
            "fired"
        });
        tokio::task::yield_now().await;
        assert!(!h.is_finished());
        time::advance_secs(300).await;
        assert_eq!(h.await.unwrap(), "fired");
        assert!(wall.elapsed() < Duration::from_millis(50));
    }

    #[test]
    fn cmp_f64_within_ok_iff_within_tolerance() {
        let base = [0.0_f64, 0.25, -0.25, 1.0];
        let near = [0.0_f64, 0.2500004, -0.25, 1.0];
        assert!(golden::cmp_f64_within(&base, &base, 1e-9).is_ok());
        assert!(golden::cmp_f64_within(&near, &base, 1e-6).is_ok());
        assert!(golden::cmp_f64_within(&near, &base, 1e-9).is_err());
        assert!(golden::cmp_f64_within(&base[..2], &base, 1.0).is_err());
    }

    #[test]
    fn json_golden_roundtrips_via_tempdir() {
        // File-backed compare without touching UPDATE_GOLDEN (avoid env races):
        // pre-write the golden, then assert a matching value passes.
        let dir = tempfile::tempdir().unwrap();
        let value = serde_json::json!({ "a": 1, "b": ["x", "y"] });
        let pretty = format!("{}\n", serde_json::to_string_pretty(&value).unwrap());
        std::fs::write(dir.path().join("demo.json"), pretty).unwrap();
        golden::assert_json_golden(dir.path(), "demo", &value); // matches → no panic
    }

    #[tokio::test]
    async fn capture_drain_collects_until_close() {
        let (tx, mut rx) = tokio::sync::mpsc::channel::<u32>(8);
        for i in 0..3 {
            tx.send(i).await.unwrap();
        }
        drop(tx); // close → drain stops early even though max is larger
        let got = capture::drain(&mut rx, 10, Duration::from_secs(1)).await;
        assert_eq!(got, vec![0, 1, 2]);
    }

    #[test]
    fn event_sink_records_and_takes() {
        let sink = capture::EventSink::<&str>::new();
        assert!(sink.is_empty());
        sink.push("a");
        sink.clone().push("b"); // clone shares the buffer
        assert_eq!(sink.len(), 2);
        assert_eq!(sink.take(), vec!["a", "b"]);
        assert!(sink.is_empty(), "take drains");
    }
}
