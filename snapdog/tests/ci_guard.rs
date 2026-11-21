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

/// The text of one top-level job in the release workflow, up to the next job.
fn release_job(name: &str) -> &'static str {
    let start = RELEASE_WORKFLOW
        .find(&format!("\n  {name}:\n"))
        .unwrap_or_else(|| panic!("release.yml has no job {name}"))
        + 1;
    let rest = &RELEASE_WORKFLOW[start..];
    let end = rest[1..]
        .match_indices("\n  ")
        .find(|(i, _)| {
            let line = &rest[1 + i + 1..];
            !line.starts_with(' ') && !line.starts_with('#')
        })
        .map_or(rest.len(), |(i, _)| 1 + i);
    &rest[..end]
}

/// #527: the version tags are pushed while the release is still being verified, the floating
/// tags only by `promote-images` once it is live. A floating tag pushed by a build job stays
/// on an unpromoted (possibly unsigned) version when the release fails later.
#[test]
fn floating_image_tags_move_only_after_promotion() {
    let updater = release_job("updater-image");
    assert!(
        !updater
            .lines()
            .filter(|l| !l.trim_start().starts_with('#'))
            .any(|l| l.contains(":latest")),
        "updater-image must push only the version tag; promote-images moves :latest"
    );
    let promote = release_job("promote-images");
    for repo in [
        "ghcr.io/snapdogrocks/snapdog\n",
        "ghcr.io/snapdogrocks/snapdog-client\n",
        "ghcr.io/snapdogrocks/snapdog-updater\n",
    ] {
        assert!(
            promote.contains(&format!("repo: {repo}")),
            "promote-images does not move the floating tags of {}",
            repo.trim_end()
        );
    }
}
