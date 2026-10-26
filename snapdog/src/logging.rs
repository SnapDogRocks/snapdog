// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! Log output: stdout always, plus an optional daily-rotated file (`[system].log_file`).
//!
//! The file keeps [`LOG_FILES_KEPT`] daily files. Day boundaries are UTC (that is how
//! `tracing-appender` rotates), so the date in a file name can differ from the local
//! date by up to a day. Write errors (a full disk) are swallowed by the background
//! writer of `tracing-appender`: stdout keeps working, the file silently loses lines.
//!
//! Old files are deleted by `SnapDog` itself, not by `tracing-appender`, whose own
//! retention selects files by name prefix and suffix and would also delete unrelated
//! files such as `snapdog-audit.log`. Only names of the exact form
//! `<stem>.<yyyy-mm-dd>[.<extension>]` are ever touched, the oldest date goes first
//! (not the oldest creation time, so copied or restored files sort correctly), and the
//! file of the current day is never deleted.

/// Number of daily log files kept: the current one and the 13 before it.
pub const LOG_FILES_KEPT: usize = 14;

/// How often the log directory is checked for files beyond [`LOG_FILES_KEPT`]. The
/// check also runs once when the file is opened.
const PRUNE_INTERVAL: Duration = Duration::from_secs(3600);

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use anyhow::{Context, Result};
use tracing::Subscriber;
use tracing_appender::non_blocking::{NonBlocking, WorkerGuard};
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

/// Keeps file logging running: the background writer of the log file and the
/// thread that deletes old files. Dropping it flushes the writer and stops the thread.
pub struct FileGuard {
    _writer: WorkerGuard,
    /// Dropping the sender wakes the pruning thread, which then ends.
    _stop_pruning: mpsc::Sender<()>,
}

/// A configured subscriber plus what has to stay alive with it.
pub struct LogSetup {
    /// The subscriber to install.
    pub subscriber: Box<dyn Subscriber + Send + Sync>,
    /// Keeps the background writer of the log file alive. Dropping it flushes
    /// and stops file logging, so hold it for the lifetime of the process.
    pub file_guard: Option<FileGuard>,
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
fn open_log_file(path: &str) -> Result<(NonBlocking, FileGuard)> {
    let (directory, stem, extension) = log_file_parts(path)?;

    let mut builder = RollingFileAppender::builder()
        .rotation(Rotation::DAILY)
        .filename_prefix(stem);
    if let Some(extension) = extension {
        builder = builder.filename_suffix(extension);
    }
    // One message, not a context chain: the I/O error is already part of the text.
    let appender = builder
        .build(directory)
        .map_err(|e| anyhow::anyhow!("cannot write log files to {}: {e}", directory.display()))?;
    let (writer, worker) = tracing_appender::non_blocking(appender);

    let stop = spawn_pruner(directory, stem, extension, PRUNE_INTERVAL);
    Ok((
        writer,
        FileGuard {
            _writer: worker,
            _stop_pruning: stop,
        },
    ))
}

/// Delete old daily files now and then every `interval`, until the returned sender
/// is dropped.
fn spawn_pruner(
    directory: &Path,
    stem: &str,
    extension: Option<&str>,
    interval: Duration,
) -> mpsc::Sender<()> {
    let (stop, stopped) = mpsc::channel::<()>();
    let (directory, stem, extension) = (
        directory.to_path_buf(),
        stem.to_string(),
        extension.map(str::to_string),
    );
    prune_daily_files(&directory, &stem, extension.as_deref(), LOG_FILES_KEPT);
    let spawned = std::thread::Builder::new()
        .name("log-pruner".into())
        .spawn(move || {
            // Timeout: time to check again. Disconnected: the guard was dropped.
            while stopped.recv_timeout(interval) == Err(mpsc::RecvTimeoutError::Timeout) {
                prune_daily_files(&directory, &stem, extension.as_deref(), LOG_FILES_KEPT);
            }
        });
    if let Err(error) = spawned {
        // Without the thread files are still pruned at every start.
        eprintln!("cannot start the log file pruning thread: {error}");
    }
    stop
}

/// The date in the name of a daily file of `stem` and `extension`
/// (`<stem>.<yyyy-mm-dd>[.<extension>]`), or `None` for any other name.
fn daily_file_date(name: &str, stem: &str, extension: Option<&str>) -> Option<chrono::NaiveDate> {
    let rest = name.strip_prefix(stem)?.strip_prefix('.')?;
    let date = match extension {
        Some(extension) => rest.strip_suffix(extension)?.strip_suffix('.')?,
        None => rest,
    };
    chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").ok()
}

/// Delete the oldest daily files of `stem` and `extension` in `directory` until `keep`
/// are left. Only regular files with an exact daily name are considered, ordered by the
/// date in the name; the file of the current UTC day is never deleted. Errors are
/// ignored: a file that cannot be removed is tried again next time.
fn prune_daily_files(directory: &Path, stem: &str, extension: Option<&str>, keep: usize) {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    let mut daily: Vec<(chrono::NaiveDate, PathBuf)> = entries
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            Some((daily_file_date(&name, stem, extension)?, e.path()))
        })
        .collect();
    daily.sort();
    let today = chrono::Utc::now().date_naive();
    let excess = daily.len().saturating_sub(keep);
    for (date, path) in daily.into_iter().take(excess) {
        if date != today {
            let _ = std::fs::remove_file(path);
        }
    }
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

    fn names(dir: &Path) -> Vec<String> {
        log_files(dir)
            .iter()
            .map(|f| f.file_name().unwrap().to_string_lossy().into_owned())
            .collect()
    }

    /// Dated files `snapdog.2020-01-NN.log` for NN in `days`, in the given order of creation.
    fn dated_files(dir: &Path, days: impl IntoIterator<Item = u32>) {
        for day in days {
            std::fs::write(dir.join(format!("snapdog.2020-01-{day:02}.log")), "old\n").unwrap();
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn keeps_fourteen_daily_files_and_deletes_the_oldest() {
        assert_eq!(LOG_FILES_KEPT, 14);
        let dir = tempfile::tempdir().unwrap();
        dated_files(dir.path(), 1..=20);
        let path = dir.path().join("snapdog.log");

        let setup = build("info", Some(path.to_str().unwrap())).unwrap();
        drop(setup.file_guard);

        let names = names(dir.path());
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
        dated_files(dir.path(), 1..=5);
        let path = dir.path().join("snapdog.log");
        drop(
            build("info", Some(path.to_str().unwrap()))
                .unwrap()
                .file_guard,
        );
        assert_eq!(log_files(dir.path()).len(), 6);
    }

    #[test]
    fn the_oldest_date_goes_first_whatever_the_creation_order() {
        // Restored from a backup in reverse: the newest day was created first.
        let dir = tempfile::tempdir().unwrap();
        dated_files(dir.path(), (1..=20).rev());
        prune_daily_files(dir.path(), "snapdog", Some("log"), 14);
        let names = names(dir.path());
        assert_eq!(names.len(), 14, "{names:?}");
        assert_eq!(names[0], "snapdog.2020-01-07.log", "{names:?}");
        assert_eq!(names[13], "snapdog.2020-01-20.log", "{names:?}");
    }

    #[test]
    fn files_that_are_not_daily_files_are_never_touched() {
        let dir = tempfile::tempdir().unwrap();
        // Older than every daily file by name and by creation time.
        let foreign = [
            "snapdog-audit.log",
            "snapdog-client.log",
            "snapdog.old.log",
            "snapdog.log",
            "snapdog.2020-01-01.log.gz",
            "snapdog.2020-01-01.txt",
            "snapdog.2020-13-40.log",
            "snapdog2020-01-01.log",
            "other.2020-01-01.log",
            "notes.txt",
        ];
        for name in foreign {
            std::fs::write(dir.path().join(name), "keep me").unwrap();
            std::thread::sleep(Duration::from_millis(5));
        }
        std::fs::create_dir(dir.path().join("snapdog.2020-01-02.log")).unwrap();
        dated_files(dir.path(), 3..=25);

        let path = dir.path().join("snapdog.log");
        drop(
            build("info", Some(path.to_str().unwrap()))
                .unwrap()
                .file_guard,
        );

        let left = names(dir.path());
        for name in foreign {
            assert!(
                left.contains(&name.to_string()),
                "{name} was deleted: {left:?}"
            );
        }
        assert!(
            dir.path().join("snapdog.2020-01-02.log").is_dir(),
            "a directory with a daily name is not a log file"
        );
        let daily = left
            .iter()
            .filter(|n| daily_file_date(n, "snapdog", Some("log")).is_some())
            .filter(|n| dir.path().join(n).is_file())
            .count();
        // 23 old daily files plus today's: exactly 14 stay (the directory is not counted).
        assert_eq!(daily, 14, "{left:?}");
    }

    #[test]
    fn the_file_of_the_current_day_is_never_deleted() {
        let dir = tempfile::tempdir().unwrap();
        let today = chrono::Utc::now().date_naive();
        let todays = format!("snapdog.{today}.log");
        std::fs::write(dir.path().join(&todays), "today").unwrap();
        // Files dated after today (a clock that went backwards) must not push it out.
        for ahead in 1..=3 {
            let day = today + chrono::Days::new(ahead);
            std::fs::write(dir.path().join(format!("snapdog.{day}.log")), "").unwrap();
        }
        prune_daily_files(dir.path(), "snapdog", Some("log"), 2);
        assert!(names(dir.path()).contains(&todays));
    }

    #[test]
    fn daily_file_names_are_recognised_exactly() {
        let date = |n: &str, e: Option<&str>| daily_file_date(n, "snapdog", e);
        assert_eq!(
            date("snapdog.2026-10-26.log", Some("log")),
            chrono::NaiveDate::from_ymd_opt(2026, 10, 26)
        );
        assert!(date("snapdog.2026-10-26", None).is_some());
        assert!(date("snapdog.log", Some("log")).is_none());
        assert!(date("snapdog-client.2026-10-26.log", Some("log")).is_none());
        assert!(date("snapdog.2026-10-26.txt", Some("log")).is_none());
        assert!(date("snapdog.old.log", Some("log")).is_none());
        assert!(date("snapdog.2026-13-40.log", Some("log")).is_none());
        // Without an extension the name is the date and nothing else.
        assert!(date("snapdog.2026-10-26.log", None).is_none());
    }

    #[test]
    fn old_files_are_also_removed_while_running() {
        let dir = tempfile::tempdir().unwrap();
        let stop = spawn_pruner(
            dir.path(),
            "snapdog",
            Some("log"),
            Duration::from_millis(50),
        );
        dated_files(dir.path(), 1..=16);
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while names(dir.path()).len() > 14 && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(names(dir.path()).len(), 14, "{:?}", names(dir.path()));
        // Dropping the guard ends the thread: later files stay.
        drop(stop);
        std::thread::sleep(Duration::from_millis(200));
        dated_files(dir.path(), 17..=18);
        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(names(dir.path()).len(), 16, "{:?}", names(dir.path()));
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
