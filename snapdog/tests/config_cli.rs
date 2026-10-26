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

#[test]
fn knx_device_flag_warns_when_it_replaces_the_configured_address() {
    let config = r#"
        [knx]
        role = "device"
        individual_address = "1.1.100"

        [[zone]]
        name = "Living"
    "#;

    let replaced = run_check(config, &["--knx-device"]);
    assert!(replaced.status.success());
    let stderr = String::from_utf8_lossy(&replaced.stderr);
    assert!(
        stderr.contains("replaces [knx].individual_address = 1.1.100")
            && stderr.contains("15.15.255"),
        "{stderr}"
    );

    let explicit_same = run_check(config, &["--knx-device", "--knx-address", "1.1.100"]);
    assert!(!String::from_utf8_lossy(&explicit_same.stderr).contains("replaces"));

    let explicit_other = run_check(config, &["--knx-device", "--knx-address", "1.1.200"]);
    assert!(String::from_utf8_lossy(&explicit_other.stderr).contains("with 1.1.200"));
}

#[test]
fn command_line_overrides_are_validated_like_file_values() {
    let output = run_check("[[zone]]\nname = \"Living\"\n", &["--sample-rate", "12345"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("Unsupported sample rate: 12345"));

    let output = run_check(
        "[[zone]]\nname = \"Living\"\n",
        &["--streaming-port", "5555"],
    );
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("must not be the same"));
}

#[test]
fn help_describes_codec_and_bind_as_the_code_behaves() {
    let output = Command::new(env!("CARGO_BIN_EXE_snapdog"))
        .arg("--help")
        .output()
        .unwrap();
    let help = String::from_utf8_lossy(&output.stdout);
    assert!(help.contains("default: ::"), "{help}");
    assert!(!help.contains("default: 0.0.0.0"), "{help}");
    assert!(help.contains("Snapcast stream codec"), "{help}");
    assert!(help.contains("--check-config"));
    assert!(help.contains("--knx-sync-system-clock"));
}

#[test]
fn codec_override_gets_the_same_key_warnings_as_the_file() {
    let zone = "[[zone]]\nname = \"Living\"\n";

    // f32lz4e only on the command line: the default key is public.
    let output = run_check(zone, &["--codec", "f32lz4e"]);
    assert!(output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("built-in default key"), "{stderr}");

    // The file asks for f32lz4e without a key, the command line switches it off:
    // the warning about the file's codec no longer applies.
    let file = format!("[snapcast]\ncodec = \"f32lz4e\"\n{zone}");
    let stderr = String::from_utf8_lossy(&run_check(&file, &[]).stderr).into_owned();
    assert!(stderr.contains("built-in default key"), "{stderr}");
    let stderr =
        String::from_utf8_lossy(&run_check(&file, &["--codec", "pcm"]).stderr).into_owned();
    assert!(!stderr.contains("built-in default key"), "{stderr}");

    // A key in the file with a codec that does not encrypt, chosen on the command line.
    let keyed = format!("[snapcast]\ncodec = \"f32lz4e\"\nencryption_psk = \"s3cret\"\n{zone}");
    let stderr = String::from_utf8_lossy(&run_check(&keyed, &[]).stderr).into_owned();
    assert!(!stderr.contains("does not encrypt"), "{stderr}");
    let stderr =
        String::from_utf8_lossy(&run_check(&keyed, &["--codec", "flac"]).stderr).into_owned();
    assert!(stderr.contains("does not encrypt"), "{stderr}");
}

#[test]
fn check_config_notes_the_open_streaming_port_and_stops_when_it_is_bound() {
    let zone = "[[zone]]\nname = \"Living\"\n";

    let output = run_check(zone, &[]);
    assert!(output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("note: Snapcast streaming port 1704 listens on all interfaces"),
        "{stderr}"
    );
    assert!(stderr.contains("no authentication"), "{stderr}");

    let bound = format!("[snapcast]\nstreaming_bind = \"127.0.0.1\"\n{zone}");
    let output = run_check(&bound, &[]);
    assert!(output.status.success());
    assert!(!String::from_utf8_lossy(&output.stderr).contains("streaming port"));
}

#[test]
fn check_config_rejects_a_streaming_bind_that_is_not_an_address() {
    let output = run_check(
        "[snapcast]\nstreaming_bind = \"not-an-address\"\n[[zone]]\nname = \"Living\"\n",
        &[],
    );
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("streaming_bind"), "{stderr}");
}

#[test]
fn check_config_drops_the_float_codec_note_when_the_command_line_changes_the_codec() {
    let file =
        "[snapcast]\ncodec = \"f32lz4e\"\nencryption_psk = \"k\"\n[[zone]]\nname = \"Living\"\n";
    let stderr = String::from_utf8_lossy(&run_check(file, &[]).stderr).into_owned();
    assert!(stderr.contains("native 32-bit float"), "{stderr}");
    for codec in ["flac", "pcm"] {
        let stderr =
            String::from_utf8_lossy(&run_check(file, &["--codec", codec]).stderr).into_owned();
        assert!(!stderr.contains("native 32-bit float"), "{codec}: {stderr}");
    }
}
