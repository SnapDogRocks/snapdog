// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! Tests for the reusable testkit primitives in `common`:
//! - `IT-T02` ephemeral resource pool (unique ports, seeded collision-free names)
//! - `IT-T03` virtual-time helpers (a 300 s timer fires in <50 ms wall time)
//! - `IT-T06` golden-vector harness (exact + float-tolerant; `UPDATE_GOLDEN` regen)
//!
//! Tier-1, default features. Run: `cargo test -p snapdog --test testkit`.

#![allow(clippy::doc_markdown)]

mod common;

use std::time::{Duration, Instant};

// ── IT-T02: ephemeral resource pool ──────────────────────────────────────────

#[tokio::test]
async fn alloc_ports_are_mutually_distinct() {
    let ports = common::alloc_ports(32).await;
    assert_eq!(ports.len(), 32);
    let mut sorted = ports.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(sorted.len(), ports.len(), "ports collided: {ports:?}");
    assert!(ports.iter().all(|&p| p != 0), "no port-0 placeholders");
}

#[tokio::test]
async fn free_port_binds_and_releases() {
    // A returned free port must be immediately re-bindable (it was released).
    let p = common::free_port().await;
    let l = tokio::net::TcpListener::bind(("127.0.0.1", p)).await;
    assert!(l.is_ok(), "free_port {p} should be re-bindable");
}

#[test]
fn ephemeral_names_unique_and_seed_reproducible() {
    // Same seed → identical sequence.
    let mut a = common::EphemeralNames::new(42);
    let mut b = common::EphemeralNames::new(42);
    let seq_a: Vec<String> = (0..100).map(|_| a.next("zone")).collect();
    let seq_b: Vec<String> = (0..100).map(|_| b.next("zone")).collect();
    assert_eq!(seq_a, seq_b, "fixed seed must reproduce the name sequence");

    // Collision-free within a sequence.
    let mut uniq = seq_a.clone();
    uniq.sort();
    uniq.dedup();
    assert_eq!(uniq.len(), seq_a.len(), "names collided: {seq_a:?}");

    // Prefix honoured.
    assert!(seq_a.iter().all(|n| n.starts_with("zone-")));

    // A different seed yields a different sequence.
    let mut c = common::EphemeralNames::new(43);
    let seq_c: Vec<String> = (0..100).map(|_| c.next("zone")).collect();
    assert_ne!(seq_a, seq_c, "distinct seeds should diverge");
}

// ── IT-T03: virtual time ─────────────────────────────────────────────────────

#[tokio::test(start_paused = true)]
async fn virtual_time_advances_a_300s_timer_instantly() {
    let wall = Instant::now();
    let handle = tokio::spawn(async {
        tokio::time::sleep(Duration::from_secs(300)).await;
        "fired"
    });

    // Nothing has elapsed in virtual time yet — the timer is still pending.
    tokio::task::yield_now().await;
    assert!(!handle.is_finished(), "timer must not fire before advance");

    common::advance_secs(300).await;
    assert_eq!(handle.await.unwrap(), "fired");
    assert!(
        wall.elapsed() < Duration::from_millis(50),
        "300 s of virtual time took {:?} of wall time",
        wall.elapsed()
    );
}

// ── IT-T06: golden-vector harness ────────────────────────────────────────────

#[test]
fn json_golden_roundtrips() {
    // Round-trips against the committed fixture tests/fixtures/testkit_demo.json.
    // (Regenerate with UPDATE_GOLDEN=1 if the shape intentionally changes.)
    let actual = serde_json::json!({
        "name": "Zone1",
        "ports": { "control": 1705, "streaming": 1704 },
        "streams": ["Zone1", "Zone2"],
    });
    common::assert_json_golden("testkit_demo", &actual);
}

#[test]
fn f64_golden_roundtrips_against_fixture() {
    // Round-trips against the committed fixture tests/fixtures/testkit_sine.json,
    // and regenerates it under UPDATE_GOLDEN=1. A single call, so it's stable in
    // both modes (unlike the tolerance assertions below, which are file-free).
    let base = [0.0_f64, 0.25, -0.25, 0.5, -0.5, 1.0];
    common::assert_f64_golden_within("testkit_sine", &base, 1e-9);
}

#[test]
fn f64_comparator_is_ok_iff_within_tolerance() {
    // Pure comparator (no fixture file, unaffected by UPDATE_GOLDEN).
    let base = [0.0_f64, 0.25, -0.25, 0.5, -0.5, 1.0];
    let perturbed = [0.0_f64, 0.2500004, -0.25, 0.5, -0.5, 1.0];

    assert!(common::cmp_f64_within(&base, &base, 1e-9).is_ok());
    assert!(common::cmp_f64_within(&perturbed, &base, 1e-6).is_ok()); // loose tol passes
    assert!(common::cmp_f64_within(&perturbed, &base, 1e-9).is_err()); // tight tol fails
    assert!(common::cmp_f64_within(&base[..3], &base, 1.0).is_err()); // length mismatch
}
