// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! Log output: stdout always, plus an optional daily-rotated file (`[system].log_file`).
//!
//! The file keeps [`LOG_FILES_KEPT`] daily files. Day boundaries are UTC (that is how
//! `tracing-appender` rotates), so the date in a file name can differ from the local
//! date by up to a day. Write errors (a full disk) are swallowed by the background
//! writer of `tracing-appender`: stdout keeps working, the file silently loses lines.

/// Number of daily log files kept: the current one and the 13 before it.
/// Older files are deleted when a new day's file is created.
pub const LOG_FILES_KEPT: usize = 14;

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
    /// Warnings about the log directory, to log once the subscriber is installed.
    pub file_warnings: Vec<String>,
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

    let file_warnings = log_file.map(retention_warnings).unwrap_or_default();
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
        file_warnings,
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

/// Where the daily files of `log_file` go: directory, file stem and extension.
fn log_file_parts(path: &str) -> Result<(&Path, &str, Option<&str>)> {
    let path = Path::new(path);
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .with_context(|| format!("log_file {} does not name a file", path.display()))?;
    let directory = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let extension = path.extension().and_then(|e| e.to_str());
    Ok((directory, stem, extension))
}

/// Open `path` as a daily-rotated, non-blocking log file that keeps
/// [`LOG_FILES_KEPT`] files. The date goes between the file stem and the
/// extension: `snapdog.log` becomes `snapdog.2026-10-25.log`.
fn open_log_file(path: &str) -> Result<(NonBlocking, WorkerGuard)> {
    let (directory, stem, extension) = log_file_parts(path)?;

    let mut builder = RollingFileAppender::builder()
        .rotation(Rotation::DAILY)
        .max_log_files(LOG_FILES_KEPT)
        .filename_prefix(stem);
    if let Some(extension) = extension {
        builder = builder.filename_suffix(extension);
    }
    // One message, not a context chain: the I/O error is already part of the text.
    let appender = builder
        .build(directory)
        .map_err(|e| anyhow::anyhow!("cannot write log files to {}: {e}", directory.display()))?;
    Ok(tracing_appender::non_blocking(appender))
}

/// Whether `name` is one of the daily files of `stem` and `extension`
/// (`<stem>.<yyyy-mm-dd>[.<extension>]`).
fn is_daily_file(name: &str, stem: &str, extension: Option<&str>) -> bool {
    let Some(rest) = name.strip_prefix(stem).and_then(|r| r.strip_prefix('.')) else {
        return false;
    };
    let date = extension.map_or(Some(rest), |extension| {
        rest.strip_suffix(extension)
            .and_then(|r| r.strip_suffix('.'))
    });
    date.is_some_and(|d| chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d").is_ok())
}

/// `tracing-appender` deletes by file-name prefix and suffix only, not by the
/// exact daily pattern: with `log_file = "/var/log/snapdog.log"` it would also
/// count (and, beyond [`LOG_FILES_KEPT`], delete) `snapdog-client.log`. Name the
/// files in the directory that retention would touch without being `SnapDog`'s own.
fn retention_warnings(path: &str) -> Vec<String> {
    let Ok((directory, stem, extension)) = log_file_parts(path) else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(directory) else {
        return Vec::new();
    };
    let mut others: Vec<String> = entries
        .filter_map(Result::ok)
        .filter(|e| e.metadata().is_ok_and(|m| m.is_file()))
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|name| {
            name.starts_with(stem)
                && extension.is_none_or(|x| name.ends_with(x))
                && !is_daily_file(name, stem, extension)
        })
        .collect();
    if others.is_empty() {
        return Vec::new();
    }
    others.sort();
    vec![format!(
        "log_file retention keeps {LOG_FILES_KEPT} files and deletes older ones whose names \
         start with \"{stem}\"{}. These files in {} are not SnapDog's daily files but match \
         that rule and can be deleted: {}. Give SnapDog a log directory of its own.",
        extension.map_or_else(String::new, |x| format!(" and end with \"{x}\"")),
        directory.display(),
        others.join(", ")
    )]
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

    /// `count` dated files `<stem>.2020-01-NN.log`, created oldest first.
    fn old_files(dir: &Path, count: u32) {
        for day in 1..=count {
            std::fs::write(dir.join(format!("snapdog.2020-01-{day:02}.log")), "old\n").unwrap();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    #[test]
    fn keeps_fourteen_daily_files_and_deletes_the_oldest() {
        assert_eq!(LOG_FILES_KEPT, 14);
        let dir = tempfile::tempdir().unwrap();
        old_files(dir.path(), 20);
        let path = dir.path().join("snapdog.log");

        let setup = build("info", Some(path.to_str().unwrap())).unwrap();
        drop(setup.file_guard);

        let names: Vec<String> = log_files(dir.path())
            .iter()
            .map(|f| f.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names.len(), 14, "{names:?}");
        assert!(
            !names.contains(&"snapdog.2020-01-07.log".to_string()),
            "{names:?}"
        );
        assert!(
            names.contains(&"snapdog.2020-01-08.log".to_string()),
            "{names:?}"
        );
        assert!(
            names.contains(&"snapdog.2020-01-20.log".to_string()),
            "{names:?}"
        );
    }

    #[test]
    fn fewer_than_the_limit_are_all_kept() {
        let dir = tempfile::tempdir().unwrap();
        old_files(dir.path(), 5);
        let path = dir.path().join("snapdog.log");
        drop(
            build("info", Some(path.to_str().unwrap()))
                .unwrap()
                .file_guard,
        );
        assert_eq!(log_files(dir.path()).len(), 6);
    }

    #[test]
    fn daily_file_names_are_recognised_exactly() {
        assert!(is_daily_file(
            "snapdog.2026-10-26.log",
            "snapdog",
            Some("log")
        ));
        assert!(is_daily_file("snapdog.2026-10-26", "snapdog", None));
        assert!(!is_daily_file("snapdog.log", "snapdog", Some("log")));
        assert!(!is_daily_file(
            "snapdog-client.2026-10-26.log",
            "snapdog",
            Some("log")
        ));
        assert!(!is_daily_file(
            "snapdog.2026-10-26.txt",
            "snapdog",
            Some("log")
        ));
        assert!(!is_daily_file("snapdog.old.log", "snapdog", Some("log")));
        assert!(!is_daily_file(
            "snapdog.2026-13-40.log",
            "snapdog",
            Some("log")
        ));
    }

    #[test]
    fn foreign_files_that_retention_would_delete_are_named() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("snapdog.log");
        let path = path.to_str().unwrap();
        assert_eq!(retention_warnings(path), Vec::<String>::new());

        std::fs::write(dir.path().join("snapdog.2026-10-26.log"), "").unwrap();
        std::fs::write(dir.path().join("unrelated.log"), "").unwrap();
        std::fs::write(dir.path().join("snapdog.conf"), "").unwrap();
        assert_eq!(retention_warnings(path), Vec::<String>::new());

        std::fs::write(dir.path().join("snapdog-client.log"), "").unwrap();
        std::fs::write(dir.path().join("snapdog.old.log"), "").unwrap();
        let warnings = retention_warnings(path);
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(
            warnings[0].contains("snapdog-client.log, snapdog.old.log"),
            "{warnings:?}"
        );
        assert!(warnings[0].contains("keeps 14 files"), "{warnings:?}");

        let setup = build("info", Some(path)).unwrap();
        assert_eq!(setup.file_warnings, warnings);
        drop(setup.file_guard);
    }

    #[test]
    fn the_open_error_names_the_cause_once() {
        let dir = tempfile::tempdir().unwrap();
        let blocker = dir.path().join("not-a-directory");
        std::fs::write(&blocker, "x").unwrap();
        let path = blocker.join("snapdog.log");
        let error = build("info", Some(path.to_str().unwrap()))
            .unwrap()
            .file_error
            .unwrap();
        // "...: Not a directory (os error 20)" once, not repeated by a cause chain.
        assert_eq!(error.matches("os error").count(), 1, "{error}");
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
