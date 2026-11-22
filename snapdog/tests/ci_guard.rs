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

/// Workflows whose commands build, run or test the workspace.
const fn build_workflows() -> [(&'static str, &'static str); 4] {
    [
        ("ci.yml", CI),
        ("release.yml", RELEASE_WORKFLOW),
        (
            "nightly.yml",
            include_str!("../../.github/workflows/nightly.yml"),
        ),
        (
            "tier2-integration.yml",
            include_str!("../../.github/workflows/tier2-integration.yml"),
        ),
    ]
}

/// Shell commands of a workflow with `\` continuations joined, comments dropped.
fn workflow_commands(workflow: &str) -> Vec<String> {
    let mut commands = Vec::new();
    let mut pending = String::new();
    for line in workflow.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('#') {
            continue;
        }
        if let Some(head) = trimmed.strip_suffix('\\') {
            pending.push_str(head);
            pending.push(' ');
            continue;
        }
        pending.push_str(trimmed);
        commands.push(std::mem::take(&mut pending));
    }
    commands
}

/// #532: every command that resolves the dependency graph uses `--locked`, so a lockfile
/// that lags the manifest fails instead of being re-resolved silently (above all at the tag).
#[test]
fn every_cargo_build_run_and_test_is_locked() {
    let subcommands = [
        "build", "run", "test", "nextest", "clippy", "llvm-cov", "check", "deb",
    ];
    for (name, workflow) in build_workflows() {
        for command in workflow_commands(workflow) {
            for segment in command.split(['|', ';', '(']).flat_map(|s| s.split("&&")) {
                let words: Vec<&str> = segment.split_whitespace().collect();
                let Some(at) = words.iter().position(|w| *w == "cargo") else {
                    continue;
                };
                if words[..at].contains(&"name:") {
                    continue; // a step name such as `- name: cargo check`, not a command
                }
                if words.get(at + 1).is_some_and(|w| subcommands.contains(w)) {
                    assert!(
                        words.contains(&"--locked"),
                        "{name}: `{}` runs without --locked",
                        segment.trim()
                    );
                }
            }
        }
    }
}

/// #532: images are referenced by digest. A floating tag alone (`:latest`, `node:24-slim`)
/// changes under the build without any change in the repository.
#[test]
fn every_image_is_pinned_by_digest() {
    let root = repository_root();
    let mut files: Vec<std::path::PathBuf> = ["Dockerfile", "Dockerfile.client", "Dockerfile.demo"]
        .iter()
        .map(|f| root.join(f))
        .collect();
    files.push(root.join("deploy/updater/Dockerfile"));
    for dir in ["devcontainer"] {
        for entry in std::fs::read_dir(root.join(dir)).expect("devcontainer exists") {
            let path = entry.expect("readable").path().join("Dockerfile");
            if path.is_file() {
                files.push(path);
            }
        }
    }
    let mut checked = 0;
    for path in &files {
        let text = std::fs::read_to_string(path).expect("readable Dockerfile");
        for line in text.lines().map(str::trim) {
            let image = if let Some(rest) = line.strip_prefix("FROM ") {
                rest.split_whitespace().next().unwrap_or("")
            } else if let Some(rest) = line.strip_prefix("ARG ")
                && let Some((name, value)) = rest.split_once('=')
                && name.ends_with("_IMAGE")
            {
                value.trim_matches('"')
            } else {
                continue;
            };
            if image == "scratch" || image.starts_with("${") {
                continue;
            }
            checked += 1;
            assert!(
                image.contains("@sha256:"),
                "{}: `{image}` is not pinned by digest",
                path.display()
            );
        }
    }
    for (name, workflow) in build_workflows() {
        for line in workflow.lines().map(str::trim) {
            let Some(image) = line
                .strip_prefix("image: ")
                .or_else(|| line.strip_prefix("container: "))
            else {
                continue;
            };
            if image.starts_with("${{") || !image.contains(':') && !image.contains('/') {
                continue; // a matrix expression or a plain matrix label such as `snapdog`
            }
            checked += 1;
            assert!(
                image.contains("@sha256:"),
                "{name}: `{image}` is not pinned by digest"
            );
        }
    }
    assert!(
        checked > 10,
        "the image scan found only {checked} references"
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

const RELEASE_PLEASE_WORKFLOW: &str = include_str!("../../.github/workflows/release-please.yml");

/// #529: a cancelled release-please run can leave a tag and a draft release without a build,
/// so the workflow must never cancel a running instance, and the safety net that reports such
/// a draft must stay.
#[test]
fn release_please_never_cancels_a_running_instance() {
    let settings: Vec<&str> = RELEASE_PLEASE_WORKFLOW
        .lines()
        .map(str::trim)
        .filter(|l| !l.starts_with('#') && l.starts_with("cancel-in-progress:"))
        .collect();
    assert_eq!(
        settings,
        ["cancel-in-progress: false"],
        "release-please.yml must set `cancel-in-progress: false` exactly once"
    );
    assert!(
        RELEASE_PLEASE_WORKFLOW.contains("Check that every draft release has a release build"),
        "the step that reports a draft release without a build is gone"
    );
}
