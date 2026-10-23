// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! Config + group-volume-mode contract (IT-T53 + config resolution). Pure.

mod common;

use snapdog::config::{self, GroupVolumeMode};

#[test]
fn load_raw_resolves_two_zones_two_clients() {
    let cfg = common::test_config();
    assert_eq!(cfg.zones.len(), 2);
    assert_eq!(cfg.clients.len(), 2);
    // 1-based, contiguous indices assigned by convention.
    assert_eq!(cfg.zones[0].index, 1);
    assert_eq!(cfg.zones[1].index, 2);
}

#[test]
fn group_volume_mode_absolute_caps_at_max() {
    assert_eq!(GroupVolumeMode::Absolute.effective(0, 80, 100), 80);
    assert_eq!(GroupVolumeMode::Absolute.effective(0, 80, 50), 50);
}

#[test]
fn group_volume_mode_relative_scales_base_by_zone() {
    assert_eq!(GroupVolumeMode::Relative.effective(50, 50, 100), 25);
    assert_eq!(GroupVolumeMode::Relative.effective(100, 100, 100), 100);
    assert_eq!(GroupVolumeMode::Relative.effective(100, 0, 100), 0);
}

#[test]
fn group_volume_mode_compressed_uses_sqrt_curve() {
    // 100 * sqrt(25/100) = 100 * 0.5 = 50
    assert_eq!(GroupVolumeMode::Compressed.effective(100, 25, 100), 50);
}

#[test]
fn base_for_inverts_effective_wherever_the_volume_is_reachable() {
    for mode in [GroupVolumeMode::Relative, GroupVolumeMode::Compressed] {
        for max in [100, 70] {
            for zone in 1..=100 {
                let ceiling = mode.effective(max, zone, max);
                for wanted in 0..=100 {
                    let base = mode.base_for(wanted, zone, max).unwrap();
                    assert!((0..=max).contains(&base), "{mode:?} base {base}");
                    let reached = mode.effective(base, zone, max);
                    assert_eq!(
                        reached,
                        wanted.min(ceiling),
                        "{mode:?} zone {zone} max {max} wanted {wanted}"
                    );
                }
            }
        }
    }
}

#[test]
fn base_for_has_no_answer_in_absolute_mode_or_a_silent_zone() {
    assert_eq!(GroupVolumeMode::Absolute.base_for(40, 80, 100), None);
    assert_eq!(GroupVolumeMode::Relative.base_for(40, 0, 100), None);
    assert_eq!(GroupVolumeMode::Compressed.base_for(40, 0, 100), None);
}

#[test]
fn parse_time_accepts_hh_mm_and_rejects_garbage() {
    assert!(config::parse_time("07:30").is_ok());
    assert!(config::parse_time("00:00").is_ok());
    assert!(config::parse_time("nope").is_err());
    assert!(config::parse_time("aa:bb").is_err());
}
