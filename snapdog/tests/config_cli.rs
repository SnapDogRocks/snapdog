// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

use std::process::Command;

#[test]
fn check_config_accepts_valid_configuration_without_starting_services() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("snapdog.toml");
    std::fs::write(
        &path,
        r#"
            [[zone]]
            name = "Living"
        "#,
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_snapdog"))
        .args(["--config", path.to_str().unwrap(), "--check-config"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        "Configuration is valid"
    );
}

#[test]
fn check_config_rejects_invalid_configuration() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("snapdog.toml");
    std::fs::write(
        &path,
        r#"
            [audio]
            sample_rate = 12345

            [[zone]]
            name = "Living"
        "#,
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_snapdog"))
        .args(["--config", path.to_str().unwrap(), "--check-config"])
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("Unsupported sample rate"));
}

fn run_check(config: &str, extra_args: &[&str]) -> std::process::Output {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("snapdog.toml");
    std::fs::write(&path, config).unwrap();
    Command::new(env!("CARGO_BIN_EXE_snapdog"))
        .args(["--config", path.to_str().unwrap(), "--check-config"])
        .args(extra_args)
        .output()
        .unwrap()
}

#[test]
fn check_config_prints_warnings_on_stderr_and_keeps_stdout_clean() {
    let output = run_check(
        r#"
            [snapcast]
            codec = "f32lz4e"
            streaming_prot = 1704

            [spotify]
            name = "Old"

            [[zone]]
            name = "Living"
        "#,
        &[],
    );

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        "Configuration is valid"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("warning: Unknown configuration key `snapcast.streaming_prot`"));
    assert!(stderr.contains("built-in default key"), "{stderr}");
    assert!(stderr.contains("[spotify].name is ignored"), "{stderr}");
}

#[test]
fn check_config_is_silent_for_a_clean_file() {
    let output = run_check("[[zone]]\nname = \"Living\"\n", &[]);
    assert!(output.status.success());
    assert!(!String::from_utf8_lossy(&output.stderr).contains("warning:"));
}
