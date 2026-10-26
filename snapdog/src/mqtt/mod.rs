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
use tracing::Instrument;

use crate::config::MqttConfig;
use crate::player::{SnapcastCmd, ZoneCommandSender};
use crate::state;

mod commands;
mod discovery;
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
use discovery::Discovery;
pub use discovery::ServerInfo;
use limits::LogThrottle;
use publisher::{Publisher, request_capacity};
use runner::{
    Backoff, RECONNECT_MAX, RECONNECT_MIN, Session, StateFeed, supervised_backlog_flusher,
    supervised_connection, supervised_state_publisher,
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
    /// Who `SnapDog` says it is in the Home Assistant discovery message.
    pub server: ServerInfo,
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
        let discovery = Discovery::new(
            config,
            publisher.clone(),
            runtime.state.clone(),
            runtime.server,
        );

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
            client_id: config.client_id.clone(),
            discovery: discovery.clone(),
            clear_legacy: config.legacy_cleanup(),
            backoff: Backoff::new(RECONNECT_MIN, RECONNECT_MAX),
            handler: handler.clone(),
            store: runtime.state.clone(),
            connected: connected_tx,
            shutdown: shutdown_rx.clone(),
            log_throttle: LogThrottle::new(Duration::from_secs(10)),
        };
        let feed = StateFeed {
            discovery,
            publisher: publisher.clone(),
            store: runtime.state,
            notifications: runtime.notifications,
            connected: connected_rx.clone(),
            shutdown: shutdown_rx.clone(),
        };
        // The tasks outlive this function, so they carry its span (the broker
        // field) with them: their log lines say which broker they are about.
        let span = tracing::Span::current();
        let tasks = vec![
            tokio::spawn(supervised_connection(eventloop, session).instrument(span.clone())),
            tokio::spawn(supervised_backlog_flusher(handler, shutdown_rx).instrument(span.clone())),
            tokio::spawn(supervised_state_publisher(feed).instrument(span)),
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

    /// What `SnapDog` puts on the wire: every subscription and every publish is
    /// `QoS` 1, state and availability are retained, the last will is a retained
    /// `offline`. (Review mutation M3: `QoS` 1 became `QoS` 0 and nothing noticed.)
    #[tokio::test]
    async fn the_wire_carries_qos_1_retained_state_and_a_retained_last_will() {
        use crate::mqtt::test_support::{FakeBroker, Script};
        use rumqttc::Packet;

        let broker = FakeBroker::start(Script::Serve).await;
        let config: MqttConfig =
            toml::from_str(&format!("broker = \"127.0.0.1:{}\"", broker.port)).unwrap();
        let (runtime, _ends) = crate::mqtt::test_support::test_runtime();
        let bridge = MqttBridge::start(&config, "http://h", runtime)
            .await
            .unwrap();
        assert!(bridge.wait_connected(Duration::from_secs(5)).await);
        // One CONNECT, 16 subscription filters, `online` and the state of 2 zones
        // and 1 client (plus the clearing of legacy topics).
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        while tokio::time::Instant::now() < deadline
            && !broker
                .packets()
                .iter()
                .any(|p| matches!(p, Packet::Publish(x) if x.topic == "snapdog/clients/1/state"))
        {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let packets = broker.packets();

        let connect = packets
            .iter()
            .find_map(|p| {
                if let Packet::Connect(c) = p {
                    Some(c)
                } else {
                    None
                }
            })
            .expect("CONNECT");
        let will = connect.last_will.as_ref().expect("last will");
        assert_eq!(will.topic, "snapdog/status");
        assert_eq!(&will.message[..], b"offline");
        assert_eq!(will.qos, rumqttc::QoS::AtLeastOnce);
        assert!(will.retain);

        let filters: Vec<_> = packets
            .iter()
            .filter_map(|p| {
                if let Packet::Subscribe(s) = p {
                    Some(s)
                } else {
                    None
                }
            })
            .flat_map(|s| s.filters.iter())
            .collect();
        assert_eq!(filters.len(), super::topics::COMMAND_ROUTES.len());
        assert!(
            filters.iter().all(|f| f.qos == rumqttc::QoS::AtLeastOnce),
            "commands are subscribed at QoS 1: {filters:?}"
        );

        let state_topics = [
            "snapdog/status",
            "snapdog/zones/1/state",
            "snapdog/zones/2/state",
            "snapdog/clients/1/state",
        ];
        for topic in state_topics {
            let publish = packets
                .iter()
                .find_map(|p| match p {
                    Packet::Publish(x) if x.topic == topic => Some(x),
                    _ => None,
                })
                .unwrap_or_else(|| panic!("{topic} was not published"));
            assert_eq!(publish.qos, rumqttc::QoS::AtLeastOnce, "{topic}");
            assert!(publish.retain, "{topic} is retained");
        }
        drop(bridge);
    }

    /// Review finding F11: the clean-up of what earlier releases left behind ran at
    /// every start, whatever the ACL, with no way to stop it.
    #[tokio::test]
    async fn the_legacy_cleanup_can_be_switched_off() {
        use crate::mqtt::test_support::{FakeBroker, Script};
        use rumqttc::Packet;

        async fn publishes(extra: &str) -> Vec<(String, usize)> {
            let broker = FakeBroker::start(Script::Serve).await;
            let config: MqttConfig =
                toml::from_str(&format!("broker = \"127.0.0.1:{}\"\n{extra}", broker.port))
                    .unwrap();
            let (runtime, _ends) = crate::mqtt::test_support::test_runtime();
            let bridge = MqttBridge::start(&config, "http://h", runtime)
                .await
                .unwrap();
            assert!(bridge.wait_connected(Duration::from_secs(5)).await);
            // The client state is queued last: when it is there, the rest is.
            let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
            while tokio::time::Instant::now() < deadline
                && !broker.packets().iter().any(
                    |p| matches!(p, Packet::Publish(x) if x.topic == "snapdog/clients/1/state"),
                )
            {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            let found = broker
                .packets()
                .iter()
                .filter_map(|p| match p {
                    Packet::Publish(x) => Some((x.topic.clone(), x.payload.len())),
                    _ => None,
                })
                .collect();
            drop(bridge);
            found
        }

        // Default: only what is inside the base topic (`snapdog//status`). A broker ACL
        // that confines SnapDog to its base topic is not asked for anything else.
        let default = publishes("").await;
        assert!(
            default.contains(&("snapdog//status".to_string(), 0)),
            "{default:?}"
        );
        assert!(
            default
                .iter()
                .all(|(topic, _)| topic.starts_with("snapdog/")),
            "by default nothing is written outside the base topic: {default:?}"
        );
        // `true` also clears the old media_player discovery topics.
        let all = publishes("clear_legacy_topics = true").await;
        assert!(all.contains(&("snapdog//status".to_string(), 0)), "{all:?}");
        assert!(
            all.contains(&(
                "homeassistant/media_player/snapdog_zone_1/config".to_string(),
                0
            )),
            "{all:?}"
        );
        let off = publishes("clear_legacy_topics = false").await;
        assert!(
            off.iter().all(
                |(topic, _)| !topic.starts_with("homeassistant/") && topic != "snapdog//status"
            ),
            "nothing may be cleared: {off:?}"
        );
        assert!(off.iter().any(|(topic, _)| topic == "snapdog/status"));
    }

    // ── A panic does not leave the bridge half-dead ───────────

    fn zone_state_publishes(broker: &crate::mqtt::test_support::FakeBroker) -> usize {
        broker
            .packets()
            .iter()
            .filter(
                |p| matches!(p, rumqttc::Packet::Publish(x) if x.topic == "snapdog/zones/1/state"),
            )
            .count()
    }

    fn volume_changed() -> Arc<str> {
        serde_json::json!({"type": "zone_volume_changed", "zone": 1, "volume": 20, "muted": false})
            .to_string()
            .into()
    }

    /// A message that makes the handler panic is dropped and logged. The connection,
    /// the subscriptions and the announcement are untouched, the next message is
    /// handled at once, and the state publisher still works. (Review finding M2.)
    #[tokio::test]
    async fn a_message_that_makes_the_handler_panic_is_dropped_and_the_bridge_carries_on() {
        let (broker, bridge, mut ends) =
            bridge_with(crate::mqtt::test_support::Script::Serve, "").await;
        let before = zone_state_publishes(&broker);
        broker.publish("snapdog/zones/1/__panic_message__", b"poison");
        broker.publish("snapdog/zones/1/volume/set", b"42");
        let cmd = tokio::time::timeout(Duration::from_secs(2), ends.zone_rx.recv())
            .await
            .expect("the message behind the poison one is handled at once");
        assert!(
            matches!(cmd, Some(crate::player::ZoneCommand::SetVolume(42))),
            "{cmd:?}"
        );
        assert!(bridge.is_connected());
        assert_eq!(
            broker.connections.load(std::sync::atomic::Ordering::SeqCst),
            1
        );
        // State still flows.
        ends.notify_tx.send(volume_changed()).unwrap();
        wait_for(&broker, |_| zone_state_publishes(&broker) > before).await;
        assert!(
            zone_state_publishes(&broker) > before,
            "state publishing stopped"
        );
    }

    /// A panic anywhere else in the connection task drops the connection and connects
    /// afresh: the flag is honest, everything is announced again, state flows. Before,
    /// the flag stayed false for good, nothing was announced again, and the retained
    /// state went stale while the entities showed as available. (Review finding M2.)
    #[tokio::test]
    async fn a_panic_in_the_connection_task_ends_in_a_clean_reconnect_and_a_full_announcement() {
        let (broker, bridge, ends) =
            bridge_with(crate::mqtt::test_support::Script::Serve, "").await;
        let online = |b: &crate::mqtt::test_support::FakeBroker| {
            b.packets()
                .iter()
                .filter(|p| matches!(p, rumqttc::Packet::Publish(x) if x.topic == "snapdog/status" && &x.payload[..] == b"online"))
                .count()
        };
        assert_eq!(online(&broker), 1);
        let before = zone_state_publishes(&broker);
        broker.publish("snapdog/zones/1/__panic_loop__", b"poison");
        wait_for(&broker, |_| {
            broker.connections.load(std::sync::atomic::Ordering::SeqCst) >= 2
        })
        .await;
        assert_eq!(
            broker.connections.load(std::sync::atomic::Ordering::SeqCst),
            2,
            "the connection is dropped and made again"
        );
        assert!(
            bridge.wait_connected(Duration::from_secs(5)).await,
            "connected again"
        );
        wait_for(&broker, |_| {
            online(&broker) >= 2 && zone_state_publishes(&broker) > before
        })
        .await;
        assert_eq!(online(&broker), 2, "announced again");
        assert!(
            zone_state_publishes(&broker) > before,
            "the state was published again"
        );
        // And it keeps working: a notification becomes a state message.
        let after = zone_state_publishes(&broker);
        ends.notify_tx.send(volume_changed()).unwrap();
        wait_for(&broker, |_| zone_state_publishes(&broker) > after).await;
        assert!(
            zone_state_publishes(&broker) > after,
            "state publishing stopped"
        );
    }

    // ── Home Assistant discovery on the wire ──────────────────

    /// A bridge against a scripted broker, with `extra` lines after the broker address.
    async fn bridge_with(
        script: crate::mqtt::test_support::Script,
        extra: &str,
    ) -> (
        crate::mqtt::test_support::FakeBroker,
        MqttBridge,
        crate::mqtt::test_support::RuntimeEnds,
    ) {
        use crate::mqtt::test_support::{FakeBroker, test_runtime};
        let broker = FakeBroker::start(script).await;
        let config: MqttConfig =
            toml::from_str(&format!("broker = \"127.0.0.1:{}\"\n{extra}", broker.port)).unwrap();
        let (runtime, ends) = test_runtime();
        let bridge = MqttBridge::start(&config, "http://h", runtime)
            .await
            .unwrap();
        assert!(bridge.wait_connected(Duration::from_secs(5)).await);
        // The client state is queued last of the announcement.
        wait_for(&broker, |p| {
            p.iter()
                .any(|x| matches!(x, rumqttc::Packet::Publish(x) if x.topic == "snapdog/clients/1/state"))
        })
        .await;
        // Give the discovery message (queued after it) time to go out.
        tokio::time::sleep(Duration::from_millis(300)).await;
        (broker, bridge, ends)
    }

    async fn wait_for(
        broker: &crate::mqtt::test_support::FakeBroker,
        done: impl Fn(&[rumqttc::Packet]) -> bool,
    ) {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        while tokio::time::Instant::now() < deadline && !done(&broker.packets()) {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    const DEVICE_TOPIC: &str =
        "homeassistant/device/snapdog_0b1c2d3e-aaaa-bbbb-cccc-1234567890ab/config";

    fn device_messages(broker: &crate::mqtt::test_support::FakeBroker) -> Vec<rumqttc::Publish> {
        broker
            .packets()
            .into_iter()
            .filter_map(|p| match p {
                rumqttc::Packet::Publish(x) if x.topic == DEVICE_TOPIC => Some(x),
                _ => None,
            })
            .collect()
    }

    fn subscribed_to(broker: &crate::mqtt::test_support::FakeBroker, topic: &str) -> bool {
        broker.packets().iter().any(|p| {
            matches!(p, rumqttc::Packet::Subscribe(s) if s.filters.iter().any(|f| f.path == topic))
        })
    }

    #[tokio::test]
    async fn without_a_home_assistant_section_nothing_is_announced_or_touched() {
        let (broker, bridge, _ends) =
            bridge_with(crate::mqtt::test_support::Script::Serve, "").await;
        assert_eq!(device_messages(&broker).len(), 0);
        assert!(!subscribed_to(&broker, "homeassistant/status"));
        drop(bridge);
    }

    #[tokio::test]
    async fn discovery_on_announces_one_retained_device_message_and_listens_for_the_birth() {
        let (broker, bridge, _ends) = bridge_with(
            crate::mqtt::test_support::Script::Serve,
            "[homeassistant]\ndiscovery = true",
        )
        .await;
        let messages = device_messages(&broker);
        assert_eq!(messages.len(), 1, "one message for the whole server");
        let message = &messages[0];
        assert!(message.retain);
        assert_eq!(message.qos, rumqttc::QoS::AtLeastOnce);
        // It is the message of the server state: 2 zones, 1 client.
        let sent: serde_json::Value = serde_json::from_slice(&message.payload).unwrap();
        let store = crate::mqtt::test_support::test_state_with_client();
        let expected = crate::mqtt::discovery::device_payload(
            &Topics::new("snapdog"),
            &crate::mqtt::test_support::test_server(),
            &crate::mqtt::discovery::Model::of(&*store.read().await),
        );
        assert_eq!(sent, expected);
        assert!(subscribed_to(&broker, "homeassistant/status"));
        drop(bridge);
    }

    #[tokio::test]
    async fn the_prefix_and_the_birth_topic_follow_the_configuration() {
        let (broker, bridge, _ends) = bridge_with(
            crate::mqtt::test_support::Script::Serve,
            "[homeassistant]\ndiscovery = true\ndiscovery_prefix = \"ha\"\nstatus_topic = \"ha/birth\"",
        )
        .await;
        assert!(broker.packets().iter().any(|p| matches!(
            p,
            rumqttc::Packet::Publish(x) if x.topic == "ha/device/snapdog_0b1c2d3e-aaaa-bbbb-cccc-1234567890ab/config"
        )));
        assert!(subscribed_to(&broker, "ha/birth"));
        assert!(!subscribed_to(&broker, "homeassistant/status"));
        drop(bridge);
    }

    #[tokio::test]
    async fn discovery_false_deletes_the_device_message_once() {
        let (broker, bridge, _ends) = bridge_with(
            crate::mqtt::test_support::Script::Serve,
            "[homeassistant]\ndiscovery = false",
        )
        .await;
        let messages = device_messages(&broker);
        assert_eq!(messages.len(), 1);
        assert!(messages[0].retain);
        assert!(
            messages[0].payload.is_empty(),
            "an empty retained payload deletes"
        );
        assert!(!subscribed_to(&broker, "homeassistant/status"));
        drop(bridge);
    }

    /// Home Assistant forgets its discovered entities when it restarts, and says so
    /// with a birth message: the device message follows (after a random delay).
    #[tokio::test]
    async fn a_birth_message_makes_the_device_message_go_out_again() {
        let (broker, bridge, _ends) = bridge_with(
            crate::mqtt::test_support::Script::Birth("homeassistant/status"),
            "[homeassistant]\ndiscovery = true",
        )
        .await;
        wait_for(&broker, |_| device_messages(&broker).len() >= 2).await;
        assert!(
            device_messages(&broker).len() >= 2,
            "no second device message after the birth message"
        );
        // The birth message is not a command and is not complained about.
        drop(bridge);
    }

    #[tokio::test]
    async fn a_client_that_appears_later_gets_entities() {
        let (broker, bridge, ends) = bridge_with(
            crate::mqtt::test_support::Script::Serve,
            "[homeassistant]\ndiscovery = true",
        )
        .await;
        assert_eq!(device_messages(&broker).len(), 1);
        {
            let mut store = ends.store.write().await;
            let mut client = store.clients[&1].clone();
            client.name = "Runtime".into();
            store.clients.insert(2, client);
        }
        let notification = serde_json::json!({
            "type": "client_state_changed", "client": 2, "volume": 50, "muted": false,
            "connected": true, "zone": 1, "is_snapdog": false
        });
        ends.notify_tx
            .send(notification.to_string().into())
            .unwrap();
        wait_for(&broker, |_| device_messages(&broker).len() >= 2).await;
        let messages = device_messages(&broker);
        assert_eq!(messages.len(), 2);
        let latest: serde_json::Value = serde_json::from_slice(&messages[1].payload).unwrap();
        assert!(latest["components"]["client_2_volume"].is_object());
        drop(bridge);
    }

    fn resolved_err(config: &MqttConfig) -> String {
        format!("{:#}", resolved(config).unwrap_err())
    }
}
