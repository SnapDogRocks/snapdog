// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! The broker connection, run as tasks of their own.
//!
//! - [`run_connection`] polls rumqttc's event loop, hands incoming messages to the
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
//! Each task runs under [`supervise`]: a panic is logged and the task starts again
//! after a backoff, so a bug in one message cannot end MQTT for the rest of the
//! process. A caught panic also clears the connected flag, so that
//! [`super::MqttBridge::is_connected`] does not claim a connection nobody serves.
//!
//! The session is a clean one on purpose: the broker keeps no subscriptions for
//! us and queues nothing while we are away. A persistent session would replay
//! commands that were issued while `SnapDog` was down, long after they made sense.
//! The price is that every connect has to subscribe and announce again.

use std::any::Any;
use std::panic::AssertUnwindSafe;
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::FutureExt;

use rumqttc::mqttbytes::Error as MqttBytesError;
use rumqttc::{
    ConnectReturnCode, ConnectionError, Event, EventLoop, Outgoing, Packet, QoS, StateError,
};
use tokio::sync::{broadcast, watch};

use super::commands::{CommandHandler, Handled};
use super::discovery::Discovery;
use super::limits::LogThrottle;
use super::publisher::Publisher;
use super::transport::MAX_INCOMING_PACKET_BYTES;
use crate::api::ws::Notification;
use crate::config::LegacyCleanup;
use crate::state;

/// Shortest wait before a reconnect attempt.
pub const RECONNECT_MIN: Duration = Duration::from_secs(1);
/// Longest wait between reconnect attempts.
pub const RECONNECT_MAX: Duration = Duration::from_secs(60);
/// A connection that held this long counts as healthy: the backoff starts over.
/// Shorter ones (a broker that accepts and drops us, two instances fighting over
/// one client id) keep the backoff growing.
const STABLE_AFTER: Duration = Duration::from_secs(30);

/// Reconnect delays with decorrelated jitter: each delay is drawn uniformly from
/// `[min, 3 * previous delay]` and capped at `max`. The spread keeps a fleet of
/// clients from reconnecting in lockstep after a broker restart.
#[derive(Clone, Debug)]
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

/// How many connections in a row that closed within [`STABLE_AFTER`] of connecting
/// make the log point at a duplicate client id.
const FLAPPING_HINT_AFTER: u32 = 3;

/// What to do about a connection that ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Retry {
    /// How long to wait before the next attempt.
    pub delay: Duration,
    /// Connections keep closing soon after they were made: the log should say what
    /// usually causes that.
    pub flapping: bool,
}

/// Bookkeeping across reconnects: the backoff, and how often a connection held
/// only briefly. Pure (the clock and the random sample are parameters), so that
/// the rules are testable.
#[derive(Debug)]
pub struct Reconnect {
    backoff: Backoff,
    up_since: Option<Instant>,
    /// Connections in a row that closed within [`STABLE_AFTER`].
    short_lived: u32,
}

impl Reconnect {
    pub const fn new(backoff: Backoff) -> Self {
        Self {
            backoff,
            up_since: None,
            short_lived: 0,
        }
    }

    /// The broker accepted the connection at `now`.
    pub const fn connected(&mut self, now: Instant) {
        self.up_since = Some(now);
    }

    /// The connection ended, or an attempt failed, at `now`; `unit` is a uniform
    /// sample in `[0, 1]` for the jitter.
    ///
    /// A connection that held for [`STABLE_AFTER`] counts as healthy and the backoff
    /// starts over. A shorter one (a broker that accepts and drops us, two
    /// instances fighting over one client id) keeps it growing.
    pub fn failed(&mut self, now: Instant, unit: f64) -> Retry {
        match self.up_since.take() {
            Some(since) if now.saturating_duration_since(since) >= STABLE_AFTER => {
                self.backoff.reset();
                self.short_lived = 0;
            }
            Some(_) => self.short_lived += 1,
            // The attempt never got a connection: a refusal, not a flap.
            None => {}
        }
        let flapping = self.short_lived >= FLAPPING_HINT_AFTER
            && (self.short_lived == FLAPPING_HINT_AFTER || self.short_lived.is_multiple_of(20));
        Retry {
            delay: self.backoff.next_with(unit),
            flapping,
        }
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

/// The text of a caught panic, for the log.
fn panic_message(payload: &(dyn Any + Send)) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|s| (*s).to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "(no message)".to_string())
}

/// Run `run(&mut state)` until it returns. If it panics, log that, let `on_panic`
/// repair what the panic left behind, wait a jittered backoff (so that a panic on
/// every message does not spin) and run it again on the same state. A shutdown
/// request ends the wait and the loop.
///
/// The state lives outside the future on purpose: it survives the panic, so the
/// broker connection is not thrown away with the task.
pub async fn supervise<S>(
    name: &'static str,
    mut backoff: Backoff,
    mut state: S,
    mut shutdown: watch::Receiver<bool>,
    mut on_panic: impl FnMut(&mut S),
    mut run: impl AsyncFnMut(&mut S),
) {
    loop {
        let started = Instant::now();
        let Err(payload) = AssertUnwindSafe(run(&mut state)).catch_unwind().await else {
            return;
        };
        on_panic(&mut state);
        if started.elapsed() >= STABLE_AFTER {
            backoff.reset();
        }
        let delay = backoff.next_delay();
        tracing::error!(
            task = name,
            panic = %panic_message(&*payload),
            retry_in_secs = delay.as_secs_f32(),
            "MQTT task panicked, restarting it"
        );
        tokio::select! {
            () = tokio::time::sleep(delay) => {}
            () = shutdown_requested(&mut shutdown) => return,
        }
    }
}

/// What a panic of the connection task leaves behind: the flag still says
/// connected, but nobody serves the connection any more.
fn after_connection_panic((_, session): &mut (EventLoop, Session)) {
    session.connected.send_replace(false);
}

/// [`run_connection`] under [`supervise`].
pub async fn supervised_connection(eventloop: EventLoop, session: Session) {
    let shutdown = session.shutdown.clone();
    supervise(
        "connection",
        Backoff::new(RECONNECT_MIN, RECONNECT_MAX),
        (eventloop, session),
        shutdown,
        after_connection_panic,
        async |(eventloop, session)| run_connection(eventloop, session).await,
    )
    .await;
}

/// [`run_state_publisher`] under [`supervise`].
pub async fn supervised_state_publisher(feed: StateFeed) {
    let shutdown = feed.shutdown.clone();
    supervise(
        "state publisher",
        Backoff::new(RECONNECT_MIN, RECONNECT_MAX),
        feed,
        shutdown,
        |_| {},
        run_state_publisher,
    )
    .await;
}

/// [`run_backlog_flusher`] under [`supervise`].
pub async fn supervised_backlog_flusher(handler: CommandHandler, shutdown: watch::Receiver<bool>) {
    supervise(
        "command backlog",
        Backoff::new(RECONNECT_MIN, RECONNECT_MAX),
        (handler, shutdown.clone()),
        shutdown,
        |_| {},
        async |(handler, shutdown)| run_backlog_flusher(handler, shutdown).await,
    )
    .await;
}

/// Everything the connection task uses apart from the event loop. Kept apart
/// from it because the event loop is not `Sync`: a `&self` that covered it could
/// not be held across an `.await` in a spawned task.
pub struct Session {
    pub publisher: Publisher,
    /// The MQTT client id, for the hint about a duplicate one.
    pub client_id: String,
    /// Home Assistant discovery, if `[mqtt.homeassistant]` configures it.
    pub discovery: Option<Discovery>,
    /// What to clear of what earlier releases left on the broker (`[mqtt]
    /// clear_legacy_topics`).
    pub clear_legacy: LegacyCleanup,
    /// The reconnect delays this session starts from.
    pub backoff: Backoff,
    pub handler: CommandHandler,
    pub store: state::SharedState,
    /// Set while a connection is established.
    pub connected: watch::Sender<bool>,
    pub shutdown: watch::Receiver<bool>,
    /// Keeps a flood of bad messages from flooding the log.
    pub log_throttle: LogThrottle,
}

/// Poll the broker until shutdown.
pub async fn run_connection(eventloop: &mut EventLoop, this: &mut Session) {
    let mut reconnect = Reconnect::new(this.backoff.clone());
    let mut first_connect = true;
    loop {
        match eventloop.poll().await {
            Ok(Event::Incoming(Packet::ConnAck(_))) => {
                tracing::info!("MQTT connected");
                reconnect.connected(Instant::now());
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
                let retry = reconnect.failed(Instant::now(), fastrand::f64());
                this.report_connection_error(&e, retry);
                tokio::select! {
                    () = tokio::time::sleep(retry.delay) => {}
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
        if first_connect && self.clear_legacy != LegacyCleanup::Off {
            let cleared = self
                .publisher
                .clear_legacy_retained(store.zones.len(), self.clear_legacy);
            tracing::info!(
                topics = cleared.len(),
                first = ?cleared.first(),
                last = ?cleared.last(),
                "Asked the broker to delete retained messages that earlier releases left behind \
                 (`clear_legacy_topics`; `false` stops this, `true` also covers the old \
                 `homeassistant/media_player/snapdog_zone_<n>/config` topics)"
            );
        }
        let states = self.publisher.snapshot(&store);
        drop(store);
        if let Some(discovery) = &self.discovery {
            discovery.on_connected(first_connect).await;
        }
        tracing::info!(subscribed, states, "MQTT subscribed and announced");
    }

    async fn on_message(&mut self, msg: &rumqttc::Publish) {
        // Home Assistant's birth message is not a command: it asks for the discovery
        // message again.
        if let Some(discovery) = &self.discovery
            && discovery.is_birth_topic(&msg.topic)
        {
            if msg.payload.as_ref() == b"online" {
                discovery.on_birth();
            }
            return;
        }
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
    fn report_connection_error(&mut self, error: &ConnectionError, retry: Retry) {
        let retry_in = retry.delay;
        if retry.flapping {
            tracing::warn!(
                client_id = %self.client_id,
                "MQTT connection was closed shortly after it was made, again and again. A common \
                 cause is another client with the same client id: the broker drops the older \
                 connection whenever a new one arrives, and the two take turns. Give every \
                 SnapDog instance (and every other MQTT client) its own client_id in [mqtt]"
            );
        }
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
pub async fn run_backlog_flusher(handler: &CommandHandler, shutdown: &mut watch::Receiver<bool>) {
    loop {
        tokio::select! {
            () = shutdown_requested(shutdown) => return,
            () = handler.backlog_added() => {}
        }
        while handler.has_backlog() {
            tokio::select! {
                () = shutdown_requested(shutdown) => return,
                () = tokio::time::sleep(BACKLOG_TICK) => {}
            }
            handler.flush_backlog(Instant::now()).await;
        }
    }
}

/// What the state publisher works with. Kept apart from the task so that it
/// survives a panic of the task.
pub struct StateFeed {
    pub discovery: Option<Discovery>,
    pub publisher: Publisher,
    pub store: state::SharedState,
    pub notifications: broadcast::Receiver<Arc<str>>,
    pub connected: watch::Receiver<bool>,
    pub shutdown: watch::Receiver<bool>,
}

/// Publish retained state when zones or clients change.
///
/// While the broker is away nothing is queued: the snapshot after the next
/// connect replaces it. If this task falls behind the notification channel and
/// notifications are dropped, it re-publishes everything.
pub async fn run_state_publisher(feed: &mut StateFeed) {
    let StateFeed {
        discovery,
        publisher,
        store,
        notifications,
        connected,
        shutdown,
    } = feed;
    loop {
        let received = tokio::select! {
            () = shutdown_requested(shutdown) => break,
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
                        drop(store);
                        // A client that was not there when the entities were announced
                        // (Snapcast accepted it at runtime) gets entities of its own.
                        if let Some(discovery) = discovery {
                            discovery.announce_if_changed().await;
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
        let task = tokio::spawn(supervised_backlog_flusher(handler.clone(), shutdown));

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

    fn quick_backoff() -> Backoff {
        Backoff::new(Duration::from_millis(1), Duration::from_millis(5))
    }

    /// Review finding F6: the tasks were spawned bare, so one panic ended MQTT for
    /// the rest of the process, silently.
    #[tokio::test]
    async fn a_panicking_task_is_restarted_on_the_state_it_left() {
        let (_stop, shutdown) = watch::channel(false);
        let mut panics = 0;
        let state = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
        let seen = state.clone();
        tokio::time::timeout(
            Duration::from_secs(5),
            supervise(
                "test",
                quick_backoff(),
                state,
                shutdown,
                |_| panics += 1,
                async |runs| {
                    let n = runs.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
                    assert!(n >= 3, "first two runs panic");
                },
            ),
        )
        .await
        .expect("restarts until the task returns");
        assert_eq!(seen.load(std::sync::atomic::Ordering::SeqCst), 3);
        assert_eq!(panics, 2, "repair ran after each panic");
    }

    #[tokio::test]
    async fn a_task_that_returns_is_not_restarted() {
        let (_stop, shutdown) = watch::channel(false);
        let mut runs = 0;
        supervise(
            "test",
            quick_backoff(),
            (),
            shutdown,
            |()| panic!("no panic happened"),
            async |()| runs += 1,
        )
        .await;
        assert_eq!(runs, 1);
    }

    #[tokio::test]
    async fn shutdown_ends_the_wait_between_restarts() {
        let (stop, shutdown) = watch::channel(false);
        let slow = Backoff::new(Duration::from_secs(60), Duration::from_secs(60));
        let task = tokio::spawn(supervise(
            "test",
            slow,
            (),
            shutdown,
            |()| {},
            async |()| panic!("always"),
        ));
        tokio::time::sleep(Duration::from_millis(100)).await;
        stop.send_replace(true);
        tokio::time::timeout(Duration::from_secs(2), task)
            .await
            .expect("supervise returns on shutdown, not after the backoff")
            .unwrap();
    }

    #[test]
    fn panic_messages_are_read_from_both_payload_kinds() {
        assert_eq!(panic_message(&"literal"), "literal");
        assert_eq!(panic_message(&String::from("formatted")), "formatted");
        assert_eq!(panic_message(&42_u8), "(no message)");
    }

    /// An event loop and a session wired to a broker on `127.0.0.1:port`, the way
    /// [`MqttBridge::start`](super::super::MqttBridge::start) wires them.
    fn rig(
        port: u16,
        backoff: Backoff,
    ) -> (
        EventLoop,
        Session,
        watch::Receiver<bool>,
        watch::Sender<bool>,
        crate::mqtt::test_support::RuntimeEnds,
    ) {
        use crate::mqtt::test_support::test_runtime;
        use crate::mqtt::topics::Topics;
        use crate::mqtt::transport::mqtt_options;
        use rumqttc::AsyncClient;

        let config: crate::config::MqttConfig =
            toml::from_str(&format!("broker = \"127.0.0.1:{port}\"")).unwrap();
        let topics = Topics::new(&config.base_topic);
        let (client, eventloop) = AsyncClient::new(mqtt_options(&config, &topics).unwrap(), 64);
        let (runtime, ends) = test_runtime();
        let (connected, connected_rx) = watch::channel(false);
        let (stop, shutdown) = watch::channel(false);
        let session = Session {
            publisher: Publisher::new(client, topics.clone(), "http://h"),
            client_id: config.client_id,
            discovery: None,
            clear_legacy: LegacyCleanup::All,
            backoff,
            handler: CommandHandler::new(
                topics,
                runtime.zone_commands,
                runtime.state.clone(),
                runtime.snap_tx,
            ),
            store: runtime.state,
            connected,
            shutdown,
            log_throttle: LogThrottle::new(Duration::from_secs(10)),
        };
        (eventloop, session, connected_rx, stop, ends)
    }

    #[tokio::test]
    async fn after_a_connection_panic_the_bridge_no_longer_claims_to_be_connected() {
        let (eventloop, session, connected_rx, _stop, _ends) = rig(1, quick_backoff());
        session.connected.send_replace(true);
        let mut state = (eventloop, session);
        assert!(*connected_rx.borrow());
        after_connection_panic(&mut state);
        assert!(
            !*connected_rx.borrow(),
            "the flag must not outlive the task"
        );
    }

    /// Review finding F9: two instances with one client id kick each other off the
    /// broker, over and over, and the log only said "connection error".
    #[tokio::test]
    async fn connections_that_keep_closing_soon_get_a_hint_about_the_client_id() {
        use crate::mqtt::test_support::{FakeBroker, Script, capture_logs};
        let (_guard, logs) = capture_logs();
        let broker = FakeBroker::start(Script::DropAfterConnack).await;
        let (mut eventloop, mut session, _connected, stop, _ends) =
            rig(broker.port, quick_backoff());
        let task = tokio::spawn(async move { run_connection(&mut eventloop, &mut session).await });

        let deadline = Instant::now() + Duration::from_secs(10);
        while !logs.text().contains("same client id") && Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        stop.send_replace(true);
        let _ = tokio::time::timeout(Duration::from_secs(2), task).await;
        let text = logs.text();
        assert!(
            text.contains("same client id"),
            "no hint in the log: {text}"
        );
        assert!(text.contains("client_id"), "{text}");
        assert!(
            broker.connections.load(std::sync::atomic::Ordering::SeqCst) >= 3,
            "the hint comes after repeated short connections, not the first"
        );
    }

    /// Review finding F1: the connection ended on a packet over the limit, and the
    /// log said "connection error, retrying".
    #[tokio::test]
    async fn an_oversized_packet_from_the_broker_is_explained_in_the_log() {
        use crate::mqtt::test_support::{FakeBroker, Script, capture_logs};
        let (_guard, logs) = capture_logs();
        let broker = FakeBroker::start(Script::SendOversized(2 << 20)).await;
        let (mut eventloop, mut session, _connected, stop, _ends) =
            rig(broker.port, quick_backoff());
        let task = tokio::spawn(async move { run_connection(&mut eventloop, &mut session).await });

        let deadline = Instant::now() + Duration::from_secs(10);
        while !logs.text().contains("somebody published below") && Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        stop.send_replace(true);
        let _ = tokio::time::timeout(Duration::from_secs(2), task).await;
        let text = logs.text();
        assert!(
            text.contains("somebody published below"),
            "no hint in the log: {text}"
        );
        assert!(text.contains("snapdog/"), "names the base topic: {text}");
        assert!(text.contains("ERROR"), "{text}");
        // It repeats on every retry, so it is throttled: one line, not one per attempt.
        assert_eq!(
            text.matches("somebody published below").count(),
            1,
            "{text}"
        );
    }

    // ── Reconnect bookkeeping ─────────────────────────────────

    fn reconnect() -> Reconnect {
        Reconnect::new(Backoff::new(RECONNECT_MIN, RECONNECT_MAX))
    }

    /// The delay in whole seconds for the worst-case jitter sample (1.0).
    fn worst_secs(retry: Retry) -> u64 {
        retry.delay.as_secs()
    }

    #[test]
    fn failed_attempts_without_a_connection_back_off_and_never_look_like_flapping() {
        let t0 = Instant::now();
        let mut r = reconnect();
        let delays: Vec<u64> = (0..5)
            .map(|i| {
                let retry = r.failed(t0 + Duration::from_secs(i), 1.0);
                assert!(!retry.flapping, "a refused connection is not a flap");
                worst_secs(retry)
            })
            .collect();
        assert_eq!(delays, [3, 9, 27, 60, 60]);
    }

    /// Mutation M10 of the review: resetting the backoff after every connection
    /// made two clients with one client id reconnect once per second, for ever.
    #[test]
    fn a_connection_that_closes_soon_does_not_reset_the_backoff() {
        let t0 = Instant::now();
        let mut r = reconnect();
        let mut delays = Vec::new();
        for round in 0..4 {
            let up = t0 + Duration::from_secs(round * 100);
            r.connected(up);
            delays.push(worst_secs(r.failed(up + Duration::from_secs(1), 1.0)));
        }
        assert_eq!(delays, [3, 9, 27, 60], "the backoff keeps growing");
    }

    #[test]
    fn a_connection_that_held_for_a_while_starts_the_backoff_over() {
        let t0 = Instant::now();
        let mut r = reconnect();
        for i in 0..4 {
            r.failed(t0 + Duration::from_secs(i), 1.0);
        }
        r.connected(t0 + Duration::from_secs(10));
        let retry = r.failed(t0 + Duration::from_secs(10) + STABLE_AFTER, 1.0);
        assert_eq!(worst_secs(retry), 3, "healthy for 30 s: start over");
        // Just under the threshold is still a short connection.
        let up = t0 + Duration::from_secs(1000);
        r.connected(up);
        let retry = r.failed(
            up + STABLE_AFTER.checked_sub(Duration::from_millis(1)).unwrap(),
            1.0,
        );
        assert_eq!(worst_secs(retry), 9);
    }

    #[test]
    fn the_flapping_hint_comes_after_three_short_connections_then_rarely() {
        let t0 = Instant::now();
        let mut r = reconnect();
        let mut hints = Vec::new();
        for round in 1..=41_u64 {
            let up = t0 + Duration::from_secs(round * 100);
            r.connected(up);
            if r.failed(up + Duration::from_secs(1), 0.0).flapping {
                hints.push(round);
            }
        }
        assert_eq!(hints, [3, 20, 40]);
        // A healthy connection ends the series.
        let up = t0 + Duration::from_secs(10_000);
        r.connected(up);
        assert!(!r.failed(up + STABLE_AFTER, 0.0).flapping);
        for _ in 0..2 {
            r.connected(up);
            assert!(!r.failed(up + Duration::from_secs(1), 0.0).flapping);
        }
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
