// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! The console warning is said once, by the config notice, and not again by the router.
//!
//! Its own test binary with a single test: `tracing` caches, per call site, whether any
//! subscriber wants the event. With other tests running in the same process, a call site
//! that another test reached first (without a subscriber) stays silent for the capturing
//! subscriber here, and a repeated warning would go unnoticed.

#![cfg(feature = "api-docs")]

mod common;

use snapdog::api;
use snapdog::config;

/// Writes of the log output into a shared buffer.
#[derive(Clone, Default)]
struct Captured(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

impl std::io::Write for Captured {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[test]
fn the_router_does_not_repeat_the_startup_warning_about_the_console() {
    let toml = "[http]\napi_console = true\napi_keys = [\"s3cret\"]\n[[zone]]\nname = \"A\"\n";
    let app = common::build_test_app(config::load_str(toml).unwrap());
    let captured = Captured::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer({
            let captured = captured.clone();
            move || captured.clone()
        })
        .with_ansi(false)
        .finish();
    tracing::subscriber::with_default(subscriber, || {
        let _ = api::build_router(&app.state);
    });
    let output = String::from_utf8(captured.0.lock().unwrap().clone()).unwrap();
    // The capture works: the router logs that authentication is on.
    assert!(
        output.contains("API authentication enabled"),
        "capture is empty: {output}"
    );
    assert!(!output.contains("console"), "{output}");
    assert!(!output.contains("/docs"), "{output}");
}
