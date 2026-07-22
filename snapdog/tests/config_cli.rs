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
