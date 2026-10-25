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

use anyhow::{Context, Result};
use rumqttc::{AsyncClient, LastWill, MqttOptions, QoS};
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
mod test_support;
mod topics;

use commands::CommandHandler;
use limits::LogThrottle;
use publisher::{Publisher, request_capacity};
use runner::{Session, run_connection, run_state_publisher};
use topics::Topics;

/// MQTT keep-alive interval.
const MQTT_KEEP_ALIVE: Duration = Duration::from_mins(1);

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
    /// Returns an error if the broker address is invalid.
    #[tracing::instrument(skip_all, fields(broker = %config.broker))]
    pub async fn start(config: &MqttConfig, base_url: &str, runtime: MqttRuntime) -> Result<Self> {
        let topics = Topics::new(&config.base_topic);
        let options = mqtt_options(config, &topics)?;

        let capacity = {
            let store = runtime.state.read().await;
            request_capacity(store.zones.len(), store.clients.len())
        };
        let (client, eventloop) = AsyncClient::new(options, capacity);
        let publisher = Publisher::new(client, topics.clone(), base_url);

        let (connected_tx, connected_rx) = watch::channel(false);
        let (shutdown_tx, shutdown_rx) = watch::channel(false);

        let session = Session {
            publisher: publisher.clone(),
            handler: CommandHandler::new(
                topics,
                runtime.zone_commands,
                runtime.state.clone(),
                runtime.snap_tx,
            ),
            store: runtime.state.clone(),
            connected: connected_tx,
            shutdown: shutdown_rx.clone(),
            log_throttle: LogThrottle::new(Duration::from_secs(10)),
        };
        let tasks = vec![
            tokio::spawn(run_connection(eventloop, session)),
            tokio::spawn(run_state_publisher(
                publisher.clone(),
                runtime.state,
                runtime.notifications,
                connected_rx.clone(),
                shutdown_rx,
            )),
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

/// Connection options for `config`: address, keep-alive, credentials, last will.
fn mqtt_options(config: &MqttConfig, topics: &Topics) -> Result<MqttOptions> {
    let mut opts = MqttOptions::new(
        &config.client_id,
        parse_host(&config.broker),
        parse_port(&config.broker)?,
    );
    opts.set_keep_alive(MQTT_KEEP_ALIVE);
    if !config.username.is_empty() {
        opts.set_credentials(&config.username, &*config.password);
    }
    opts.set_last_will(LastWill::new(
        topics.availability(),
        "offline",
        QoS::AtLeastOnce,
        true,
    ));
    Ok(opts)
}

fn parse_host(broker: &str) -> &str {
    broker.rsplit_once(':').map_or(broker, |(h, _)| h)
}

fn parse_port(broker: &str) -> Result<u16> {
    broker
        .rsplit_once(':')
        .and_then(|(_, p)| p.parse().ok())
        .context("Invalid broker address — expected host:port")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_broker_port() {
        assert_eq!(parse_port("mqtt:1883").unwrap(), 1883);
        assert_eq!(parse_port("localhost:1883").unwrap(), 1883);
        assert!(parse_port("no-port").is_err());
    }

    #[test]
    fn last_will_is_the_retained_offline_on_the_availability_topic() {
        let cfg = crate::config::load_raw(
            toml::from_str(
                r#"
                [mqtt]
                broker = "localhost:1883"
                base_topic = "home/audio"
                [[zone]]
                name = "Z1"
                [[client]]
                name = "C1"
                mac = "00:00:00:00:00:01"
                zone = "Z1"
                "#,
            )
            .unwrap(),
        )
        .unwrap();
        let mqtt = cfg.mqtt.unwrap();
        let topics = Topics::new(&mqtt.base_topic);
        let opts = mqtt_options(&mqtt, &topics).unwrap();
        let will = opts.last_will().expect("last will set");
        assert_eq!(will.topic, "home/audio/status");
        assert_eq!(&will.message[..], b"offline");
        assert!(will.retain);
    }
}
