// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! Keeps the CI step alive that runs the feature-gated tests. The tests that prove
//! the `OpenAPI` document sits behind the API key and the console is opt-in exist only
//! with the `api-docs` feature, and every other CI test run uses default features, so
//! without this step those guards would compile out and never run.

const CI: &str = include_str!("../../.github/workflows/ci.yml");

/// Active (not commented-out) lines of the workflow.
fn active_lines() -> impl Iterator<Item = &'static str> {
    CI.lines().filter(|l| !l.trim_start().starts_with('#'))
}

#[test]
fn ci_runs_the_api_docs_guards_with_the_full_feature_set() {
    for required in [
        "api_docs",
        "api_console",
        "api_console_warning",
        "openapi_contract",
    ] {
        let found = active_lines().any(|line| {
            let words: Vec<&str> = line.split_whitespace().collect();
            line.contains("cargo test")
                && line.contains("-p snapdog")
                && line.contains("--features full")
                // A whole word: `--test api_console` is not `--test api_console_warning`.
                && words.windows(2).any(|w| w == ["--test", required])
        });
        assert!(
            found,
            "ci.yml no longer runs `cargo test -p snapdog --features full --test {required}`: \
             the guards behind `#[cfg(feature = \"api-docs\")]` would never run"
        );
    }
}

#[test]
fn the_test_files_the_step_names_exist() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests");
    for name in [
        "api_docs",
        "api_console",
        "api_console_warning",
        "openapi_contract",
        "config_contract",
        "config_cli",
    ] {
        assert!(
            std::path::Path::new(&format!("{dir}/{name}.rs")).is_file(),
            "{name}.rs is named by ci.yml but does not exist"
        );
    }
}
