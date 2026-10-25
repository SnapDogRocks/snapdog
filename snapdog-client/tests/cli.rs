// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! Command-line behaviour of the real binary: flags that cannot work stop it
//! before it touches the network or the audio device.

use std::process::{Command, Output};

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_snapdog-client"))
        .args(args)
        .output()
        .expect("snapdog-client starts")
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn websocket_urls_fail_at_startup_with_a_clear_message() {
    for url in ["ws://127.0.0.1", "wss://127.0.0.1:1788"] {
        let output = run(&[url]);
        assert!(!output.status.success(), "{url} must be refused");
        let text = stderr(&output);
        assert!(
            text.contains("is not supported") && text.contains("tcp://"),
            "{text}"
        );
    }
}

#[cfg(unix)]
#[test]
fn user_flag_is_refused_instead_of_running_with_the_wrong_privileges() {
    let output = run(&["--daemon", "--user", "nobody", "tcp://127.0.0.1:1"]);
    assert!(!output.status.success());
    let text = stderr(&output);
    assert!(text.contains("--user nobody is not implemented"), "{text}");
}

#[test]
fn unknown_soundcard_fails_before_connecting() {
    // Port 1 would refuse the connection; the error has to come from the device check.
    let output = run(&["--soundcard", "no-such-device-0xdead", "tcp://127.0.0.1:1"]);
    assert!(!output.status.success());
    let text = stderr(&output);
    assert!(text.contains("--soundcard no-such-device-0xdead"), "{text}");
}

#[test]
fn help_lists_working_flags_and_hides_the_stubs() {
    let output = run(&["--help"]);
    assert!(output.status.success());
    let help = String::from_utf8_lossy(&output.stdout);
    for shown in [
        "--soundcard",
        "--list",
        "--player",
        "--mixer-sync",
        "--encryption-psk",
    ] {
        assert!(help.contains(shown), "{shown} missing:\n{help}");
    }
    for hidden in [
        "--sampleformat",
        "--server-cert",
        "--key-password",
        "--cert-key",
    ] {
        assert!(!help.contains(hidden), "{hidden} should be hidden:\n{help}");
    }
    assert!(!help.contains("<tcp|ws|wss>"), "{help}");
}
