// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! Inbound commands: topic and payload to zone or Snapcast command.
//!
//! Zone commands go through the per-zone `ZonePlayer` channel, client commands
//! to the Snapcast command channel. Nothing here blocks: a full channel does not
//! hold up the MQTT event loop (and with it the keep-alive).
//!
//! Every message passes the same gates, cheapest first:
//!
//! 1. a **retained** message is ignored: a retained command would run again on
//!    every restart, long after anyone meant it;
//! 2. the payload must be small ([`MAX_PAYLOAD_BYTES`]) and UTF-8;
//! 3. the topic must be a command below our base topic;
//! 4. the payload must be understood exactly (see [`super::parse`]);
//! 5. the addressed zone or client is **rate limited**.
//!
//! **Nothing is lost to a flood but the intermediate values.** A command that
//! sets a value (volume, mute, position, ...) that cannot be delivered right now,
//! because the budget of its zone or client is used up or the player's channel is
//! full, is kept in a backlog that holds the *latest* value per zone or client and
//! command. [`CommandHandler::flush_backlog`] delivers it as soon as there is room,
//! so the last position of a dragged slider is the one that arrives. A backlog
//! entry older than [`BACKLOG_TTL`] is dropped: a value that waited that long is
//! not what anybody wants applied any more. Transport commands (`play`, `next`,
//! ...) are events, not values: two `next` are not one, so they are dropped
//! instead of coalesced when they cannot be delivered.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use tokio::sync::{Notify, mpsc};

use super::limits::RateLimiter;
use super::parse::{
    MAX_PAYLOAD_BYTES, PayloadError, Transport, parse_bool, parse_client_volume, parse_count,
    parse_latency_ms, parse_repeat, parse_seconds, parse_seek_ms, parse_transport,
    parse_zone_volume,
};
use super::topics::{ParsedCommand, Route, Scope, TopicError, Topics};
use crate::player::{ClientAction, SnapcastCmd, ZoneCommand, ZoneCommandSender};
use crate::state;

/// How long a value waits in the backlog before it is dropped as stale.
pub const BACKLOG_TTL: Duration = Duration::from_secs(5);

/// What became of a message that was not rejected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Handled {
    /// A command was queued.
    Dispatched,
    /// A newer value for the same setting arrived first, so this one was dropped.
    Superseded,
    /// A value could not be delivered yet (budget used up, channel full): the latest
    /// one per zone or client and command waits in the backlog.
    Deferred,
    /// Retained messages are never executed.
    IgnoredRetained,
    /// A transport command beyond the budget of its zone or client, dropped.
    RateLimited,
    /// Below the base topic, but not one of our command topics.
    NotACommand,
    /// The client exists but is not connected to Snapcast, so there is nobody to tell.
    ClientOffline,
}

/// Why a message was refused. Never carries the payload.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rejected {
    Topic(TopicError),
    PayloadTooLarge,
    PayloadNotUtf8,
    Payload(PayloadError),
    /// No such zone or client.
    NoSuchTarget,
    /// The command channel is full or closed.
    NotDelivered,
}

impl std::fmt::Display for Rejected {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Topic(e) => e.fmt(f),
            Self::PayloadTooLarge => f.write_str("payload is too large"),
            Self::PayloadNotUtf8 => f.write_str("payload is not valid UTF-8"),
            Self::Payload(e) => e.fmt(f),
            Self::NoSuchTarget => f.write_str("no such zone or client"),
            Self::NotDelivered => f.write_str("command channel is full or closed"),
        }
    }
}

impl std::error::Error for Rejected {}

impl From<PayloadError> for Rejected {
    fn from(e: PayloadError) -> Self {
        Self::Payload(e)
    }
}

impl Rejected {
    /// A stable name for log throttling.
    pub const fn category(&self) -> &'static str {
        match self {
            Self::Topic(_) => "rejected: topic",
            Self::PayloadTooLarge | Self::PayloadNotUtf8 | Self::Payload(_) => "rejected: payload",
            Self::NoSuchTarget => "rejected: no such target",
            Self::NotDelivered => "rejected: not delivered",
        }
    }
}

/// What a command does, once its payload is understood.
enum Delivery {
    Zone(ZoneCommand),
    Client(ClientAction),
    /// Move the client to this zone (1-based).
    MoveClient(usize),
}

/// A parsed command, ready to be delivered now or later.
struct Job {
    target: ParsedCommand,
    delivery: Delivery,
    /// Arrival number: a later command for the same slot has a higher one, and an
    /// older one is never delivered after it ([`Gate::superseded`]).
    seq: u64,
}

impl Job {
    /// One backlog slot per zone or client and command: the latest value wins.
    const fn key(&self) -> (Scope, usize, Route) {
        (self.target.scope, self.target.index, self.target.route)
    }
}

/// The result of an attempt to deliver a [`Job`].
enum Attempt {
    /// Delivered (or there was nobody to deliver to).
    Done(Handled),
    /// The channel is full: the job is handed back.
    Full(Job),
}

/// A value that waits for room.
struct Waiting {
    job: Job,
    since: Instant,
}

/// The rate limiter and the backlog. One lock, held only for bookkeeping and never
/// across an `.await`.
struct Gate {
    limiter: RateLimiter,
    backlog: HashMap<(Scope, usize, Route), Waiting>,
    next_seq: u64,
    /// The arrival number of the newest value of every slot (zone or client, and
    /// setting). Bounded by the number of slots.
    latest: HashMap<(Scope, usize, Route), u64>,
    /// Test only: the arrival numbers in the order they were really delivered.
    #[cfg(test)]
    delivered: Vec<((Scope, usize, Route), u64)>,
}

impl Gate {
    /// Test only: note that `seq` was really delivered, in delivery order.
    #[cfg(test)]
    fn record_delivery(&mut self, target: &ParsedCommand, seq: u64) {
        self.delivered
            .push(((target.scope, target.index, target.route), seq));
    }

    /// Whether a newer value for the slot of `job` has arrived since it did. Only
    /// values (not events) are ever superseded.
    fn superseded(&self, target: &ParsedCommand, seq: u64) -> bool {
        target.route.is_level()
            && self.latest.get(&(target.scope, target.index, target.route)) != Some(&seq)
    }

    /// Keep `job` for later, unless a newer value has overtaken it.
    fn defer(&mut self, job: Job, since: Instant) {
        if !self.superseded(&job.target, job.seq) {
            self.backlog.insert(job.key(), Waiting { job, since });
        }
    }
}

/// Turns incoming MQTT messages into commands. Cheap to clone: clones share the
/// rate limiter and the backlog.
#[derive(Clone)]
pub struct CommandHandler {
    topics: Topics,
    zone_commands: HashMap<usize, ZoneCommandSender>,
    state: state::SharedState,
    snap_tx: mpsc::Sender<SnapcastCmd>,
    gate: Arc<Mutex<Gate>>,
    /// One zone change at a time: it writes the shared state before it queues the
    /// reconcile, so two of them must not interleave.
    move_serial: Arc<tokio::sync::Mutex<()>>,
    /// Wakes the task that flushes the backlog.
    wake: Arc<Notify>,
}

impl CommandHandler {
    /// A handler for the topics below `topics`.
    pub fn new(
        topics: Topics,
        zone_commands: HashMap<usize, ZoneCommandSender>,
        state: state::SharedState,
        snap_tx: mpsc::Sender<SnapcastCmd>,
    ) -> Self {
        Self {
            topics,
            zone_commands,
            state,
            snap_tx,
            gate: Arc::new(Mutex::new(Gate {
                limiter: RateLimiter::new(Instant::now()),
                backlog: HashMap::new(),
                next_seq: 0,
                latest: HashMap::new(),
                #[cfg(test)]
                delivered: Vec::new(),
            })),
            move_serial: Arc::new(tokio::sync::Mutex::new(())),
            wake: Arc::new(Notify::new()),
        }
    }

    fn gate(&self) -> MutexGuard<'_, Gate> {
        // The lock guards plain bookkeeping: a panic elsewhere cannot leave it
        // half-updated in a way that matters, so a poisoned lock is still usable.
        self.gate
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Whether a value waits for room.
    pub fn has_backlog(&self) -> bool {
        !self.gate().backlog.is_empty()
    }

    /// Resolves when a value has been put into the backlog.
    pub async fn backlog_added(&self) {
        self.wake.notified().await;
    }

    /// Handle one incoming message.
    ///
    /// # Errors
    ///
    /// See [`Rejected`].
    pub async fn handle(
        &self,
        topic: &str,
        payload: &[u8],
        retain: bool,
    ) -> Result<Handled, Rejected> {
        self.handle_at(topic, payload, retain, Instant::now()).await
    }

    /// [`Self::handle`] with the clock as a parameter.
    ///
    /// # Errors
    ///
    /// See [`Rejected`].
    pub async fn handle_at(
        &self,
        topic: &str,
        payload: &[u8],
        retain: bool,
        now: Instant,
    ) -> Result<Handled, Rejected> {
        if retain {
            return Ok(Handled::IgnoredRetained);
        }
        if payload.len() > MAX_PAYLOAD_BYTES {
            return Err(Rejected::PayloadTooLarge);
        }
        let payload = std::str::from_utf8(payload).map_err(|_| Rejected::PayloadNotUtf8)?;
        let target = match self.topics.parse_command(topic) {
            Ok(parsed) => parsed,
            Err(TopicError::UnknownRoute) => return Ok(Handled::NotACommand),
            Err(e) => return Err(Rejected::Topic(e)),
        };
        let mut job = Job {
            delivery: plan(target.route, payload)?,
            target,
            seq: 0,
        };

        {
            let mut gate = self.gate();
            gate.next_seq += 1;
            job.seq = gate.next_seq;
            if target.route.is_level() {
                gate.latest.insert(job.key(), job.seq);
            }
            // This value replaces whatever older one still waits for the same slot,
            // whether or not it can be delivered right now: the old one must never
            // arrive after the new one.
            gate.backlog.remove(&job.key());
            if !gate.limiter.allow(target.scope, target.index, now) {
                if !target.route.is_level() {
                    return Ok(Handled::RateLimited);
                }
                gate.defer(job, now);
                drop(gate);
                self.wake.notify_one();
                return Ok(Handled::Deferred);
            }
        }
        match self.deliver(job).await? {
            Attempt::Done(handled) => Ok(handled),
            Attempt::Full(job) if job.target.route.is_level() => {
                // (`defer` drops it if a newer value has overtaken it meanwhile.)
                self.gate().defer(job, now);
                self.wake.notify_one();
                Ok(Handled::Deferred)
            }
            Attempt::Full(_) => Err(Rejected::NotDelivered),
        }
    }

    /// Deliver the values that waited for room, as far as the rate limit and the
    /// channels allow, and drop the ones that waited too long.
    pub async fn flush_backlog(&self, now: Instant) {
        let ready: Vec<Waiting> = {
            let mut gate = self.gate();
            gate.backlog
                .retain(|_, waiting| now.saturating_duration_since(waiting.since) < BACKLOG_TTL);
            let mut keys: Vec<_> = gate
                .backlog
                .iter()
                .map(|(key, waiting)| (waiting.job.seq, *key))
                .collect();
            keys.sort_unstable_by_key(|(seq, _)| *seq);
            let mut ready = Vec::new();
            for (_, key) in keys {
                if gate.limiter.allow(key.0, key.1, now)
                    && let Some(waiting) = gate.backlog.remove(&key)
                {
                    ready.push(waiting);
                }
            }
            ready
        };
        for waiting in ready {
            let Waiting { job, since } = waiting;
            match self.deliver(job).await {
                Ok(Attempt::Full(job)) => {
                    // Still no room. A newer value that arrived meanwhile stays (and
                    // `defer` drops this one if it has been overtaken).
                    #[cfg_attr(not(test), allow(unused_mut, unused_variables))]
                    let mut gate = self.gate();
                    if !gate.backlog.contains_key(&job.key()) {
                        gate.defer(job, since);
                    }
                }
                // Delivered, or the target is gone: nothing left to do.
                Ok(Attempt::Done(_)) | Err(_) => {}
            }
        }
    }

    /// Try to deliver `job` without waiting.
    ///
    /// A value is delivered only if no newer one has arrived for its slot, and that is
    /// checked under the same lock as the send, so that an older value can never
    /// slip in behind a newer one (two deliveries for one slot cannot interleave).
    // The lock is held across the send on purpose: that is what makes the check and the
    // send one step.
    #[allow(clippy::significant_drop_tightening)]
    async fn deliver(&self, job: Job) -> Result<Attempt, Rejected> {
        let index = job.target.index;
        match job.delivery {
            Delivery::Zone(cmd) => {
                let tx = self
                    .zone_commands
                    .get(&index)
                    .ok_or(Rejected::NoSuchTarget)?;
                #[cfg_attr(not(test), allow(unused_mut, unused_variables))]
                let mut gate = self.gate();
                if gate.superseded(&job.target, job.seq) {
                    return Ok(Attempt::Done(Handled::Superseded));
                }
                match tx.try_send(cmd) {
                    Ok(()) => {
                        #[cfg(test)]
                        gate.record_delivery(&job.target, job.seq);
                        Ok(Attempt::Done(Handled::Dispatched))
                    }
                    Err(mpsc::error::TrySendError::Full(cmd)) => Ok(Attempt::Full(Job {
                        delivery: Delivery::Zone(cmd),
                        ..job
                    })),
                    Err(mpsc::error::TrySendError::Closed(_)) => Err(Rejected::NotDelivered),
                }
            }
            Delivery::Client(action) => {
                let snap_id = self
                    .state
                    .read()
                    .await
                    .clients
                    .get(&index)
                    .map(|client| client.snapcast_id.clone())
                    .ok_or(Rejected::NoSuchTarget)?;
                let Some(client_id) = snap_id else {
                    return Ok(Attempt::Done(Handled::ClientOffline));
                };
                #[cfg_attr(not(test), allow(unused_mut, unused_variables))]
                let mut gate = self.gate();
                if gate.superseded(&job.target, job.seq) {
                    return Ok(Attempt::Done(Handled::Superseded));
                }
                match self
                    .snap_tx
                    .try_send(SnapcastCmd::Client { client_id, action })
                {
                    Ok(()) => {
                        #[cfg(test)]
                        gate.record_delivery(&job.target, job.seq);
                        Ok(Attempt::Done(Handled::Dispatched))
                    }
                    Err(mpsc::error::TrySendError::Full(SnapcastCmd::Client {
                        action, ..
                    })) => Ok(Attempt::Full(Job {
                        delivery: Delivery::Client(action),
                        ..job
                    })),
                    Err(_) => Err(Rejected::NotDelivered),
                }
            }
            Delivery::MoveClient(zone) => {
                // The state write and the reconcile of one move are not interleaved
                // with another move's.
                let _serial = self.move_serial.lock().await;
                if self.gate().superseded(&job.target, job.seq) {
                    return Ok(Attempt::Done(Handled::Superseded));
                }
                self.move_client(index, zone).await?;
                #[cfg_attr(not(test), allow(unused_mut, unused_variables))]
                let mut gate = self.gate();
                match self.snap_tx.try_send(SnapcastCmd::ReconcileZones) {
                    Ok(()) => {
                        #[cfg(test)]
                        gate.record_delivery(&job.target, job.seq);
                        Ok(Attempt::Done(Handled::Dispatched))
                    }
                    Err(mpsc::error::TrySendError::Full(_)) => Ok(Attempt::Full(Job {
                        delivery: Delivery::MoveClient(zone),
                        ..job
                    })),
                    Err(mpsc::error::TrySendError::Closed(_)) => Err(Rejected::NotDelivered),
                }
            }
        }
    }

    /// Assign client `index` to zone `zone` in the shared state. Idempotent: a
    /// delivery that is retried applies it again.
    async fn move_client(&self, index: usize, zone: usize) -> Result<(), Rejected> {
        let mut store = self.state.write().await;
        if zone == 0 || !store.zones.contains_key(&zone) {
            return Err(Rejected::NoSuchTarget);
        }
        let client = store
            .clients
            .get_mut(&index)
            .ok_or(Rejected::NoSuchTarget)?;
        client.zone_index = zone;
        store.dirty = true;
        drop(store);
        Ok(())
    }
}

/// Understand `payload` for `route`.
fn plan(route: Route, payload: &str) -> Result<Delivery, Rejected> {
    Ok(match route {
        // Zone commands → routed through ZonePlayer
        Route::ZoneVolume => Delivery::Zone(ZoneCommand::SetVolume(parse_zone_volume(payload)?)),
        Route::ZoneMute => Delivery::Zone(ZoneCommand::SetMute(parse_bool(payload)?)),
        Route::ZoneControl => Delivery::Zone(match parse_transport(payload)? {
            Transport::Play => ZoneCommand::Play,
            Transport::Pause => ZoneCommand::Pause,
            Transport::Stop => ZoneCommand::Stop,
            Transport::Next => ZoneCommand::Next,
            Transport::Previous => ZoneCommand::Previous,
        }),
        Route::ZonePlaylist => Delivery::Zone(ZoneCommand::SetPlaylist(parse_count(payload)?, 0)),
        Route::ZoneTrack => Delivery::Zone(ZoneCommand::SetTrack(parse_count(payload)?)),
        Route::ZonePosition => Delivery::Zone(ZoneCommand::Seek(parse_seek_ms(payload)?)),
        Route::ZoneShuffle => Delivery::Zone(ZoneCommand::SetShuffle(parse_bool(payload)?)),
        Route::ZoneRepeat => Delivery::Zone(ZoneCommand::SetRepeat(parse_repeat(payload)?)),
        Route::ZonePresence => Delivery::Zone(ZoneCommand::SetPresence(parse_bool(payload)?)),
        Route::ZonePresenceEnable => {
            Delivery::Zone(ZoneCommand::SetPresenceEnabled(parse_bool(payload)?))
        }
        Route::ZonePresenceTimeout => {
            Delivery::Zone(ZoneCommand::SetAutoOffDelay(parse_seconds(payload)?))
        }

        // Client commands → Snapcast commands or direct state mutation
        Route::ClientVolume => {
            Delivery::Client(ClientAction::Volume(parse_client_volume(payload)?))
        }
        Route::ClientMute => Delivery::Client(ClientAction::Mute(parse_bool(payload)?)),
        Route::ClientLatency => Delivery::Client(ClientAction::Latency(parse_latency_ms(payload)?)),
        Route::ClientZone => Delivery::MoveClient(parse_count(payload)?),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mqtt::test_support::{
        snap_channel, test_state_with_client, zone_channels, zone_channels_with,
    };
    use std::sync::Arc;
    use std::time::Duration;

    /// A handler on `base` plus the receivers of its channels.
    fn handler(
        base: &str,
    ) -> (
        CommandHandler,
        mpsc::Receiver<ZoneCommand>,
        mpsc::Receiver<SnapcastCmd>,
        state::SharedState,
    ) {
        let (snap_tx, snap_rx) = snap_channel();
        let (cmds, zone_rx) = zone_channels();
        let state = test_state_with_client();
        (
            CommandHandler::new(Topics::new(base), cmds, state.clone(), snap_tx),
            zone_rx,
            snap_rx,
            state,
        )
    }

    /// Deliver a live (not retained) message.
    async fn send(h: &CommandHandler, topic: &str, payload: &str) -> Result<Handled, Rejected> {
        h.handle(topic, payload.as_bytes(), false).await
    }

    #[tokio::test]
    async fn routes_zone_volume() {
        let (h, mut rx, _snap, _state) = handler("snapdog/");
        let r = send(&h, "snapdog/zones/1/volume/set", "75").await;
        assert_eq!(r, Ok(Handled::Dispatched));
        assert!(matches!(rx.recv().await, Some(ZoneCommand::SetVolume(75))));
    }

    #[tokio::test]
    async fn zone_volume_takes_both_scales() {
        let (h, mut rx, _snap, _state) = handler("snapdog/");
        for (payload, expected) in [("0.5", 50), ("50", 50), ("1", 100), ("0.01", 1), ("1.5", 1)] {
            send(&h, "snapdog/zones/1/volume/set", payload)
                .await
                .unwrap();
            assert!(
                matches!(rx.recv().await, Some(ZoneCommand::SetVolume(v)) if v == expected),
                "{payload}"
            );
        }
    }

    #[tokio::test]
    async fn routes_zone_mute() {
        let (h, mut rx, _snap, _state) = handler("snapdog/");
        send(&h, "snapdog/zones/1/mute/set", "true").await.unwrap();
        assert!(matches!(rx.recv().await, Some(ZoneCommand::SetMute(true))));
        send(&h, "snapdog/zones/1/mute/set", "OFF").await.unwrap();
        assert!(matches!(rx.recv().await, Some(ZoneCommand::SetMute(false))));
    }

    #[tokio::test]
    async fn on_means_mute_in_any_case() {
        // Home Assistant's switch sends "ON"; it used to be read as "unmute".
        let (h, mut rx, _snap, _state) = handler("snapdog/");
        send(&h, "snapdog/zones/1/mute/set", "ON").await.unwrap();
        assert!(matches!(rx.recv().await, Some(ZoneCommand::SetMute(true))));
    }

    #[tokio::test]
    async fn every_command_family_is_strict_about_booleans() {
        // Zone mute and shuffle were lenient (anything but true/1/on meant off),
        // client mute and presence strict. All of them are strict now.
        let (h, mut rx, mut snap_rx, _state) = handler("snapdog/");
        for topic in [
            "snapdog/zones/1/mute/set",
            "snapdog/zones/1/shuffle/set",
            "snapdog/zones/1/presence/set",
            "snapdog/zones/1/presence/enable/set",
            "snapdog/clients/1/mute/set",
        ] {
            for bad in ["", "maybe", "unmute", "2"] {
                assert!(
                    matches!(send(&h, topic, bad).await, Err(Rejected::Payload(_))),
                    "{topic} {bad:?}"
                );
            }
            // Understood spellings pass on every one of them. The limiter is not
            // what this test is about, so give each spelling its own time.
            for (i, good) in ["true", "ON", "1", "false", "off", "0"].iter().enumerate() {
                let now = Instant::now() + Duration::from_secs(i as u64 + 1);
                assert_eq!(
                    h.handle_at(topic, good.as_bytes(), false, now).await,
                    Ok(Handled::Dispatched),
                    "{topic} {good}"
                );
            }
            while rx.try_recv().is_ok() {}
            while snap_rx.try_recv().is_ok() {}
        }
    }

    #[tokio::test]
    async fn a_rejected_payload_changes_nothing() {
        let (h, mut rx, mut snap_rx, state) = handler("snapdog/");
        for (topic, payload) in [
            ("snapdog/zones/1/volume/set", "NaN"),
            ("snapdog/zones/1/volume/set", "loud"),
            ("snapdog/zones/1/repeat/set", "sideways"),
            ("snapdog/zones/1/control/set", "explode"),
            ("snapdog/zones/1/position/set", "-5"),
            ("snapdog/zones/1/track/set", "-1"),
            ("snapdog/clients/1/latency/set", "99999"),
            ("snapdog/clients/1/volume/set", "0.5"),
            ("snapdog/clients/1/zone/set", "x"),
        ] {
            assert!(
                matches!(send(&h, topic, payload).await, Err(Rejected::Payload(_))),
                "{topic} {payload}"
            );
        }
        assert!(rx.try_recv().is_err() && snap_rx.try_recv().is_err());
        assert_eq!(state.read().await.clients[&1].zone_index, 1);
    }

    #[tokio::test]
    async fn routes_zone_control_commands() {
        let (h, mut rx, _snap, _state) = handler("snapdog/");
        for (payload, expected) in [
            ("play", ZoneCommand::Play),
            ("pause", ZoneCommand::Pause),
            ("STOP", ZoneCommand::Stop),
            ("next\n", ZoneCommand::Next),
            ("previous", ZoneCommand::Previous),
        ] {
            send(&h, "snapdog/zones/1/control/set", payload)
                .await
                .unwrap();
            let cmd = rx.recv().await.unwrap();
            assert_eq!(
                std::mem::discriminant(&cmd),
                std::mem::discriminant(&expected)
            );
        }
    }

    #[tokio::test]
    async fn routes_zone_playlist_and_track() {
        let (h, mut rx, _snap, _state) = handler("snapdog/");
        send(&h, "snapdog/zones/1/playlist/set", "3").await.unwrap();
        assert!(matches!(
            rx.recv().await,
            Some(ZoneCommand::SetPlaylist(3, 0))
        ));
        send(&h, "snapdog/zones/1/track/set", "7").await.unwrap();
        assert!(matches!(rx.recv().await, Some(ZoneCommand::SetTrack(7))));
    }

    #[tokio::test]
    async fn routes_zone_seek_on_both_topics() {
        let (h, mut rx, _snap, _state) = handler("snapdog/");
        for topic in [
            "snapdog/zones/1/track/position/set",
            "snapdog/zones/1/position/set",
        ] {
            send(&h, topic, "30").await.unwrap();
            assert!(matches!(rx.recv().await, Some(ZoneCommand::Seek(30000))));
        }
    }

    #[tokio::test]
    async fn routes_zone_repeat_and_presence() {
        let (h, mut rx, _snap, _state) = handler("snapdog/");
        send(&h, "snapdog/zones/1/repeat/set", "all").await.unwrap();
        assert!(matches!(
            rx.recv().await,
            Some(ZoneCommand::SetRepeat(snapdog_common::RepeatMode::Playlist))
        ));
        send(&h, "snapdog/zones/1/presence/timeout/set", "90")
            .await
            .unwrap();
        assert!(matches!(
            rx.recv().await,
            Some(ZoneCommand::SetAutoOffDelay(90))
        ));
    }

    #[tokio::test]
    async fn routes_client_volume_mute_and_latency() {
        let (h, _rx, mut snap_rx, _state) = handler("snapdog/");
        send(&h, "snapdog/clients/1/volume/set", "80")
            .await
            .unwrap();
        assert!(matches!(
            snap_rx.recv().await,
            Some(SnapcastCmd::Client { client_id, action: ClientAction::Volume(80) }) if client_id == "snap-1"
        ));
        send(&h, "snapdog/clients/1/mute/set", "true")
            .await
            .unwrap();
        assert!(matches!(
            snap_rx.recv().await,
            Some(SnapcastCmd::Client { client_id, action: ClientAction::Mute(true) }) if client_id == "snap-1"
        ));
        send(&h, "snapdog/clients/1/latency/set", "-20")
            .await
            .unwrap();
        assert!(matches!(
            snap_rx.recv().await,
            Some(SnapcastCmd::Client {
                action: ClientAction::Latency(-20),
                ..
            })
        ));
    }

    #[tokio::test]
    async fn clamps_client_volume() {
        let (h, _rx, mut snap_rx, _state) = handler("snapdog/");
        send(&h, "snapdog/clients/1/volume/set", "200")
            .await
            .unwrap();
        assert!(matches!(
            snap_rx.recv().await,
            Some(SnapcastCmd::Client {
                action: ClientAction::Volume(100),
                ..
            })
        ));
    }

    #[tokio::test]
    async fn routes_client_zone_change() {
        let (h, _rx, mut snap_rx, state) = handler("snapdog/");
        send(&h, "snapdog/clients/1/zone/set", "2").await.unwrap();
        assert_eq!(state.read().await.clients[&1].zone_index, 2);
        assert!(matches!(
            snap_rx.recv().await,
            Some(SnapcastCmd::ReconcileZones)
        ));
    }

    #[tokio::test]
    async fn client_zone_change_to_an_unknown_zone_or_client_is_refused() {
        let (h, _rx, mut snap_rx, state) = handler("snapdog/");
        for (topic, payload) in [
            ("snapdog/clients/1/zone/set", "99"),
            ("snapdog/clients/1/zone/set", "0"),
            ("snapdog/clients/9/zone/set", "2"),
        ] {
            assert_eq!(
                send(&h, topic, payload).await,
                Err(Rejected::NoSuchTarget),
                "{topic} {payload}"
            );
        }
        assert_eq!(state.read().await.clients[&1].zone_index, 1);
        assert!(snap_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn a_command_for_an_unknown_target_is_an_error_not_a_silent_drop() {
        let (h, _rx, _snap, _state) = handler("snapdog/");
        assert_eq!(
            send(&h, "snapdog/zones/2/volume/set", "10").await,
            Err(Rejected::NoSuchTarget),
            "zone 2 has no player in this fixture"
        );
        assert_eq!(
            send(&h, "snapdog/clients/5/volume/set", "10").await,
            Err(Rejected::NoSuchTarget)
        );
    }

    #[tokio::test]
    async fn a_client_without_snapcast_id_is_offline_not_an_error() {
        let (h, _rx, mut snap_rx, state) = handler("snapdog/");
        state.write().await.clients.get_mut(&1).unwrap().snapcast_id = None;
        assert_eq!(
            send(&h, "snapdog/clients/1/volume/set", "10").await,
            Ok(Handled::ClientOffline)
        );
        assert!(snap_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn ignores_unknown_topic_below_the_base() {
        let (h, mut rx, mut snap_rx, _state) = handler("snapdog/");
        assert_eq!(
            send(&h, "snapdog/unknown/topic", "x").await,
            Ok(Handled::NotACommand)
        );
        assert!(rx.try_recv().is_err() && snap_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn rejects_topic_outside_the_base() {
        let (h, _rx, _snap, _state) = handler("snapdog/");
        for topic in ["other/zones/1/volume/set", "snapdogs/zones/1/volume/set"] {
            assert_eq!(
                send(&h, topic, "50").await,
                Err(Rejected::Topic(TopicError::OutsideBase)),
                "{topic}"
            );
        }
    }

    // ── Gates ─────────────────────────────────────────────────

    #[tokio::test]
    async fn retained_commands_are_never_executed() {
        // A retained message on a `/set` topic is replayed by the broker on every
        // subscribe, i.e. on every restart and every reconnect.
        let (h, mut rx, mut snap_rx, _state) = handler("snapdog/");
        for topic in [
            "snapdog/zones/1/volume/set",
            "snapdog/zones/1/control/set",
            "snapdog/clients/1/mute/set",
        ] {
            assert_eq!(
                h.handle(topic, b"1", true).await,
                Ok(Handled::IgnoredRetained),
                "{topic}"
            );
        }
        // Not even a rejected payload or a foreign topic is looked at.
        assert_eq!(
            h.handle("snapdog/zones/1/volume/set", b"\xff", true).await,
            Ok(Handled::IgnoredRetained)
        );
        assert!(rx.try_recv().is_err() && snap_rx.try_recv().is_err());
        // The same message, live, is executed.
        assert_eq!(
            h.handle("snapdog/zones/1/control/set", b"play", false)
                .await,
            Ok(Handled::Dispatched)
        );
        assert!(matches!(rx.try_recv(), Ok(ZoneCommand::Play)));
    }

    #[tokio::test]
    async fn oversized_and_non_utf8_payloads_are_refused() {
        let (h, mut rx, _snap, _state) = handler("snapdog/");
        let topic = "snapdog/zones/1/volume/set";
        let big = vec![b'5'; MAX_PAYLOAD_BYTES + 1];
        assert_eq!(
            h.handle(topic, &big, false).await,
            Err(Rejected::PayloadTooLarge)
        );
        let mut at_limit = vec![b' '; MAX_PAYLOAD_BYTES - 2];
        at_limit.extend_from_slice(b"50");
        assert_eq!(
            h.handle(topic, &at_limit, false).await,
            Ok(Handled::Dispatched),
            "exactly at the limit still works"
        );
        assert_eq!(
            h.handle(topic, b"5\xff0", false).await,
            Err(Rejected::PayloadNotUtf8)
        );
        assert!(matches!(rx.try_recv(), Ok(ZoneCommand::SetVolume(50))));
        assert!(rx.try_recv().is_err());
    }

    /// A handler whose zone channel holds `capacity` commands, plus the receiver.
    fn handler_with_zone_capacity(
        capacity: usize,
    ) -> (CommandHandler, mpsc::Receiver<ZoneCommand>) {
        let (snap_tx, _snap_rx) = snap_channel();
        let (cmds, rx) = zone_channels_with(capacity);
        let h = CommandHandler::new(
            Topics::new("snapdog/"),
            cmds,
            test_state_with_client(),
            snap_tx,
        );
        (h, rx)
    }

    /// The volumes in `rx`, in the order they arrive.
    fn volumes(rx: &mut mpsc::Receiver<ZoneCommand>) -> Vec<i32> {
        let mut seen = Vec::new();
        while let Ok(cmd) = rx.try_recv() {
            if let ZoneCommand::SetVolume(v) = cmd {
                seen.push(v);
            }
        }
        seen
    }

    #[tokio::test]
    async fn a_flood_on_one_zone_is_bounded_and_the_last_value_still_arrives() {
        // Review finding F5: past the budget the newest command was dropped, so the
        // final position of a dragged slider was the one that got lost.
        let (h, mut rx) = handler_with_zone_capacity(1000);
        let t0 = Instant::now();
        let (mut dispatched, mut deferred) = (0, 0);
        for volume in 1..=1000 {
            match h
                .handle_at(
                    "snapdog/zones/1/volume/set",
                    format!("{}", 2 + volume % 98).as_bytes(),
                    false,
                    t0,
                )
                .await
            {
                Ok(Handled::Dispatched) => dispatched += 1,
                Ok(Handled::Deferred) => deferred += 1,
                other => panic!("{other:?}"),
            }
        }
        assert_eq!(dispatched, 30, "one burst gets through the limiter");
        assert_eq!(deferred, 970);
        assert_eq!(volumes(&mut rx).len(), 30, "the zone is not flooded");

        // The 1000th command asked for 2 + 1000 % 98 = 22; it waits, and arrives
        // once the budget has refilled. The 969 before it are superseded.
        assert!(h.has_backlog());
        h.flush_backlog(t0 + Duration::from_secs(1)).await;
        assert_eq!(volumes(&mut rx), [22]);
        assert!(!h.has_backlog());
    }

    #[tokio::test]
    async fn the_backlog_holds_one_value_per_slot_however_many_arrive() {
        let (h, _rx) = handler_with_zone_capacity(1);
        let t0 = Instant::now();
        for _ in 0..10_000 {
            let _ = h
                .handle_at("snapdog/zones/1/volume/set", b"50", false, t0)
                .await;
            let _ = h
                .handle_at("snapdog/zones/1/mute/set", b"on", false, t0)
                .await;
        }
        assert_eq!(h.gate().backlog.len(), 2, "volume and mute, one each");
    }

    #[tokio::test]
    async fn every_setting_keeps_its_own_latest_value() {
        let (h, mut rx) = handler_with_zone_capacity(1000);
        let t0 = Instant::now();
        // Use up the budget of zone 1, then change two settings twice.
        for _ in 0..30 {
            h.handle_at("snapdog/zones/1/volume/set", b"5", false, t0)
                .await
                .unwrap();
        }
        volumes(&mut rx);
        for (topic, payload) in [
            ("snapdog/zones/1/volume/set", "10"),
            ("snapdog/zones/1/mute/set", "on"),
            ("snapdog/zones/1/volume/set", "20"),
            ("snapdog/zones/1/mute/set", "off"),
        ] {
            assert_eq!(
                h.handle_at(topic, payload.as_bytes(), false, t0).await,
                Ok(Handled::Deferred)
            );
        }
        h.flush_backlog(t0 + Duration::from_secs(1)).await;
        let mut got = Vec::new();
        while let Ok(cmd) = rx.try_recv() {
            got.push(format!("{cmd:?}"));
        }
        assert_eq!(got, ["SetVolume(20)", "SetMute(false)"]);
    }

    #[tokio::test]
    async fn a_newer_value_is_never_overtaken_by_an_older_one_that_waited() {
        let (h, mut rx) = handler_with_zone_capacity(1000);
        let t0 = Instant::now();
        for _ in 0..30 {
            h.handle_at("snapdog/zones/1/volume/set", b"5", false, t0)
                .await
                .unwrap();
        }
        volumes(&mut rx);
        assert_eq!(
            h.handle_at("snapdog/zones/1/volume/set", b"10", false, t0)
                .await,
            Ok(Handled::Deferred)
        );
        // Budget is back: 20 passes at once. The waiting 10 must not follow it.
        let later = t0 + Duration::from_secs(1);
        assert_eq!(
            h.handle_at("snapdog/zones/1/volume/set", b"20", false, later)
                .await,
            Ok(Handled::Dispatched)
        );
        h.flush_backlog(later + Duration::from_secs(1)).await;
        assert_eq!(volumes(&mut rx), [20]);
        assert!(!h.has_backlog());
    }

    #[tokio::test]
    async fn a_full_zone_channel_defers_the_command_instead_of_blocking_or_losing_it() {
        let (h, mut rx) = handler_with_zone_capacity(1);
        send(&h, "snapdog/zones/1/volume/set", "10").await.unwrap();
        // The channel holds one command and nobody receives: this must return, not hang.
        let second = tokio::time::timeout(
            Duration::from_secs(1),
            send(&h, "snapdog/zones/1/volume/set", "20"),
        )
        .await
        .expect("handle must not block on a full channel");
        assert_eq!(second, Ok(Handled::Deferred));
        assert_eq!(volumes(&mut rx), [10]);
        // Room again: the latest value follows.
        h.flush_backlog(Instant::now()).await;
        assert_eq!(volumes(&mut rx), [20]);
    }

    #[tokio::test]
    async fn a_value_that_still_finds_no_room_stays_in_the_backlog() {
        let (h, mut rx) = handler_with_zone_capacity(1);
        send(&h, "snapdog/zones/1/volume/set", "10").await.unwrap();
        send(&h, "snapdog/zones/1/volume/set", "20").await.unwrap();
        h.flush_backlog(Instant::now()).await;
        assert!(h.has_backlog(), "the channel is still full");
        assert_eq!(volumes(&mut rx), [10]);
        h.flush_backlog(Instant::now()).await;
        assert_eq!(volumes(&mut rx), [20]);
        assert!(!h.has_backlog());
    }

    #[tokio::test]
    async fn a_value_that_waited_too_long_is_dropped_as_stale() {
        let (h, mut rx) = handler_with_zone_capacity(1);
        let t0 = Instant::now();
        send(&h, "snapdog/zones/1/volume/set", "10").await.unwrap();
        send(&h, "snapdog/zones/1/volume/set", "20").await.unwrap();
        volumes(&mut rx);
        h.flush_backlog(t0 + BACKLOG_TTL + Duration::from_secs(1))
            .await;
        assert!(volumes(&mut rx).is_empty(), "a stale volume is not applied");
        assert!(!h.has_backlog());
    }

    /// Review finding m1: an older value could be delivered after a newer one, when a
    /// flush that had taken the old value out of the backlog raced a command that
    /// delivered the new one, or when a value went back into the backlog after a full
    /// channel. 4 senders hammer one slot, the channel is tiny, the flusher runs
    /// meanwhile: the arrival numbers of what was delivered must only go up.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn an_older_value_is_never_delivered_after_a_newer_one_under_concurrency() {
        let (h, mut rx) = handler_with_zone_capacity(2);
        h.gate().limiter = RateLimiter::unlimited(Instant::now());
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));

        // The receiving end drains on a thread of its own, so that the channel is
        // full and has room again at unpredictable moments.
        let consumer = {
            let stop = stop.clone();
            std::thread::spawn(move || {
                while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                    while rx.try_recv().is_ok() {}
                    std::thread::yield_now();
                }
            })
        };
        let flusher = {
            let (h, stop) = (h.clone(), stop.clone());
            tokio::spawn(async move {
                while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                    h.flush_backlog(Instant::now()).await;
                    tokio::task::yield_now().await;
                }
            })
        };
        let senders: Vec<_> = (0..4)
            .map(|worker| {
                let h = h.clone();
                tokio::spawn(async move {
                    for i in 0..150_000 {
                        let payload = format!("{}", 2 + (worker * 7 + i) % 90);
                        h.handle("snapdog/zones/1/volume/set", payload.as_bytes(), false)
                            .await
                            .unwrap();
                        if i % 4 == 0 {
                            tokio::task::yield_now().await;
                        }
                    }
                })
            })
            .collect();
        for sender in senders {
            sender.await.unwrap();
        }
        stop.store(true, std::sync::atomic::Ordering::Relaxed);
        consumer.join().unwrap();
        flusher.await.unwrap();

        let (delivered, violations) = {
            let gate = h.gate();
            (
                gate.delivered.len(),
                gate.delivered
                    .windows(2)
                    .filter(|pair| pair[0].1 >= pair[1].1)
                    .count(),
            )
        };
        assert!(
            delivered > 1000,
            "too little was delivered to prove anything: {delivered}"
        );
        assert_eq!(
            violations, 0,
            "{violations} older values after newer ones in {delivered} deliveries"
        );
    }

    #[tokio::test]
    async fn transport_commands_are_events_and_are_dropped_not_coalesced() {
        // Two `next` are not one: past the budget they are dropped, as before.
        let (h, _rx) = handler_with_zone_capacity(1000);
        let t0 = Instant::now();
        let mut limited = 0;
        for _ in 0..100 {
            match h
                .handle_at("snapdog/zones/1/control/set", b"next", false, t0)
                .await
            {
                Ok(Handled::Dispatched) => {}
                Ok(Handled::RateLimited) => limited += 1,
                other => panic!("{other:?}"),
            }
        }
        assert_eq!(limited, 70);
        assert!(!h.has_backlog(), "nothing is kept for later");
        // A full channel refuses a transport command rather than deferring it.
        let (h, _rx) = handler_with_zone_capacity(1);
        send(&h, "snapdog/zones/1/control/set", "play")
            .await
            .unwrap();
        assert_eq!(
            send(&h, "snapdog/zones/1/control/set", "next").await,
            Err(Rejected::NotDelivered)
        );
    }

    #[tokio::test]
    async fn flooding_one_zone_does_not_hold_back_other_targets() {
        let (h, _rx, _snap_rx, _state) = handler("snapdog/");
        let t0 = Instant::now();
        for _ in 0..100 {
            let _ = h
                .handle_at("snapdog/zones/1/volume/set", b"50", false, t0)
                .await;
        }
        assert_eq!(
            h.handle_at("snapdog/clients/1/mute/set", b"on", false, t0)
                .await,
            Ok(Handled::Dispatched)
        );
    }

    #[tokio::test]
    async fn a_deferred_client_value_is_delivered_with_the_client_id() {
        let (h, _rx, mut snap_rx, _state) = handler("snapdog/");
        let t0 = Instant::now();
        for _ in 0..30 {
            h.handle_at("snapdog/clients/1/volume/set", b"10", false, t0)
                .await
                .unwrap();
        }
        while snap_rx.try_recv().is_ok() {}
        assert_eq!(
            h.handle_at("snapdog/clients/1/volume/set", b"77", false, t0)
                .await,
            Ok(Handled::Deferred)
        );
        h.flush_backlog(t0 + Duration::from_secs(1)).await;
        assert!(matches!(
            snap_rx.try_recv(),
            Ok(SnapcastCmd::Client { client_id, action: ClientAction::Volume(77) }) if client_id == "snap-1"
        ));
    }

    #[test]
    fn rejection_messages_never_contain_a_payload() {
        for rejected in [
            Rejected::PayloadTooLarge,
            Rejected::PayloadNotUtf8,
            Rejected::Payload(PayloadError("volume is not a number")),
            Rejected::NoSuchTarget,
            Rejected::NotDelivered,
            Rejected::Topic(TopicError::InvalidIndex),
        ] {
            assert_ne!(rejected.to_string(), "");
            assert_ne!(rejected.category(), "");
        }
    }

    // ── Regression: command topics under the real, normalised base topic ──
    //
    // `config` appends the trailing slash ("snapdog" becomes "snapdog/"). The bridge
    // once re-derived the base by hand when parsing, so every inbound command was
    // dropped in production while the tests (which used "snapdog" without slash)
    // passed. These tests go through the config loader and take the topics from the
    // function that builds the subscriptions.

    /// The `[mqtt]` section as the config loader resolves it.
    fn mqtt_config(base_topic: Option<&str>) -> crate::config::MqttConfig {
        let base_line = base_topic.map_or(String::new(), |b| format!("base_topic = \"{b}\""));
        let toml = format!(
            r#"
            [mqtt]
            broker = "localhost:1883"
            {base_line}

            [[zone]]
            name = "Z1"
            [[zone]]
            name = "Z2"
            [[client]]
            name = "C1"
            mac = "00:00:00:00:00:01"
            zone = "Z1"
            "#
        );
        crate::config::load_raw(toml::from_str(&toml).unwrap())
            .unwrap()
            .mqtt
            .unwrap()
    }

    /// A payload every command accepts.
    fn valid_payload(tail: &str) -> &'static str {
        match tail {
            "volume/set" => "50",
            "mute/set" | "shuffle/set" | "presence/set" | "presence/enable/set" => "true",
            "control/set" => "play",
            "repeat/set" => "all",
            "position/set" | "track/position/set" => "30",
            "track/set" | "zone/set" => "2",
            "playlist/set" => "1",
            "presence/timeout/set" => "60",
            "latency/set" => "20",
            other => panic!("no sample payload for {other}: extend valid_payload"),
        }
    }

    #[tokio::test]
    async fn command_on_default_normalised_base_is_not_dropped() {
        // The exact topic from the bug report, with the base the config really produces.
        let mqtt = mqtt_config(None);
        assert_eq!(mqtt.base_topic, "snapdog/");
        let (h, mut rx, _snap, _state) = handler(&mqtt.base_topic);
        send(&h, "snapdog/zones/1/volume/set", "75").await.unwrap();
        assert!(matches!(rx.recv().await, Some(ZoneCommand::SetVolume(75))));
    }

    #[tokio::test]
    async fn every_subscribed_topic_is_handled_under_the_real_config_base() {
        for (configured, effective) in [
            (None, "snapdog/"),
            (Some("snapdog"), "snapdog/"),
            (Some("snapdog/"), "snapdog/"),
            (Some("home/audio"), "home/audio/"),
            (Some("home/audio/"), "home/audio/"),
        ] {
            let mqtt = mqtt_config(configured);
            assert_eq!(mqtt.base_topic, effective, "config normalisation");
            // The same function the bridge subscribes with.
            let filters: Vec<String> = Topics::new(&mqtt.base_topic).command_filters().collect();
            assert_eq!(
                filters.len(),
                16,
                "documented command set, base {effective}"
            );

            for filter in filters {
                assert!(filter.starts_with(effective), "{filter}");
                // A concrete topic that this very subscription delivers.
                let topic = filter.replace('+', "1");
                let tail = topic
                    .strip_prefix(effective)
                    .and_then(|t| t.split_once("/1/"))
                    .map(|(_, tail)| tail)
                    .expect("scope/1/tail");

                let (h, mut zone_rx, mut snap_rx, _state) = handler(&mqtt.base_topic);
                send(&h, &topic, valid_payload(tail))
                    .await
                    .unwrap_or_else(|e| panic!("{topic} was rejected: {e}"));
                assert!(
                    zone_rx.try_recv().is_ok() || snap_rx.try_recv().is_ok(),
                    "{topic} (subscribed) produced no command"
                );
            }
        }
    }
}
