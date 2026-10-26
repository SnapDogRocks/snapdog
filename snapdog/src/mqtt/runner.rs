// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! The broker connection, run as tasks of their own.
//!
//! - [`Connection`] polls rumqttc's event loop, hands incoming messages to the
//!   [`CommandHandler`], reconnects with a jittered backoff and, after every
//!   (re)connect, subscribes again and re-announces (availability and a state
//!   snapshot).
//! - [`run_state_publisher`] turns notifications into retained state messages.
//!
//! The event loop is only ever polled by its own task and never inside a
//! `select!` with other work. Dropping a `poll()` future part-way (as the main
//! loop used to, whenever any other branch fired) can cancel an in-flight
//! CONNECT, which makes the broker publish the last will.
//!
//! The session is a clean one on purpose: the broker keeps no subscriptions for
//! us and queues nothing while we are away. A persistent session would replay
//! commands that were issued while `SnapDog` was down, long after they made sense.
//! The price is that every connect has to subscribe and announce again.

use std::sync::Arc;
use std::time::{Duration, Instant};

use rumqttc::mqttbytes::Error as MqttBytesError;
use rumqttc::{
    ConnectReturnCode, ConnectionError, Event, EventLoop, Outgoing, Packet, QoS, StateError,
};
use tokio::sync::{broadcast, watch};

use super::commands::{CommandHandler, Handled};
use super::limits::LogThrottle;
use super::publisher::Publisher;
use super::transport::MAX_INCOMING_PACKET_BYTES;
use crate::api::ws::Notification;
use crate::state;

/// Shortest wait before a reconnect attempt.
const RECONNECT_MIN: Duration = Duration::from_secs(1);
/// Longest wait between reconnect attempts.
const RECONNECT_MAX: Duration = Duration::from_secs(60);
/// A connection that held this long counts as healthy: the backoff starts over.
/// Shorter ones (a broker that accepts and drops us, two instances fighting over
/// one client id) keep the backoff growing.
const STABLE_AFTER: Duration = Duration::from_secs(30);

/// Reconnect delays with decorrelated jitter: each delay is drawn uniformly from
/// `[min, 3 * previous delay]` and capped at `max`. The spread keeps a fleet of
/// clients from reconnecting in lockstep after a broker restart.
#[derive(Debug)]
pub struct Backoff {
    min: Duration,
    max: Duration,
    prev: Duration,
}

impl Backoff {
    pub const fn new(min: Duration, max: Duration) -> Self {
        Self {
            min,
            max,
            prev: min,
        }
    }

    /// The next delay.
    pub fn next_delay(&mut self) -> Duration {
        self.next_with(fastrand::f64())
    }

    /// The next delay for the uniform sample `unit` in `[0, 1]`.
    fn next_with(&mut self, unit: f64) -> Duration {
        let hi = self.prev.saturating_mul(3).min(self.max).max(self.min);
        let delay = self.min + hi.saturating_sub(self.min).mul_f64(unit.clamp(0.0, 1.0));
        self.prev = delay;
        delay
    }

    /// Start over at the minimum.
    pub const fn reset(&mut self) {
        self.prev = self.min;
    }
}

/// Resolves when shutdown is requested (or the sender is gone).
async fn shutdown_requested(rx: &mut watch::Receiver<bool>) {
    loop {
        if *rx.borrow_and_update() {
            return;
        }
        if rx.changed().await.is_err() {
            return;
        }
    }
}

/// Everything the connection task uses apart from the event loop. Kept apart
/// from it because the event loop is not `Sync`: a `&self` that covered it could
/// not be held across an `.await` in a spawned task.
pub struct Session {
    pub publisher: Publisher,
    pub handler: CommandHandler,
    pub store: state::SharedState,
    /// Set while a connection is established.
    pub connected: watch::Sender<bool>,
    pub shutdown: watch::Receiver<bool>,
    /// Keeps a flood of bad messages from flooding the log.
    pub log_throttle: LogThrottle,
}

/// Poll the broker until shutdown.
pub async fn run_connection(mut eventloop: EventLoop, mut this: Session) {
    let mut backoff = Backoff::new(RECONNECT_MIN, RECONNECT_MAX);
    let mut up_since: Option<Instant> = None;
    let mut first_connect = true;
    loop {
        match eventloop.poll().await {
            Ok(Event::Incoming(Packet::ConnAck(_))) => {
                tracing::info!("MQTT connected");
                up_since = Some(Instant::now());
                // The flag first: a notification that arrives while the snapshot
                // is queued is then published rather than skipped.
                this.connected.send_replace(true);
                this.announce(first_connect).await;
                first_connect = false;
            }
            Ok(Event::Incoming(Packet::Publish(msg))) => this.on_message(&msg).await,
            Ok(Event::Incoming(Packet::SubAck(ack))) => {
                let refused = ack
                    .return_codes
                    .iter()
                    .filter(|code| matches!(code, rumqttc::SubscribeReasonCode::Failure))
                    .count();
                if refused > 0 {
                    tracing::warn!(
                        refused,
                        "MQTT broker refused subscriptions, commands on those topics will not \
                         arrive (check the ACL for the base topic)"
                    );
                }
            }
            // Our own DISCONNECT went out: this is the graceful end.
            Ok(Event::Outgoing(Outgoing::Disconnect)) => break,
            Ok(_) => {}
            Err(e) => {
                this.connected.send_replace(false);
                if *this.shutdown.borrow() {
                    break;
                }
                if up_since
                    .take()
                    .is_some_and(|since| since.elapsed() >= STABLE_AFTER)
                {
                    backoff.reset();
                }
                let delay = backoff.next_delay();
                this.report_connection_error(&e, delay);
                tokio::select! {
                    () = tokio::time::sleep(delay) => {}
                    () = shutdown_requested(&mut this.shutdown) => break,
                }
            }
        }
    }
    this.connected.send_replace(false);
    tracing::debug!("MQTT connection task finished");
}

impl Session {
    /// After a (re)connect the broker knows nothing about us: subscribe, announce
    /// `online` and publish the current state of every zone and client.
    async fn announce(&self, first_connect: bool) {
        let client = self.publisher.client();
        let mut subscribed = 0_usize;
        for filter in self.publisher.topics().command_filters() {
            match client.try_subscribe(&filter, QoS::AtLeastOnce) {
                Ok(()) => subscribed += 1,
                Err(e) => {
                    tracing::warn!(error = %e, topic = ?filter, "MQTT subscribe could not be queued");
                }
            }
        }
        if let Err(e) = self.publisher.online() {
            tracing::warn!(error = %e, "MQTT online announcement could not be queued");
        }
        let store = self.store.read().await;
        if first_connect {
            self.publisher.clear_legacy_retained(store.zones.len());
        }
        let states = self.publisher.snapshot(&store);
        drop(store);
        tracing::info!(subscribed, states, "MQTT subscribed and announced");
    }

    async fn on_message(&mut self, msg: &rumqttc::Publish) {
        // The payload is logged here and nowhere else, and only at debug level, in
        // part and escaped (`?`): it is whatever somebody on the network sent.
        tracing::debug!(
            topic = ?msg.topic,
            retain = msg.retain,
            bytes = msg.payload.len(),
            payload = ?String::from_utf8_lossy(&msg.payload[..msg.payload.len().min(64)]),
            "MQTT message received"
        );
        let result = self
            .handler
            .handle(&msg.topic, &msg.payload, msg.retain)
            .await;
        let (category, outcome) = match result {
            Ok(Handled::Dispatched | Handled::NotACommand) => return,
            Ok(Handled::ClientOffline) => {
                tracing::debug!(topic = ?msg.topic, "MQTT command for a client that is not connected, ignored");
                return;
            }
            Ok(Handled::Deferred) => (
                "deferred",
                "Delaying MQTT commands: more arrive than one zone or client can take. The latest \
                 value of each setting is delivered as soon as there is room"
                    .to_string(),
            ),
            Ok(Handled::IgnoredRetained) => (
                "ignored: retained",
                "Ignoring a retained message on a command topic: retained commands are never \
                 executed (clear it with an empty retained publish)"
                    .to_string(),
            ),
            Ok(Handled::RateLimited) => (
                "ignored: rate limited",
                "Dropping MQTT commands: too many for one zone or client".to_string(),
            ),
            Err(rejected) => (
                rejected.category(),
                format!("Rejected MQTT command: {rejected}"),
            ),
        };
        if let Some(held_back) = self.log_throttle.check(category, Instant::now()) {
            // The topic is whatever a client published to: escaped (`?`), never raw.
            tracing::warn!(topic = ?msg.topic, held_back, "{outcome}");
        }
    }
}

/// The size of the packet that ended the connection, if that was the cause: the
/// broker sent more than [`MAX_INCOMING_PACKET_BYTES`].
const fn oversized_packet(error: &ConnectionError) -> Option<usize> {
    match error {
        ConnectionError::MqttState(StateError::Deserialization(
            MqttBytesError::PayloadSizeLimitExceeded(size),
        )) => Some(*size),
        _ => None,
    }
}

/// The hint for an oversized packet: what it most likely is and how to get rid of it.
fn oversized_packet_hint(size: usize, base: &str) -> String {
    format!(
        "MQTT broker sent a packet of {size} bytes, above the limit of \
         {MAX_INCOMING_PACKET_BYTES} bytes, and the connection was dropped. This is usually a \
         retained message that somebody published below {base}, typically to a `.../set` command \
         topic: the broker replays it at every connect, so MQTT stays down until it is removed. \
         Find it (for example with `mosquitto_sub -v -t '{base}#'`) and clear it with an empty \
         retained publish to the same topic (`mosquitto_pub -r -n -t <topic>`). Limit the \
         broker's message size (Mosquitto: `message_size_limit`) to keep it from happening again"
    )
}

impl Session {
    /// Log why a connection attempt failed, with a hint where the cause is typical.
    /// A cause that repeats on every retry is logged once per throttle interval.
    fn report_connection_error(&mut self, error: &ConnectionError, retry_in: Duration) {
        if let Some(size) = oversized_packet(error) {
            if let Some(held_back) = self
                .log_throttle
                .check("connection: packet too large", Instant::now())
            {
                let base = self.publisher.topics().join("");
                tracing::error!(
                    size,
                    held_back,
                    retry_in_secs = retry_in.as_secs_f32(),
                    "{}",
                    oversized_packet_hint(size, &base)
                );
            }
            return;
        }
        log_connection_error(error, retry_in);
    }
}

/// Log why a connection attempt failed, with a hint where the cause is typical.
fn log_connection_error(error: &ConnectionError, retry_in: Duration) {
    let retry_in_secs = retry_in.as_secs_f32();
    match error {
        ConnectionError::ConnectionRefused(
            code @ (ConnectReturnCode::BadUserNamePassword | ConnectReturnCode::NotAuthorized),
        ) => tracing::error!(
            ?code,
            retry_in_secs,
            "MQTT broker refused the credentials: check username, password and the ACL"
        ),
        ConnectionError::ConnectionRefused(code) => {
            tracing::error!(?code, retry_in_secs, "MQTT broker refused the connection");
        }
        ConnectionError::Tls(e) => tracing::error!(
            error = %e,
            retry_in_secs,
            "MQTT TLS handshake failed: check the broker certificate, the CA and the host name"
        ),
        other => tracing::warn!(error = %other, retry_in_secs, "MQTT connection error, retrying"),
    }
}

/// How often the backlog is retried while it is not empty.
const BACKLOG_TICK: Duration = Duration::from_millis(50);

/// Deliver the values that could not be delivered at once (see
/// [`CommandHandler::flush_backlog`]) until shutdown. Sleeps while there is
/// nothing to deliver.
pub async fn run_backlog_flusher(handler: CommandHandler, mut shutdown: watch::Receiver<bool>) {
    loop {
        tokio::select! {
            () = shutdown_requested(&mut shutdown) => return,
            () = handler.backlog_added() => {}
        }
        while handler.has_backlog() {
            tokio::select! {
                () = shutdown_requested(&mut shutdown) => return,
                () = tokio::time::sleep(BACKLOG_TICK) => {}
            }
            handler.flush_backlog(Instant::now()).await;
        }
    }
}

/// Publish retained state when zones or clients change.
///
/// While the broker is away nothing is queued: the snapshot after the next
/// connect replaces it. If this task falls behind the notification channel and
/// notifications are dropped, it re-publishes everything.
pub async fn run_state_publisher(
    publisher: Publisher,
    store: state::SharedState,
    mut notifications: broadcast::Receiver<Arc<str>>,
    connected: watch::Receiver<bool>,
    mut shutdown: watch::Receiver<bool>,
) {
    loop {
        let received = tokio::select! {
            () = shutdown_requested(&mut shutdown) => break,
            received = notifications.recv() => received,
        };
        match received {
            Ok(json) => {
                if !*connected.borrow() {
                    continue;
                }
                let Ok(notification) = serde_json::from_str::<Notification>(&json) else {
                    continue;
                };
                let store = store.read().await;
                match notification {
                    Notification::ZoneChanged { zone, .. }
                    | Notification::ZoneVolumeChanged { zone, .. }
                    | Notification::ZoneProgress { zone, .. } => {
                        if let Some(z) = store.zones.get(&zone) {
                            let _ = publisher.zone_state(zone, z);
                        }
                    }
                    Notification::ClientStateChanged { client, .. } => {
                        if let Some(c) = store.clients.get(&client) {
                            let _ = publisher.client_state(client, c);
                        }
                    }
                    _ => {}
                }
            }
            Err(broadcast::error::RecvError::Lagged(missed)) => {
                tracing::warn!(
                    missed,
                    "MQTT state publisher lagged, re-publishing all state"
                );
                if *connected.borrow() {
                    let store = store.read().await;
                    publisher.snapshot(&store);
                }
            }
            Err(broadcast::error::RecvError::Closed) => break,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_packet_over_the_limit_is_recognised_and_gets_a_hint_about_retained_messages() {
        let error = ConnectionError::MqttState(StateError::Deserialization(
            MqttBytesError::PayloadSizeLimitExceeded(20_000),
        ));
        assert_eq!(oversized_packet(&error), Some(20_000));
        let hint = oversized_packet_hint(20_000, "snapdog/");
        assert!(hint.contains("retained"), "{hint}");
        assert!(hint.contains("snapdog/"), "{hint}");
        assert!(hint.contains("20000"), "{hint}");
        assert!(hint.contains("message_size_limit"), "{hint}");
        // Other errors are not mistaken for it.
        assert_eq!(oversized_packet(&ConnectionError::NetworkTimeout), None);
        assert_eq!(
            oversized_packet(&ConnectionError::MqttState(StateError::AwaitPingResp)),
            None
        );
    }

    /// The flusher task delivers the value that waited, without any further message
    /// arriving to nudge it (the event loop may be idle for a minute).
    #[tokio::test]
    async fn the_flusher_task_delivers_a_deferred_value_by_itself() {
        use crate::mqtt::test_support::{snap_channel, test_state_with_client, zone_channels_with};
        use crate::mqtt::topics::Topics;
        use crate::player::ZoneCommand;

        let (snap_tx, _snap_rx) = snap_channel();
        let (cmds, mut rx) = zone_channels_with(1);
        let handler = CommandHandler::new(
            Topics::new("snapdog/"),
            cmds,
            test_state_with_client(),
            snap_tx,
        );
        let (stop, shutdown) = watch::channel(false);
        let task = tokio::spawn(run_backlog_flusher(handler.clone(), shutdown));

        handler
            .handle("snapdog/zones/1/volume/set", b"10", false)
            .await
            .unwrap();
        let deferred = handler
            .handle("snapdog/zones/1/volume/set", b"20", false)
            .await;
        assert_eq!(deferred, Ok(crate::mqtt::commands::Handled::Deferred));
        assert!(matches!(rx.recv().await, Some(ZoneCommand::SetVolume(10))));
        // Room is free now; the flusher notices on its own.
        let next = tokio::time::timeout(Duration::from_secs(3), rx.recv())
            .await
            .expect("the deferred value arrives");
        assert!(matches!(next, Some(ZoneCommand::SetVolume(20))), "{next:?}");

        stop.send_replace(true);
        tokio::time::timeout(Duration::from_secs(2), task)
            .await
            .expect("the flusher stops on shutdown")
            .unwrap();
    }

    #[test]
    fn backoff_stays_within_its_bounds_and_grows_to_the_cap() {
        let mut backoff = Backoff::new(RECONNECT_MIN, RECONNECT_MAX);
        let mut max_seen = Duration::ZERO;
        for _ in 0..10_000 {
            let d = backoff.next_delay();
            assert!(d >= RECONNECT_MIN && d <= RECONNECT_MAX, "{d:?}");
            max_seen = max_seen.max(d);
        }
        assert!(
            max_seen > RECONNECT_MAX.mul_f64(0.9),
            "never approached the cap: {max_seen:?}"
        );
    }

    #[test]
    fn first_delay_is_at_most_three_times_the_minimum() {
        for unit in [0.0, 0.25, 0.5, 1.0] {
            let mut backoff = Backoff::new(RECONNECT_MIN, RECONNECT_MAX);
            let d = backoff.next_with(unit);
            assert!(
                d >= RECONNECT_MIN && d <= RECONNECT_MIN * 3,
                "{unit}: {d:?}"
            );
        }
    }

    #[test]
    fn worst_case_growth_is_tripling_up_to_the_cap() {
        let mut backoff = Backoff::new(RECONNECT_MIN, RECONNECT_MAX);
        let delays: Vec<u64> = (0..6).map(|_| backoff.next_with(1.0).as_secs()).collect();
        assert_eq!(delays, [3, 9, 27, 60, 60, 60]);
    }

    #[test]
    fn best_case_stays_at_the_minimum() {
        let mut backoff = Backoff::new(RECONNECT_MIN, RECONNECT_MAX);
        for _ in 0..10 {
            assert_eq!(backoff.next_with(0.0), RECONNECT_MIN);
        }
    }

    #[test]
    fn reset_starts_over() {
        let mut backoff = Backoff::new(RECONNECT_MIN, RECONNECT_MAX);
        for _ in 0..6 {
            backoff.next_with(1.0);
        }
        backoff.reset();
        assert!(backoff.next_with(1.0) <= RECONNECT_MIN * 3);
    }

    #[test]
    fn delays_are_jittered() {
        let mut distinct = std::collections::HashSet::new();
        for _ in 0..50 {
            let mut backoff = Backoff::new(RECONNECT_MIN, RECONNECT_MAX);
            distinct.insert(backoff.next_delay().as_millis());
        }
        assert!(distinct.len() > 10, "delays do not vary: {distinct:?}");
    }
}
