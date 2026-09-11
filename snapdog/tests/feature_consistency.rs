//! Keeps the `full` feature (Cargo.toml) honest: every feature that should ship
//! in a released binary has to be listed there, and no test-only feature ever
//! sneaks in. A future contributor who adds a feature and forgets to update
//! `full` gets a failing test here instead of a silently incomplete release
//! build (see rust.md's "Features" section).

use std::collections::BTreeSet;

/// Features that exist purely for the test suite and must never ship.
const TEST_ONLY_FEATURES: &[&str] = &["test-harness"];

/// Features that select an alternate, mutually-exclusive *base engine*
/// (guarded by the `compile_error!` in src/main.rs) rather than adding a
/// capability on top of the default one. `full` builds on the default engine
/// (`snapcast-embedded`); these are exercised by their own CI rows/matrix
/// instead of being folded into `full`.
const ALTERNATE_ENGINE_FEATURES: &[&str] = &["snapcast-process"];

#[test]
fn full_feature_matches_every_shippable_feature() {
    let manifest_path = format!("{}/Cargo.toml", env!("CARGO_MANIFEST_DIR"));
    let contents = std::fs::read_to_string(&manifest_path)
        .unwrap_or_else(|e| panic!("failed to read {manifest_path}: {e}"));
    let manifest: toml::Table = contents
        .parse()
        .unwrap_or_else(|e| panic!("failed to parse {manifest_path}: {e}"));

    let features = manifest
        .get("features")
        .and_then(toml::Value::as_table)
        .expect("Cargo.toml must have a [features] table");

    let full: BTreeSet<&str> = features
        .get("full")
        .and_then(toml::Value::as_array)
        .expect("Cargo.toml [features] must declare a `full` feature as an array")
        .iter()
        .map(|v| {
            v.as_str()
                .expect("every entry of the `full` feature must be a string")
        })
        .collect();

    for name in features.keys() {
        let name = name.as_str();
        if name == "default" || name == "full" {
            continue;
        }

        if TEST_ONLY_FEATURES.contains(&name) {
            assert!(
                !full.contains(name),
                "`full` must not include the test-only feature `{name}`; test \
                 scaffolding has no business in a release build"
            );
            continue;
        }

        if ALTERNATE_ENGINE_FEATURES.contains(&name) {
            // Deliberately excluded — see the module doc comment above.
            continue;
        }

        assert!(
            full.contains(name),
            "feature `{name}` is neither test-only nor an alternate base engine, \
             so it ships in a release build and must be listed in `full`; if it \
             is genuinely test-only or an alternate engine instead, add it to \
             TEST_ONLY_FEATURES / ALTERNATE_ENGINE_FEATURES in this test"
        );
    }

    for name in &full {
        assert!(
            !TEST_ONLY_FEATURES.contains(name),
            "`full` lists test-only feature `{name}`; remove it from `full`"
        );
    }
}
