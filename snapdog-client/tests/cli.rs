// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! Command-line behaviour of the real binary: flags that cannot work stop it
//! before it touches the network or the audio device.

use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

/// A client that should have refused to start but did not would connect, daemonise or
/// wait for audio, and a test that waits for it would hang the whole run. So the client
/// gets this long, then it is killed and the test fails with what it printed.
const LIMIT: Duration = Duration::from_secs(30);

/// Run the client with `args`. Output goes to files, not pipes: a daemonised child that
/// outlived its parent would hold a pipe open and block the reader forever.
fn run(args: &[&str]) -> Output {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let id = format!(
        "snapdog-client-cli-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    );
    let dir = std::env::temp_dir();
    let (out_path, err_path) = (dir.join(format!("{id}.out")), dir.join(format!("{id}.err")));
    let mut child = Command::new(env!("CARGO_BIN_EXE_snapdog-client"))
        .args(args)
        .stdin(Stdio::null())
        .stdout(std::fs::File::create(&out_path).unwrap())
        .stderr(std::fs::File::create(&err_path).unwrap())
        .spawn()
        .expect("snapdog-client starts");

    let deadline = Instant::now() + LIMIT;
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break Some(status);
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            break None;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let stdout = std::fs::read(&out_path).unwrap_or_default();
    let stderr = std::fs::read(&err_path).unwrap_or_default();
    let _ = std::fs::remove_file(&out_path);
    let _ = std::fs::remove_file(&err_path);
    let Some(status) = status else {
        panic!(
            "snapdog-client {args:?} still running after {LIMIT:?}; it was killed.\nstderr: {}",
            String::from_utf8_lossy(&stderr)
        );
    };
    Output {
        status,
        stdout,
        stderr,
    }
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
    // No --daemon here: a client that wrongly accepted --user would fork into the
    // background and keep running. The combination is covered in cli.rs unit tests.
    let output = run(&["--user", "nobody", "tcp://127.0.0.1:1"]);
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
