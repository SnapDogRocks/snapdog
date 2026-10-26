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

/// A wrong `[mqtt]` section does not stop the server (MQTT is switched off), but it
/// must not pass `--check-config`: the check exists to find exactly this.
#[test]
fn check_config_flags_an_invalid_mqtt_section_and_names_the_key() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("snapdog.toml");
    std::fs::write(
        &path,
        r#"
            [mqtt]
            broker = "192.168.1.10:1883"
            password = "hunter2-secret"

            [[zone]]
            name = "Living"
        "#,
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_snapdog"))
        .args(["--config", path.to_str().unwrap(), "--check-config"])
        .env_remove("SNAPDOG_MQTT_USERNAME")
        .env_remove("SNAPDOG_MQTT_PASSWORD")
        .output()
        .unwrap();

    assert!(
        !output.status.success(),
        "an invalid [mqtt] must be flagged"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("[mqtt]") && stderr.contains("username"),
        "{stderr}"
    );
    assert!(stderr.contains("without MQTT"), "{stderr}");
    assert!(
        !stderr.contains("hunter2"),
        "the password must not appear: {stderr}"
    );
    assert!(!String::from_utf8_lossy(&output.stdout).contains("Configuration is valid"));
}

#[test]
fn check_config_accepts_a_valid_mqtt_section() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("snapdog.toml");
    std::fs::write(
        &path,
        r#"
            [mqtt]
            broker = "192.168.1.10:1883"
            username = "snapdog"
            password = "pw"

            [[zone]]
            name = "Living"
        "#,
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_snapdog"))
        .args(["--config", path.to_str().unwrap(), "--check-config"])
        .env_remove("SNAPDOG_MQTT_USERNAME")
        .env_remove("SNAPDOG_MQTT_PASSWORD")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// A value of the wrong type in `[mqtt]` is reported like any other wrong `[mqtt]`
/// setting: an error and a non-zero exit from `--check-config` that names the key,
/// not a TOML parse error that also would stop the server.
#[test]
fn check_config_flags_a_wrong_type_in_mqtt_and_names_the_key() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("snapdog.toml");
    std::fs::write(
        &path,
        "[mqtt]\nbroker = \"b:1883\"\n[mqtt.homeassistant]\ndiscovery = \"yes\"\n\n[[zone]]\nname = \"Living\"\n",
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_snapdog"))
        .args(["--config", path.to_str().unwrap(), "--check-config"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Invalid [mqtt] section"), "{stderr}");
    assert!(
        stderr.contains("`discovery`") && stderr.contains("boolean"),
        "{stderr}"
    );
    assert!(stderr.contains("without MQTT"), "{stderr}");
}
