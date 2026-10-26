// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! MQTT integration via rumqttc.
//!
//! Bidirectional: subscribes to command topics (`{base}zones/{n}/.../set`,
//! `{base}clients/{n}/.../set`) and publishes retained state (`.../state`) and
//! availability (`{base}status`).
//!
//! The broker connection runs in its own tasks ([`MqttBridge::start`]). Commands
//! are routed through the `ZonePlayer` command channels and the Snapcast command
//! channel; the connection re-subscribes and re-announces after every reconnect.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use rumqttc::AsyncClient;
use tokio::sync::{broadcast, mpsc, watch};
use tokio::task::JoinHandle;

use crate::config::MqttConfig;
use crate::player::{SnapcastCmd, ZoneCommandSender};
use crate::state;

mod commands;
mod limits;
mod parse;
mod payloads;
mod publisher;
mod runner;
#[cfg(test)]
pub(crate) mod test_pki;
#[cfg(test)]
mod test_support;
pub(crate) mod tls;
mod topics;
mod transport;

use commands::CommandHandler;
use limits::LogThrottle;
use publisher::{Publisher, request_capacity};
use runner::{
    Session, StateFeed, supervised_backlog_flusher, supervised_connection,
    supervised_state_publisher,
};
use topics::Topics;
use transport::mqtt_options;

/// What the bridge needs from the rest of the server.
pub struct MqttRuntime {
    /// Command channels of the zone players, by 1-based zone index.
    pub zone_commands: HashMap<usize, ZoneCommandSender>,
    /// Shared server state: source of the published state, target of client moves.
    pub state: state::SharedState,
    /// Snapcast command channel (client volume, mute, latency, zone moves).
    pub snap_tx: mpsc::Sender<SnapcastCmd>,
    /// Notification stream (`ZoneChanged`, `ClientStateChanged`, ...) as JSON.
    pub notifications: broadcast::Receiver<Arc<str>>,
}

/// `config`, checked and with its file-based secrets loaded.
///
/// The config loader has done this already (it is idempotent), but a caller may
/// hand over a section it built by hand, and a `password_file` in it would
/// otherwise be silently ignored. Warnings that only this pass finds are logged.
fn resolved(config: &MqttConfig) -> Result<MqttConfig> {
    let mut resolved = config.clone();
    resolved.resolve()?;
    for warning in &resolved.warnings[config.warnings.len()..] {
        tracing::warn!("{warning}");
    }
    Ok(resolved)
}

/// Handle to the running MQTT bridge. Dropping it aborts the tasks without a
/// DISCONNECT, so the broker publishes the last will; use [`Self::shutdown`] for
/// a clean exit.
pub struct MqttBridge {
    publisher: Publisher,
    connected: watch::Receiver<bool>,
    shutdown: watch::Sender<bool>,
    tasks: Vec<JoinHandle<()>>,
}

impl MqttBridge {
    /// Start the bridge: spawn the connection and the state publisher.
    ///
    /// Nothing is dialled here, the connection task does that (and retries). The
    /// only errors are configuration errors.
    ///
    /// # Errors
    ///
    /// Returns an error if the section is invalid (see [`MqttConfig::resolve`]) or the
    /// TLS material cannot be used. The bridge then does not start; it never falls
    /// back to plaintext.
    #[tracing::instrument(skip_all, fields(broker = %config.broker))]
    pub async fn start(config: &MqttConfig, base_url: &str, runtime: MqttRuntime) -> Result<Self> {
        let config = &resolved(config)?;
        let topics = Topics::new(&config.base_topic);
        let options = mqtt_options(config, &topics)?;
        if let Some(warning) = config.plaintext_credentials_warning() {
            tracing::warn!("{warning}");
        }

        let capacity = {
            let store = runtime.state.read().await;
            request_capacity(store.zones.len(), store.clients.len())
        };
        let (client, eventloop) = AsyncClient::new(options, capacity);
        let publisher = Publisher::new(client, topics.clone(), base_url);

        let (connected_tx, connected_rx) = watch::channel(false);
        let (shutdown_tx, shutdown_rx) = watch::channel(false);

        let handler = CommandHandler::new(
            topics,
            runtime.zone_commands,
            runtime.state.clone(),
            runtime.snap_tx,
        );
        let session = Session {
            publisher: publisher.clone(),
            handler: handler.clone(),
            store: runtime.state.clone(),
            connected: connected_tx,
            shutdown: shutdown_rx.clone(),
            log_throttle: LogThrottle::new(Duration::from_secs(10)),
        };
        let feed = StateFeed {
            publisher: publisher.clone(),
            store: runtime.state,
            notifications: runtime.notifications,
            connected: connected_rx.clone(),
            shutdown: shutdown_rx.clone(),
        };
        let tasks = vec![
            tokio::spawn(supervised_connection(eventloop, session)),
            tokio::spawn(supervised_backlog_flusher(handler, shutdown_rx)),
            tokio::spawn(supervised_state_publisher(feed)),
        ];
        tracing::info!(capacity, "MQTT bridge started");

        Ok(Self {
            publisher,
            connected: connected_rx,
            shutdown: shutdown_tx,
            tasks,
        })
    }

    /// Whether a broker connection is established right now.
    #[must_use]
    pub fn is_connected(&self) -> bool {
        *self.connected.borrow()
    }

    /// Wait until the broker has accepted the connection, up to `timeout`.
    pub async fn wait_connected(&self, timeout: Duration) -> bool {
        let mut connected = self.connected.clone();
        tokio::time::timeout(timeout, connected.wait_for(|up| *up))
            .await
            .is_ok_and(|r| r.is_ok())
    }

    /// Queue a retained status value for `{base}{topic}`.
    ///
    /// # Errors
    ///
    /// Returns an error if the request queue is full.
    pub fn publish(&self, topic: &str, payload: &str) -> Result<()> {
        self.publisher
            .retained(&self.publisher.topics().join(topic), payload)
    }

    /// Queue the retained zone state.
    ///
    /// # Errors
    ///
    /// Returns an error if the request queue is full.
    pub fn publish_zone_state(&self, index: usize, zone: &state::ZoneState) -> Result<()> {
        self.publisher.zone_state(index, zone)
    }

    /// Queue the retained client state.
    ///
    /// # Errors
    ///
    /// Returns an error if the request queue is full.
    pub fn publish_client_state(&self, index: usize, client: &state::ClientState) -> Result<()> {
        self.publisher.client_state(index, client)
    }

    /// Leave cleanly: publish the retained `offline` (a DISCONNECT suppresses the
    /// last will), send DISCONNECT, and wait up to `timeout` for the tasks.
    pub async fn shutdown(mut self, timeout: Duration) {
        let _ = self.publisher.offline();
        let _ = self.publisher.client().try_disconnect();
        self.shutdown.send_replace(true);
        let tasks = std::mem::take(&mut self.tasks);
        let finished = tokio::time::timeout(timeout, async {
            for task in tasks {
                let _ = task.await;
            }
        })
        .await;
        if finished.is_err() {
            tracing::warn!("MQTT shutdown timed out, closing the connection");
        }
        // Dropping `self` aborts whatever is left.
    }
}

impl Drop for MqttBridge {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Review finding F13: `start` took a section as it was handed over, so a
    /// `password_file` in a hand-built one was silently ignored.
    #[test]
    fn a_section_that_skipped_the_config_loader_is_resolved_before_it_is_used() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("pw");
        std::fs::write(&file, "from-file\n").unwrap();
        let config: MqttConfig = toml::from_str(&format!(
            "broker = \"b:1883\"\nusername = \"u\"\npassword_file = {:?}",
            file.display().to_string()
        ))
        .unwrap();
        let resolved = resolved(&config).unwrap();
        assert_eq!(resolved.password.as_str(), "from-file");
        let options = mqtt_options(&resolved, &Topics::new(&resolved.base_topic)).unwrap();
        assert_eq!(options.credentials().unwrap().password, "from-file");

        // And a section that is wrong is refused, not started.
        let bad: MqttConfig = toml::from_str("broker = \"b:1883\"\npassword = \"p\"").unwrap();
        assert!(resolved_err(&bad).contains("username"));
        // Resolving twice is harmless: what the loader did is not undone.
        assert_eq!(
            super::resolved(&resolved).unwrap().password.as_str(),
            "from-file"
        );
    }

    /// The same through `start`: the password of an unresolved `password_file` is
    /// what reaches the broker in the CONNECT packet.
    #[tokio::test]
    async fn start_sends_the_password_from_a_password_file_it_resolved_itself() {
        use tokio::io::AsyncReadExt;
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("pw");
        std::fs::write(&file, "from-file-secret\n").unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let config: MqttConfig = toml::from_str(&format!(
            "broker = \"127.0.0.1:{port}\"\nusername = \"the-user\"\npassword_file = {:?}",
            file.display().to_string()
        ))
        .unwrap();
        let (runtime, _ends) = crate::mqtt::test_support::test_runtime();
        let bridge = MqttBridge::start(&config, "http://h", runtime)
            .await
            .unwrap();

        let (mut socket, _) = tokio::time::timeout(Duration::from_secs(5), listener.accept())
            .await
            .expect("the bridge dials")
            .unwrap();
        let mut connect = vec![0_u8; 512];
        let n = tokio::time::timeout(Duration::from_secs(5), socket.read(&mut connect))
            .await
            .expect("CONNECT arrives")
            .unwrap();
        let connect = &connect[..n];
        let has = |needle: &[u8]| connect.windows(needle.len()).any(|w| w == needle);
        assert!(has(b"the-user"), "username in CONNECT");
        assert!(
            has(b"from-file-secret"),
            "password from the file in CONNECT"
        );
        drop(bridge);
    }

    fn resolved_err(config: &MqttConfig) -> String {
        format!("{:#}", resolved(config).unwrap_err())
    }
}
