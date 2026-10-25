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

// ── snapdog.example.toml must only use keys the code knows ────────────────
//
// The `.deb` installs this file as the default configuration and users copy it,
// so a stale key there is a lie that no parser complains about (unknown keys are
// ignored for compatibility). Both the active lines and the commented-out
// examples are checked.

const EXAMPLE: &str = include_str!("../../snapdog.example.toml");

/// Table paths that are TOML arrays of tables in `FileConfig`.
const ARRAY_TABLES: &[&str] = &["zone", "client", "radio", "zone.presence.schedule"];

/// Unknown key paths for a snippet. Required fields may be missing in a snippet,
/// so a final deserialisation error is irrelevant: only the keys seen matter.
fn unknown_in(snippet: &str) -> Vec<String> {
    let mut unknown = Vec::new();
    let deserializer = toml::Deserializer::parse(snippet)
        .unwrap_or_else(|e| panic!("snippet is not valid TOML: {e}\n{snippet}"));
    let _: Result<config::FileConfig, _> = serde_ignored::deserialize(deserializer, |path| {
        unknown.push(config::display_key_path(&path.to_string()));
    });
    unknown
}

/// `[zone.presence.schedule]` → the header lines that make it a valid snippet
/// (`[[zone]]`, `[zone.presence]`, `[[zone.presence.schedule]]`).
fn header_chain(path: &str) -> String {
    let mut chain = String::new();
    let mut prefix = String::new();
    for part in path.split('.').filter(|p| !p.is_empty()) {
        if !prefix.is_empty() {
            prefix.push('.');
        }
        prefix.push_str(part);
        let (open, close) = if ARRAY_TABLES.contains(&prefix.as_str()) {
            ("[[", "]]")
        } else {
            ("[", "]")
        };
        chain.extend([open, &prefix, close, "\n"]);
    }
    chain
}

fn is_key_line(text: &str) -> bool {
    text.split_once(" = ").is_some_and(|(key, _)| {
        !key.is_empty()
            && key
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
    })
}

fn header_path(text: &str) -> Option<&str> {
    let inner = text
        .strip_prefix("[[")
        .and_then(|t| t.split_once("]]"))
        .or_else(|| text.strip_prefix('[').and_then(|t| t.split_once(']')))?
        .0;
    inner
        .chars()
        .all(|c| c.is_ascii_lowercase() || c == '_' || c == '.')
        .then_some(inner)
}

#[test]
fn example_config_loads_without_warnings() {
    let config = config::load_str(EXAMPLE).expect("example config loads");
    let warnings: Vec<_> = config
        .notices
        .iter()
        .filter(|n| n.level == config::NoticeLevel::Warn)
        .map(|n| n.message.as_str())
        .collect();
    assert!(warnings.is_empty(), "example config warns: {warnings:#?}");
}

#[test]
fn example_config_has_no_unknown_keys_active_or_commented() {
    let mut context = String::new();
    let mut problems = Vec::new();
    let mut checked = 0;

    for (number, line) in EXAMPLE.lines().enumerate() {
        let number = number + 1;
        let text = line.trim();
        // `# key = value` and `# [table]` are commented-out examples; anything
        // else after `#` is prose.
        let (text, commented) = text
            .strip_prefix('#')
            .map_or((text, false), |rest| (rest.trim_start(), true));

        if let Some(path) = header_path(text) {
            context = path.to_string();
            if commented {
                let snippet = header_chain(path);
                for key in unknown_in(&snippet) {
                    problems.push(format!("line {number}: unknown table {key}"));
                }
                checked += 1;
            }
        } else if commented && is_key_line(text) {
            let snippet = format!("{}{text}\n", header_chain(&context));
            for key in unknown_in(&snippet) {
                problems.push(format!("line {number}: unknown key {key} (in [{context}])"));
            }
            checked += 1;
        }
    }

    assert!(
        checked > 100,
        "only {checked} commented examples found; parser broken?"
    );
    assert!(
        problems.is_empty(),
        "stale keys in snapdog.example.toml:\n{}",
        problems.join("\n")
    );
}

#[test]
fn unknown_key_checker_catches_the_keys_that_used_to_be_stale() {
    let stale = format!(
        "{}presence_timeout = \"1/1/53\"\n",
        header_chain("zone.knx")
    );
    assert_eq!(unknown_in(&stale), ["zone[0].knx.presence_timeout"]);
    assert_eq!(unknown_in("[spotify]\nnaem = \"x\"\n"), ["spotify.naem"]);
    assert_eq!(
        unknown_in("[spotify]\nname = \"x\"\n"),
        Vec::<String>::new()
    );
}

#[test]
fn example_config_documents_every_zone_and_client_knx_key() {
    // The example is the only place that lists the key names; a key added to the
    // code without an example line would otherwise never be documented.
    let keys = |table: &str| -> Vec<String> {
        let mut found = Vec::new();
        let mut inside = false;
        for line in EXAMPLE.lines() {
            let text = line.trim().trim_start_matches('#').trim_start();
            if let Some(path) = header_path(text) {
                inside = path == table;
            } else if inside && is_key_line(text) {
                found.push(text.split_once(" = ").unwrap().0.to_string());
            }
        }
        found
    };
    let zone = keys("zone.knx");
    assert_eq!(zone.len(), 34, "zone.knx keys in the example: {zone:?}");
    let client = keys("client.knx");
    assert_eq!(
        client.len(),
        11,
        "client.knx keys in the example: {client:?}"
    );
}
