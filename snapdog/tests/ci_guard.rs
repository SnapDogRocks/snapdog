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

const RELEASE_WORKFLOW: &str = include_str!("../../.github/workflows/release.yml");

fn repository_root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("the crate sits in the workspace")
        .to_path_buf()
}

/// MED-0011 section 2.8: no 1.x version (pre-releases included) may exist while the API
/// description has a deprecated entry. The check itself is `.github/scripts/release_guard.py`,
/// which the release workflow also runs, and its probes (a 1.0.0-rc.1 against a description
/// with and without deprecated entries, 0.30.0, the document version `info.version` that must
/// be ignored) are in `test_release_guard.py`. This test runs it on the product version of
/// this checkout, so the release pull request that bumps the version cannot pass `CI Success`
/// while a deprecated entry is left.
#[test]
fn no_one_x_version_while_the_api_has_deprecated_entries() {
    let output = std::process::Command::new("python3")
        .arg(repository_root().join(".github/scripts/release_guard.py"))
        .output()
        .expect("python3 runs the release guard");
    assert!(
        output.status.success(),
        "release guard: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// The guard only protects a release if the workflow still runs it before anything is built.
#[test]
fn the_release_workflow_runs_the_release_guard() {
    let step = RELEASE_WORKFLOW
        .lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .position(|l| l.contains("release_guard.py") && l.contains("--version"));
    let first_build = RELEASE_WORKFLOW
        .lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .position(|l| l.starts_with("  webui:"));
    assert!(
        step.is_some(),
        "release.yml no longer runs release_guard.py --version"
    );
    assert!(
        step < first_build,
        "release_guard.py must run in the first job, before anything is built"
    );
}

/// Non-Rust files that a Rust test reads or embeds. Each must start `rust-core`, or a pull
/// request that changes only that file passes without the test that guards it (#528). Add a
/// file here when a test starts to read one.
const GUARDED_FILES: &[&str] = &[
    ".github/workflows/ci.yml",
    ".github/workflows/release.yml",
    ".github/workflows/release-please.yml",
    ".github/scripts/release_guard.py",
    "README.md",
    "snapdog.example.toml",
    "docs/api/asyncapi.json",
    "aur/snapdog/PKGBUILD",
    "assets/radio-cover.png",
    "knx/snapdog.knxprod",
    "xtask/i18n/de-DE.toml",
    "snapdog/assets/scalar/console.html",
    "snapdog/dbus/snapdog.conf",
    "snapdog/debian/snapdog.toml",
    "snapdog/systemd/snapdog.service",
    "snapdog-client/dbus/snapdog-client.conf",
    "snapdog-client/debian/snapdog-client.default",
];

/// The patterns of one `dorny/paths-filter` filter in ci.yml.
fn path_filter(name: &str) -> Vec<&'static str> {
    let header = format!("            {name}:");
    CI.lines()
        .skip_while(|l| l.trim_end() != header)
        .skip(1)
        .take_while(|l| l.starts_with("              "))
        .filter_map(|l| l.trim().strip_prefix("- "))
        .map(|p| p.trim_matches('\''))
        .collect()
}

/// Minimal glob as paths-filter (picomatch) reads these patterns: `**` spans directories,
/// `*` stays within one path segment.
fn glob_matches(pattern: &str, path: &str) -> bool {
    fn go(p: &[u8], s: &[u8]) -> bool {
        match p {
            [] => s.is_empty(),
            [b'*', b'*', b'/', rest @ ..] => {
                (0..=s.len()).any(|i| (i == 0 || s[i - 1] == b'/') && go(rest, &s[i..]))
            }
            [b'*', b'*', rest @ ..] => (0..=s.len()).any(|i| go(rest, &s[i..])),
            [b'*', rest @ ..] => (0..=s.len())
                .take_while(|&i| i == 0 || s[i - 1] != b'/')
                .any(|i| go(rest, &s[i..])),
            [c, rest @ ..] => s.first() == Some(c) && go(rest, &s[1..]),
        }
    }
    go(pattern.as_bytes(), path.as_bytes())
}

#[test]
fn the_glob_reads_patterns_like_paths_filter() {
    assert!(glob_matches("**/*.rs", "snapdog/src/main.rs"));
    assert!(glob_matches("**/*.rs", "build.rs"));
    assert!(glob_matches("knx/**", "knx/snapdog.knxprod"));
    assert!(glob_matches("rust-toolchain*", "rust-toolchain.toml"));
    assert!(!glob_matches("knx/**", "snapdog/knx/x"));
    assert!(!glob_matches("docs/*.json", "docs/api/asyncapi.json"));
}

#[test]
fn every_guarded_file_starts_the_rust_tests() {
    let rust = path_filter("rust");
    assert!(
        rust.contains(&"**/*.rs"),
        "the `rust` filter was not found in ci.yml"
    );
    for file in GUARDED_FILES {
        assert!(
            repository_root().join(file).is_file(),
            "{file} is listed in GUARDED_FILES but does not exist"
        );
        assert!(
            rust.iter().any(|p| glob_matches(p, file)),
            "{file} is read by a Rust test but does not start rust-core: add it to the `rust` \
             filter in ci.yml"
        );
    }
}

#[test]
fn ci_runs_the_updater_script_tests() {
    assert!(
        active_lines().any(|l| l.contains("run: bash deploy/updater/test-update.sh")),
        "ci.yml no longer runs deploy/updater/test-update.sh"
    );
}
