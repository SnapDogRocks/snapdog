// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! Child process management for snapserver.
//!
//! Generates snapserver.conf from app config, spawns and monitors the process.
//! In dev mode (managed=false), this is a no-op.

use std::fmt::Write;
use std::path::PathBuf;

use anyhow::{Context, Result};
use tokio::process::{Child, Command};

use crate::config::AppConfig;

/// Handle to a managed snapserver process. Kills the process on drop.
pub struct SnapserverHandle {
    child: Option<Child>,
    config_path: PathBuf,
}

impl SnapserverHandle {
    /// Start snapserver if managed=true, otherwise return a no-op handle.
    pub fn start(config: &AppConfig) -> Result<Self> {
        if !config.snapcast.managed {
            tracing::info!("Snapserver not managed — skipping");
            return Ok(Self {
                child: None,
                config_path: PathBuf::new(),
            });
        }

        let config_path = generate_config(config)?;
        tracing::info!(path = %config_path.display(), "Snapserver config generated");

        let stdio = if config.snapcast.verbose {
            std::process::Stdio::inherit
        } else {
            std::process::Stdio::null
        };

        let child = Command::new("snapserver")
            .arg("-c")
            .arg(&config_path)
            .stdout(stdio())
            .stderr(stdio())
            .kill_on_drop(true)
            .spawn()
            .context("Failed to start snapserver — is it installed?")?;

        tracing::info!(pid = child.id().unwrap_or(0), "Snapserver started");

        Ok(Self {
            child: Some(child),
            config_path,
        })
    }

    /// Gracefully stop the snapserver process.
    pub async fn stop(&mut self) -> Result<()> {
        if let Some(ref mut child) = self.child {
            tracing::info!("Stopping snapserver");
            child.kill().await.context("Failed to kill snapserver")?;
            self.child = None;
        }
        // Clean up generated config
        if self.config_path.exists() {
            let _ = std::fs::remove_file(&self.config_path);
        }
        Ok(())
    }
}

impl Drop for SnapserverHandle {
    fn drop(&mut self) {
        if self.config_path.exists() {
            let _ = std::fs::remove_file(&self.config_path);
        }
    }
}

/// Generate snapserver.conf from app config. Returns path to the generated file.
fn generate_config(config: &AppConfig) -> Result<PathBuf> {
    let path = std::env::temp_dir().join(format!(
        "snapdog-snapserver-{}-{}.conf",
        std::process::id(),
        config.snapcast.streaming_port
    ));
    let content = render_config(config);
    std::fs::write(&path, &content)
        .with_context(|| format!("Failed to write {}", path.display()))?;
    Ok(path)
}

fn render_config(config: &AppConfig) -> String {
    let mut out = String::new();

    // HTTP / JSON-RPC (WebSocket) — not used by SnapDog, disable or use separate port
    out.push_str("[http]\nenabled = false\n\n");

    // TCP control (raw JSON-RPC — used by SnapDog)
    write!(
        out,
        "[tcp-control]\nenabled = true\nport = {}\n\n",
        config.snapcast.jsonrpc_port
    )
    .unwrap();
    // Streaming server
    write!(
        out,
        "[tcp-streaming]\nport = {}\n\n",
        config.snapcast.streaming_port
    )
    .unwrap();

    // TCP sources — one per zone
    out.push_str("[stream]\n");
    for zone in &config.zones {
        let sf = config.audio.sample_format();
        writeln!(
            out,
            "source = tcp://127.0.0.1:{}?name={}&sampleformat={}&mode=server",
            zone.tcp_source_port, zone.stream_name, sf
        )
        .unwrap();
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config;

    fn test_config() -> AppConfig {
        let raw: config::FileConfig = toml::from_str(
            r#"
            [[zone]]
            name = "Ground Floor"
            [[zone]]
            name = "1st Floor"
            [[client]]
            name = "X"
            mac = "00:00:00:00:00:00"
            zone = "Ground Floor"
        "#,
        )
        .unwrap();
        config::load_raw(raw).unwrap()
    }

    #[test]
    fn generates_correct_sources() {
        let conf = render_config(&test_config());
        assert!(conf.contains(
            "source = tcp://127.0.0.1:4953?name=Zone1&sampleformat=48000:16:2&mode=server"
        ));
        assert!(conf.contains(
            "source = tcp://127.0.0.1:4954?name=Zone2&sampleformat=48000:16:2&mode=server"
        ));
    }

    #[test]
    fn disables_http() {
        let conf = render_config(&test_config());
        assert!(conf.contains("[http]\nenabled = false"));
    }

    #[test]
    fn tcp_control_uses_jsonrpc_port() {
        let conf = render_config(&test_config());
        assert!(conf.contains("[tcp-control]"));
        assert!(conf.contains("port = 1705"));
    }

    #[test]
    fn uses_default_bind_address() {
        let conf = render_config(&test_config());
        // No bind_to_address — snapserver uses default dual-stack (::)
        assert!(!conf.contains("bind_to_address"));
    }
}
