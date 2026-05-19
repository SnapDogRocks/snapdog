//! Logging setup — wires `--logsink` and `--logfilter` into tracing.
//!
//! Sink options: `stdout`, `stderr`, `null`, `system`, `file:<path>`
//! Filter format: `<tag>:<level>[,<tag>:<level>]*`
//!   - tag: `*` (all) or a module name like `Stream`, `Controller`
//!   - level: trace, debug, info, notice, warning, error, fatal

use anyhow::{Result, bail};
use tracing_subscriber::EnvFilter;

/// Convert snapcast-style log filter to tracing `EnvFilter` syntax.
///
/// Snapcast: `*:info,Stream:debug` → tracing: `info,snapdog_client::stream=debug`
fn convert_filter(filter: &str) -> String {
    filter
        .split(',')
        .map(|part| {
            let (tag, level) = part.split_once(':').unwrap_or((part, "info"));

            let level = match level {
                "fatal" => "error",
                "warning" => "warn",
                "notice" => "info",
                other => other,
            };

            if tag == "*" {
                level.to_string()
            } else {
                let tag_lower = tag.to_lowercase();
                let module = match tag_lower.as_str() {
                    "stream" | "stats" | "latency" => "snapdog_client::stream",
                    "controller" => "snapdog_client::controller",
                    "connection" => "snapdog_client::connection",
                    "timeprovider" => "snapdog_client::time_provider",
                    "player" | "coreaudioplayer" | "alsaplayer" | "pulseplayer" => {
                        "snapdog_client::player"
                    }
                    "flac" | "flacdecoder" => "snapdog_client::decoder::flac",
                    "opus" | "opusdecoder" => "snapdog_client::decoder::opus",
                    "ogg" | "oggdecoder" => "snapdog_client::decoder::vorbis",
                    other => other,
                };
                format!("{module}={level}")
            }
        })
        .collect::<Vec<_>>()
        .join(",")
}

/// Initialize logging from CLI options.
///
/// Falls back to `RUST_LOG` env var if set, otherwise uses the provided filter.
pub fn init(sink: &str, filter: &str) -> Result<()> {
    let env_filter = if std::env::var("RUST_LOG").is_ok() {
        EnvFilter::from_default_env()
    } else {
        EnvFilter::try_new(convert_filter(filter)).unwrap_or_else(|_| EnvFilter::new("info"))
    };

    match sink {
        "null" => {
            // No output
            tracing_subscriber::fmt()
                .with_env_filter(env_filter)
                .with_writer(std::io::sink)
                .init();
        }
        "stdout" | "" => {
            tracing_subscriber::fmt()
                .with_env_filter(env_filter)
                .with_writer(std::io::stdout)
                .init();
        }
        "stderr" => {
            tracing_subscriber::fmt()
                .with_env_filter(env_filter)
                .with_writer(std::io::stderr)
                .init();
        }
        "system" => {
            // On Unix, "system" means syslog. tracing doesn't have native syslog,
            // so we fall back to stderr with a note.
            tracing_subscriber::fmt()
                .with_env_filter(env_filter)
                .with_writer(std::io::stderr)
                .init();
            tracing::debug!("Log sink 'system' mapped to stderr (syslog not available)");
        }
        s if s.starts_with("file:") => {
            let path = &s[5..];
            let file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .map_err(|e| anyhow::anyhow!("failed to open log file {path}: {e}"))?;
            tracing_subscriber::fmt()
                .with_env_filter(env_filter)
                .with_writer(file)
                .with_ansi(false)
                .init();
        }
        other => {
            bail!("invalid log sink: {other} (expected stdout|stderr|null|system|file:<path>)")
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn convert_default_filter() {
        assert_eq!(convert_filter("*:info"), "info");
    }

    #[test]
    fn convert_multi_filter() {
        let result = convert_filter("*:info,Stream:debug");
        assert_eq!(result, "info,snapdog_client::stream=debug");
    }

    #[test]
    fn convert_warning_to_warn() {
        assert_eq!(convert_filter("*:warning"), "warn");
    }

    #[test]
    fn convert_fatal_to_error() {
        assert_eq!(convert_filter("*:fatal"), "error");
    }

    #[test]
    fn convert_notice_to_info() {
        assert_eq!(convert_filter("*:notice"), "info");
    }

    #[test]
    fn convert_controller_tag() {
        let result = convert_filter("Controller:trace");
        assert_eq!(result, "snapdog_client::controller=trace");
    }
}
