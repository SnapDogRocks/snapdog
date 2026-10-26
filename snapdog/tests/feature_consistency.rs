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

/// Reads a file relative to the workspace root (the parent of this crate).
fn workspace_file(relative: &str) -> String {
    let path = format!("{}/../{relative}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("failed to read {path}: {e}"))
}

/// The names a Cargo feature (`full`, `default`, ...) enables, from this crate's manifest.
fn manifest_feature(name: &str) -> BTreeSet<String> {
    let manifest: toml::Table =
        std::fs::read_to_string(format!("{}/Cargo.toml", env!("CARGO_MANIFEST_DIR")))
            .expect("failed to read Cargo.toml")
            .parse()
            .expect("failed to parse Cargo.toml");
    manifest["features"][name]
        .as_array()
        .unwrap_or_else(|| panic!("feature `{name}` must be an array"))
        .iter()
        .map(|v| v.as_str().expect("feature entries are strings").to_owned())
        .collect()
}

/// The feature names a shell line passes to Cargo (`--features a,b`, `--features=a`,
/// `-F a`). A trailing `# comment` and shell quoting are ignored, and the match is on
/// whole names, so `snapdog/full-x` does not count as `snapdog/full`.
fn cargo_features(line: &str) -> BTreeSet<String> {
    let code = line.split(" #").next().unwrap_or(line);
    let mut tokens = code.split_whitespace();
    let mut names = BTreeSet::new();
    while let Some(token) = tokens.next() {
        let value = if let Some(v) = token.strip_prefix("--features=") {
            v
        } else if token == "--features" || token == "-F" {
            match tokens.next() {
                Some(v) => v,
                None => break,
            }
        } else {
            continue;
        };
        for name in value
            .trim_matches(|c| c == '"' || c == '\'')
            .split(',')
            .filter(|n| !n.is_empty())
        {
            names.insert(name.to_owned());
        }
    }
    names
}

#[test]
fn cargo_features_parser_matches_whole_names_only() {
    let want = |line: &str| cargo_features(line).contains("snapdog/full");
    assert!(want("cargo build -p a -p b --features snapdog/full"));
    assert!(want("cargo build --features=snapdog/full -p a"));
    assert!(want("cargo build --features \"x,snapdog/full\" --locked"));
    assert!(want("cargo build -F snapdog/full"));
    assert!(want("cargo build --features snapdog/full # trailing"));
    assert!(!want("cargo build --features snapdog/full-x"));
    assert!(!want("cargo build --features snapdog/fullx,other"));
    assert!(!want("cargo build -p snapdog # --features snapdog/full"));
    assert!(!want("cargo build -p snapdog --features"));
    assert!(!want("cargo build --features snapdog/ap2 snapdog/full"));
}

/// `full` only matters if the release actually builds with it. Before this test
/// existed the release workflow and the Dockerfile ran a plain `cargo build`, so
/// released binaries silently lacked `AirPlay` 2 and the API console while the
/// README and the `full` comment promised them.
#[test]
fn release_workflow_builds_the_server_with_the_full_feature() {
    let workflow = workflow_build_lines(&workspace_file(".github/workflows/release.yml"));
    assert_eq!(
        workflow.len(),
        1,
        "expected exactly one `cargo build --release` of snapdog in release.yml, found: {workflow:?}"
    );
    assert!(
        cargo_features(&workflow[0]).contains("snapdog/full"),
        "release.yml must build with `--features snapdog/full` (qualified so that it names \
         the crate that owns the feature and never applies to snapdog-client), found: {}",
        workflow[0]
    );
}

#[test]
fn dockerfile_compile_path_builds_the_server_with_the_full_feature() {
    let dockerfile = workspace_file("Dockerfile");
    let build: Vec<&str> = dockerfile
        .lines()
        .filter(|l| l.contains("cargo build") && l.contains("-p snapdog"))
        .filter(|l| !l.contains("-p snapdog-client"))
        .collect();
    assert_eq!(
        build.len(),
        1,
        "expected exactly one server `cargo build` in the Dockerfile, found: {build:?}"
    );
    assert!(
        cargo_features(build[0]).contains("full"),
        "the Dockerfile's compile path must build with `--features full`, found: {}",
        build[0]
    );
}

fn workflow_build_lines(workflow: &str) -> Vec<String> {
    workflow
        .lines()
        .map(str::trim)
        .filter(|l| !l.starts_with('#'))
        .filter(|l| l.contains("cargo build --release") && l.contains("-p snapdog"))
        .map(str::to_owned)
        .collect()
}

/// The text of one job of a workflow file: from its `  <job>:` line to the next job.
fn workflow_job(workflow: &str, job: &str) -> String {
    let header = format!("  {job}:");
    let mut lines = workflow.lines();
    assert!(
        lines.any(|l| l == header),
        "workflow has no job `{job}` (looked for the line `{header}`)"
    );
    let mut body = vec![header];
    for line in lines {
        let starts_job = line.starts_with("  ") && !line.starts_with("   ") && line.ends_with(':');
        let top_level = !line.is_empty() && !line.starts_with(' ') && !line.starts_with('#');
        if starts_job || top_level {
            break;
        }
        body.push(line.to_owned());
    }
    body.join("\n")
}

#[test]
fn workflow_job_extracts_one_job() {
    let workflow = "jobs:\n  a:\n    run: one\n  b:\n    run: two\n    with:\n      x: y\n  c:\n    run: three\n";
    let b = workflow_job(workflow, "b");
    assert!(b.contains("two") && b.contains("x: y"));
    assert!(!b.contains("one") && !b.contains("three"));
}

/// Tests behind `ap2` and `api-docs` only compile with those features. The default test
/// runs never see them, so one CI step has to run the suite with the release feature set.
#[test]
fn ci_runs_the_server_tests_with_the_release_feature_set() {
    let ci = workspace_file(".github/workflows/ci.yml");
    let core = workflow_job(&ci, "rust-core");
    assert!(
        core.lines().any(|l| {
            let features = cargo_features(l);
            l.contains("cargo test -p snapdog")
                && !l.contains("--test ")
                && !l.contains("--lib")
                && (features.contains("full") || features.contains("snapdog/full"))
        }),
        "rust-core must run the whole `cargo test -p snapdog` suite with `--features full`"
    );
}

/// The path patterns of one `dorny/paths-filter` filter in ci.yml.
fn filter_paths(workflow: &str, filter: &str) -> Vec<String> {
    let header = format!("            {filter}:");
    let mut lines = workflow.lines();
    assert!(
        lines.any(|l| l == header),
        "ci.yml has no paths filter `{filter}`"
    );
    lines
        .map_while(|l| l.strip_prefix("              "))
        .filter_map(|l| l.strip_prefix("- "))
        .map(|p| p.trim_matches('\'').to_owned())
        .collect()
}

#[test]
fn filter_paths_reads_one_filter() {
    let ci = "            a:\n              - 'x/**'\n              # note\n              - 'y'\n            b:\n              - 'z'\n";
    assert_eq!(filter_paths(ci, "a"), ["x/**", "y"]);
    assert_eq!(filter_paths(ci, "b"), ["z"]);
}

/// The release feature set is checked on Windows and macOS only when the path filter says
/// the change can affect it. The server code that the `full` set compiles (HTTP API,
/// configuration, logging, receivers) has to trigger the jobs, not only the manifests.
#[test]
fn ci_path_filters_cover_the_server_code_the_release_check_compiles() {
    let ci = workspace_file(".github/workflows/ci.yml");
    for filter in ["windows", "macos_client"] {
        let paths = filter_paths(&ci, filter);
        for needed in [
            "snapdog/src/api/**",
            "snapdog/src/config/**",
            "snapdog/src/receiver/**",
            "snapdog/src/main.rs",
        ] {
            assert!(
                paths.iter().any(|p| p == needed),
                "the `{filter}` filter in ci.yml must include `{needed}`, found {paths:?}"
            );
        }
    }
}

/// The release builds the server for macOS (both architectures) and Windows with
/// `snapdog/full`. Nothing else compiles that set on those platforms, so CI has to,
/// or the first failure shows up while a release is being built.
#[test]
fn ci_checks_the_release_feature_set_on_windows_and_macos() {
    let ci = workspace_file(".github/workflows/ci.yml");

    let windows = workflow_job(&ci, "windows-check");
    assert!(
        windows
            .lines()
            .any(|l| cargo_features(l).contains("snapdog/full") && l.contains("cargo check")),
        "windows-check must run `cargo check` with `--features snapdog/full`"
    );

    let macos = workflow_job(&ci, "macos-client-check");
    assert!(
        macos
            .lines()
            .any(|l| cargo_features(l).contains("snapdog/full") && l.contains("cargo check")),
        "macos-client-check must run `cargo check` with `--features snapdog/full`"
    );
    let targets = ["aarch64-apple-darwin", "x86_64-apple-darwin"];
    assert!(
        macos
            .lines()
            .any(|l| l.contains("for target in") && targets.iter().all(|t| l.contains(t))),
        "macos-client-check must loop over both release targets {targets:?} when it checks"
    );
}

/// The exact feature set of a release. A change here changes what every released
/// binary does for every user, so it must not slip through as a side effect of a
/// feature edit: when this list changes, write the upgrade notes (docs/upgrading.md)
/// and mark the commit as breaking if behaviour changes (`feat!`/`fix!` plus a
/// `BREAKING CHANGE:` footer). `ap2` flipped the default receiver from `AirPlay` 1 to
/// `AirPlay` 2 for everyone that way once, which is why this list exists.
const RELEASE_FEATURES: &[&str] = &["ap2", "api-docs", "dbus", "snapcast-embedded", "spotify"];

#[test]
fn release_feature_set_is_pinned() {
    let full = manifest_feature("full");
    let expected: BTreeSet<String> = RELEASE_FEATURES.iter().map(|f| (*f).to_owned()).collect();
    assert_eq!(
        full, expected,
        "the `full` feature (what a release ships) changed. Update RELEASE_FEATURES, add or \
         amend the upgrade notes in docs/upgrading.md and mark the commit as breaking if \
         users see different behaviour"
    );
}

/// A feature that only a release build carries (in `full`, not in `default`) changes
/// behaviour for everyone who moves from a source build or an older release to a
/// release. Each needs a section in the upgrade notes.
#[test]
fn every_release_only_feature_has_an_upgrade_note() {
    let default = manifest_feature("default");
    let notes = workspace_file("docs/upgrading.md");
    let headings: Vec<&str> = notes.lines().filter(|l| l.starts_with('#')).collect();
    for feature in manifest_feature("full").difference(&default) {
        let marker = format!("`{feature}`");
        assert!(
            headings.iter().any(|h| h.contains(&marker)),
            "feature {marker} ships only in release builds but docs/upgrading.md has no \
             heading naming it; describe what changes for users and how to opt out"
        );
    }
}

/// The notes are only useful if they carry the way back and the network facts that
/// the `AirPlay` 2 switch brought with it.
#[test]
fn upgrade_notes_cover_the_airplay_2_switch() {
    let notes = workspace_file("docs/upgrading.md");
    for needle in [r#"mode = "airplay1""#, "_airplay._tcp", "UDP 319"] {
        assert!(
            notes.contains(needle),
            "docs/upgrading.md must mention `{needle}`"
        );
    }
}
