// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! Log output: stdout always, plus an optional daily-rotated file (`[system].log_file`).

use std::path::Path;

use anyhow::{Context, Result};
use tracing::Subscriber;
use tracing_appender::non_blocking::{NonBlocking, WorkerGuard};
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

/// A configured subscriber plus what has to stay alive with it.
pub struct LogSetup {
    /// The subscriber to install.
    pub subscriber: Box<dyn Subscriber + Send + Sync>,
    /// Keeps the background writer of the log file alive. Dropping it flushes
    /// and stops file logging, so hold it for the lifetime of the process.
    pub file_guard: Option<WorkerGuard>,
    /// Set when `log_file` was configured but could not be opened. Log it as a
    /// warning once the subscriber is installed.
    pub file_error: Option<String>,
}

/// Build the subscriber for `level` (the `snapdog` target; `RUST_LOG` can
/// add further directives) and the optional `log_file`.
///
/// # Errors
///
/// Returns an error if `level` is not a valid filter directive.
pub fn build(level: &str, log_file: Option<&str>) -> Result<LogSetup> {
    let filter = EnvFilter::from_default_env().add_directive(
        format!("snapdog={level}")
            .parse()
            .with_context(|| format!("invalid log level {level:?}"))?,
    );

    let (file_writer, file_guard, file_error) = match log_file.map(open_log_file) {
        Some(Ok((writer, guard))) => (Some(writer), Some(guard), None),
        Some(Err(error)) => (None, None, Some(format!("{error:#}"))),
        None => (None, None, None),
    };

    let stdout = tracing_subscriber::fmt::layer();
    let file = file_writer.map(|writer| {
        tracing_subscriber::fmt::layer()
            .with_ansi(false)
            .with_writer(writer)
    });
    let subscriber = tracing_subscriber::registry()
        .with(filter)
        .with(stdout)
        .with(file);

    Ok(LogSetup {
        subscriber: Box::new(subscriber),
        file_guard,
        file_error,
    })
}

/// Install the subscriber as the global default and bridge the `log` crate
/// (used by the `AirPlay` and Spotify libraries) into it.
///
/// # Errors
///
/// Returns an error if a global subscriber is already installed.
pub fn install(subscriber: Box<dyn Subscriber + Send + Sync>) -> Result<()> {
    subscriber
        .try_init()
        .context("logging is already initialised")
}

/// Open `path` as a daily-rotated, non-blocking log file. The date goes between
/// the file stem and the extension: `snapdog.log` becomes `snapdog.2026-10-25.log`.
fn open_log_file(path: &str) -> Result<(NonBlocking, WorkerGuard)> {
    let path = Path::new(path);
    let name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .with_context(|| format!("log_file {} does not name a file", path.display()))?;
    let directory = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));

    let mut builder = RollingFileAppender::builder()
        .rotation(Rotation::DAILY)
        .filename_prefix(name);
    if let Some(extension) = path.extension().and_then(|e| e.to_str()) {
        builder = builder.filename_suffix(extension);
    }
    let appender = builder
        .build(directory)
        .with_context(|| format!("cannot write log files to {}", directory.display()))?;
    Ok(tracing_appender::non_blocking(appender))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn log_files(dir: &Path) -> Vec<std::path::PathBuf> {
        let mut files: Vec<_> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        files.sort();
        files
    }

    #[test]
    fn writes_to_a_dated_file_and_stays_filtered() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("logs").join("snapdog.log");

        let setup = build("info", Some(path.to_str().unwrap())).unwrap();
        assert!(setup.file_error.is_none());
        tracing::subscriber::with_default(setup.subscriber, || {
            tracing::info!(target: "snapdog::test", "hello file");
            tracing::debug!(target: "snapdog::test", "below the level");
        });
        drop(setup.file_guard); // flushes

        let files = log_files(&dir.path().join("logs"));
        assert_eq!(files.len(), 1, "{files:?}");
        let name = files[0].file_name().unwrap().to_str().unwrap();
        let date = name
            .strip_prefix("snapdog.")
            .and_then(|rest| rest.strip_suffix(".log"))
            .unwrap_or_else(|| panic!("snapdog.<date>.log expected, got {name}"));
        assert!(
            chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").is_ok(),
            "daily name expected, got {name}"
        );
        let text = std::fs::read_to_string(&files[0]).unwrap();
        assert!(text.contains("hello file"), "{text}");
        assert!(!text.contains("below the level"), "{text}");
        assert!(
            !text.contains('\u{1b}'),
            "no ANSI colour codes in a file: {text:?}"
        );
    }

    #[test]
    fn without_a_log_file_nothing_is_created() {
        let setup = build("info", None).unwrap();
        assert!(setup.file_guard.is_none());
        assert!(setup.file_error.is_none());
    }

    #[test]
    fn an_unusable_path_is_reported_not_swallowed() {
        let dir = tempfile::tempdir().unwrap();
        let blocker = dir.path().join("not-a-directory");
        std::fs::write(&blocker, "x").unwrap();
        let path = blocker.join("snapdog.log");

        let setup = build("info", Some(path.to_str().unwrap())).unwrap();
        assert!(setup.file_guard.is_none());
        let error = setup.file_error.expect("error is reported");
        assert!(error.contains("cannot write log files"), "{error}");
    }

    #[test]
    fn a_directory_path_is_reported() {
        let setup = build("info", Some("/")).unwrap();
        assert!(
            setup
                .file_error
                .is_some_and(|e| e.contains("does not name a file"))
        );
    }

    #[test]
    fn an_invalid_level_is_an_error() {
        assert!(build("loud", None).is_err());
    }
}
