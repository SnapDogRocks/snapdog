// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! The way out of the device task: frames for the KNXnet/IP server, queued, paced and bounded.
//!
//! The device task must never wait for the server. The server is a single task that sends every
//! frame to every tunnel client and waits up to three seconds for each acknowledgement, and it
//! blocks on its own event channel when the device task does not take events. If the device task
//! in turn waits for room in the server's command channel, the two wait for each other and the
//! device is dead until the process restarts. A tunnel client that stops acknowledging (a hung
//! visualisation, a half-open NAT mapping) is enough to start that.
//!
//! [`Outbound`] takes every frame the device stack produces and hands them over without ever
//! waiting:
//!
//! - **Management frames** (answers to ETS) keep their order and are handed over as soon as the
//!   server has room.
//! - **Group telegrams** are paced, one per [`PACE`] (about the rate of a TP line), and
//!   coalesced: a telegram for a group address and service replaces a still queued one, so the
//!   latest value per address wins and a burst of changes costs one telegram per address.
//! - Frames for destination 0 (broadcasts such as the answer to an individual address read) and
//!   all point-to-point frames count as management: ordered, not paced.
//! - A newer write to a group address retires the queued responses for it (they carry a value
//!   the write has made stale), and a response queues behind a write for the same address, so
//!   the order on the wire is the order of events.
//! - Both queues are bounded. When one is full, the oldest frame is dropped.
//! - When the server cannot keep up (its command channel stays full and the queue does not
//!   drain) for [`STALL_LIMIT`], [`Outbound::poll_stall`] says so, rate limited, and the counters in
//!   [`OutboundStats`] tell how many frames were dropped or coalesced and for how long the
//!   server was behind. Nothing restarts the server: dropping a `DeviceServer` does not stop its
//!   task in knx-rs (it keeps sending what is queued, to clients that never acknowledge, and holds
//!   the port for minutes), so a restart would only make things worse. The bounded queue is what
//!   protects the device task. (A tunnel client that does not acknowledge costs the server three
//!   seconds per frame: slow, not stopped, and never dropped by the server itself.)

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use knx_rs_core::cemi::CemiFrame;
use knx_rs_core::message::ApduType;
use tokio::time::Instant;

/// Shortest time between two group telegrams handed to the server (40 per second).
pub const PACE: Duration = Duration::from_millis(25);

/// How long the server may stay behind (its command channel full, the queue not draining)
/// before it counts as stalled.
pub const STALL_LIMIT: Duration = Duration::from_secs(10);

/// A server that stays stalled is reported again after this long.
pub const STALL_REPORT_EVERY: Duration = Duration::from_secs(60);

/// Consecutive hand-overs after which the server counts as keeping up again.
const RECOVERED_AFTER: u32 = 4;

/// Most group telegrams that wait; more drop the oldest.
pub const MAX_GROUP_FRAMES: usize = 512;

/// Most management frames that wait; more drop the oldest.
pub const MAX_MANAGEMENT_FRAMES: usize = 256;

/// What handing a frame to the server came to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendResult {
    /// The server took it.
    Sent,
    /// The server's command channel is full.
    Full,
    /// The server has ended.
    Closed,
}

/// Where frames go: a non-blocking hand-over.
pub trait FrameSink {
    /// Hand `frame` over if there is room right now.
    fn try_send(&self, frame: &CemiFrame) -> SendResult;
}

impl FrameSink for knx_rs_ip::tunnel_server::DeviceServer {
    fn try_send(&self, frame: &CemiFrame) -> SendResult {
        use futures_util::FutureExt as _;
        // `send` on a bounded channel is cancel safe: a future that is not ready has sent nothing.
        match self.send_frame(frame.clone()).now_or_never() {
            Some(Ok(())) => SendResult::Sent,
            Some(Err(_)) => SendResult::Closed,
            None => SendResult::Full,
        }
    }
}

/// What the outbound queue counted, readable from other tasks.
#[derive(Debug, Default)]
pub struct OutboundStats {
    /// Frames dropped because a queue was full.
    pub dropped: AtomicU64,
    /// Queued group telegrams replaced or retired by a newer one.
    pub coalesced: AtomicU64,
    /// Seconds the server spent stalled, completed stalls and the current one.
    pub stalled_seconds: AtomicU64,
    /// Number of stalls detected.
    pub stalls: AtomicU64,
    /// Whether the server is stalled now.
    pub stalled: AtomicBool,
}

/// A change in how well the server keeps up, for the caller to log.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StallEvent {
    /// The server has been behind for `seconds` and `waiting` frames wait.
    Stalled {
        seconds: u64,
        waiting: usize,
        dropped: u64,
        coalesced: u64,
    },
    /// The queue drained again after a stall of `seconds`.
    Recovered { seconds: u64 },
}

/// What distinguishes two group telegrams that may replace each other.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct GroupKey {
    destination: u16,
    service: Option<ApduType>,
}

fn group_key(frame: &CemiFrame) -> Option<GroupKey> {
    use knx_rs_core::types::AddressType;
    // Destination 0 is the broadcast address (the answer to an individual address read, say):
    // it is no group telegram and must not wait behind a burst of them.
    if frame.address_type() != AddressType::Group || frame.destination_address_raw() == 0 {
        return None;
    }
    Some(GroupKey {
        destination: frame.destination_address_raw(),
        service: frame.tpdu().and_then(|t| t.apdu().map(|a| a.apdu_type)),
    })
}

/// Queued, paced, bounded frames for the server.
#[derive(Debug)]
pub struct Outbound {
    management: VecDeque<CemiFrame>,
    groups: VecDeque<CemiFrame>,
    /// The next group telegram may go at this time.
    next_group_at: Instant,
    /// The server was full until this time (retry then).
    blocked_until: Option<Instant>,
    /// Since when the server has been behind: its channel was full and the queue has not
    /// drained since. Cleared when the queue is empty and after a run of hand-overs.
    behind_since: Option<Instant>,
    /// Frames the server took since `behind_since`.
    behind_sent: u64,
    /// When the stall was last reported (or when it ended, for the recovery to be reported).
    last_report: Option<Instant>,
    /// A stall was reported and the queue has drained since: the recovery waits to be reported.
    recovered: Option<u64>,
    /// Hand-overs in a row without finding the channel full.
    consecutive_sent: u32,
    /// Frames dropped because a queue was full, since the last call of `take_dropped`.
    dropped: u64,
    /// Time of the completed stalls.
    stalled_total: Duration,
    stats: Arc<OutboundStats>,
}

impl Default for Outbound {
    fn default() -> Self {
        Self::new()
    }
}

impl Outbound {
    /// Empty queues.
    #[must_use]
    pub fn new() -> Self {
        Self::with_stats(Arc::new(OutboundStats::default()))
    }

    /// Empty queues that count into `stats`.
    #[must_use]
    pub fn with_stats(stats: Arc<OutboundStats>) -> Self {
        Self {
            management: VecDeque::new(),
            groups: VecDeque::new(),
            next_group_at: Instant::now(),
            blocked_until: None,
            behind_since: None,
            behind_sent: 0,
            last_report: None,
            recovered: None,
            consecutive_sent: 0,
            dropped: 0,
            stalled_total: Duration::ZERO,
            stats,
        }
    }

    /// Whether nothing waits.
    #[cfg(test)]
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.management.is_empty() && self.groups.is_empty()
    }

    /// Number of frames that wait.
    #[allow(clippy::len_without_is_empty)] // `is_empty` is only needed by the tests
    #[must_use]
    pub fn len(&self) -> usize {
        self.management.len() + self.groups.len()
    }

    /// Frames dropped on a full queue since the last call.
    pub fn take_dropped(&mut self) -> u64 {
        std::mem::take(&mut self.dropped)
    }

    /// Queue `frame`. A group telegram replaces a queued one for the same address and service,
    /// a write retires the queued responses for its address, and a response queues behind a
    /// write for the same address.
    pub fn push(&mut self, frame: CemiFrame) {
        let Some(key) = group_key(&frame) else {
            if self.management.len() >= MAX_MANAGEMENT_FRAMES {
                self.management.pop_front();
                self.count_drop();
            }
            self.management.push_back(frame);
            return;
        };
        let is_response = |f: &CemiFrame| {
            group_key(f).is_some_and(|k| {
                k.destination == key.destination && k.service == Some(ApduType::GroupValueResponse)
            })
        };
        let has_write = self.groups.iter().any(|f| {
            group_key(f).is_some_and(|k| {
                k.destination == key.destination && k.service == Some(ApduType::GroupValueWrite)
            })
        });
        match key.service {
            Some(ApduType::GroupValueWrite) => {
                // Responses queued for this address carry the value this write replaces.
                let before = self.groups.len();
                self.groups.retain(|f| !is_response(f));
                self.count_coalesced((before - self.groups.len()) as u64);
            }
            Some(ApduType::GroupValueResponse) if has_write => {
                // Behind the write: the reader must see the response after the value.
                let before = self.groups.len();
                self.groups.retain(|f| !is_response(f));
                self.count_coalesced((before - self.groups.len()) as u64);
            }
            _ => {}
        }
        let in_place = !(key.service == Some(ApduType::GroupValueResponse) && has_write);
        if in_place && let Some(queued) = self.groups.iter_mut().find(|f| group_key(f) == Some(key))
        {
            *queued = frame;
            self.count_coalesced(1);
            return;
        }
        if self.groups.len() >= MAX_GROUP_FRAMES {
            self.groups.pop_front();
            self.count_drop();
        }
        self.groups.push_back(frame);
    }

    fn count_drop(&mut self) {
        self.dropped += 1;
        self.stats.dropped.fetch_add(1, Ordering::Relaxed);
    }

    fn count_coalesced(&self, n: u64) {
        if n > 0 {
            self.stats.coalesced.fetch_add(n, Ordering::Relaxed);
        }
    }

    /// Hand over what may go now. Returns `true` when the server has ended.
    pub fn flush(&mut self, sink: &impl FrameSink, now: Instant) -> bool {
        if self.blocked_until.is_some_and(|until| now < until) {
            return false;
        }
        while let Some(frame) = self.management.front() {
            match sink.try_send(frame) {
                SendResult::Sent => {
                    self.management.pop_front();
                    self.sent(now);
                }
                SendResult::Full => {
                    self.block(now);
                    return false;
                }
                SendResult::Closed => return true,
            }
        }
        if now >= self.next_group_at
            && let Some(frame) = self.groups.front()
        {
            match sink.try_send(frame) {
                SendResult::Sent => {
                    self.groups.pop_front();
                    self.next_group_at = now + PACE;
                    self.sent(now);
                }
                SendResult::Full => self.block(now),
                SendResult::Closed => return true,
            }
        }
        if self.management.is_empty() && self.groups.is_empty() {
            self.end_behind(now);
        }
        false
    }

    fn sent(&mut self, now: Instant) {
        self.blocked_until = None;
        self.consecutive_sent += 1;
        if self.behind_since.is_some() {
            self.behind_sent += 1;
        }
        if self.consecutive_sent >= RECOVERED_AFTER {
            self.end_behind(now);
        }
    }

    fn block(&mut self, now: Instant) {
        self.consecutive_sent = 0;
        if self.behind_since.is_none() {
            self.behind_since = Some(now);
            self.behind_sent = 0;
        }
        self.blocked_until = Some(now + PACE);
    }

    /// The queue is empty, or the server took a run of frames: whatever held it back is over.
    fn end_behind(&mut self, now: Instant) {
        self.behind_sent = 0;
        if let Some(since) = self.behind_since.take()
            && self.last_report.is_some()
        {
            let lasted = now.duration_since(since);
            self.stalled_total += lasted;
            self.stats
                .stalled_seconds
                .store(self.stalled_total.as_secs(), Ordering::Relaxed);
            self.stats.stalled.store(false, Ordering::Relaxed);
            self.recovered = Some(lasted.as_secs());
        }
        self.last_report = None;
    }

    /// When [`flush`](Self::flush) or [`poll_stall`](Self::poll_stall) has something to do next,
    /// `None` when nothing waits.
    #[must_use]
    pub fn next_wake(&self) -> Option<Instant> {
        let retry = self.blocked_until;
        let mut wake = if self.management.is_empty() {
            None
        } else {
            retry
        };
        if !self.groups.is_empty() {
            let at = retry.map_or(self.next_group_at, |r| r.max(self.next_group_at));
            wake = Some(wake.map_or(at, |w| w.min(at)));
        }
        if let Some(since) = self.behind_since {
            let at = self
                .last_report
                .map_or(since + STALL_LIMIT, |r| r + STALL_REPORT_EVERY);
            wake = Some(wake.map_or(at, |w| w.min(at)));
        }
        wake
    }

    /// Whether the server has been behind for [`STALL_LIMIT`] and has not been reported for
    /// [`STALL_REPORT_EVERY`] (or at all), or has caught up after a reported stall.
    pub fn poll_stall(&mut self, now: Instant) -> Option<StallEvent> {
        if let Some(seconds) = self.recovered.take() {
            return Some(StallEvent::Recovered { seconds });
        }
        let since = self.behind_since?;
        let lasted = now.duration_since(since);
        if lasted < STALL_LIMIT {
            return None;
        }
        // A server that takes at least a frame a second is slow or busy, not stalled: start
        // looking again from now.
        if self.behind_sent >= lasted.as_secs() {
            self.end_behind(now);
            self.behind_since = Some(now);
            return None;
        }
        if self
            .last_report
            .is_some_and(|r| now.duration_since(r) < STALL_REPORT_EVERY)
        {
            return None;
        }
        if self.last_report.is_none() {
            self.stats.stalls.fetch_add(1, Ordering::Relaxed);
        }
        self.last_report = Some(now);
        self.stats.stalled.store(true, Ordering::Relaxed);
        self.stats
            .stalled_seconds
            .store((self.stalled_total + lasted).as_secs(), Ordering::Relaxed);
        Some(StallEvent::Stalled {
            seconds: lasted.as_secs(),
            waiting: self.len(),
            dropped: self.stats.dropped.load(Ordering::Relaxed),
            coalesced: self.stats.coalesced.load(Ordering::Relaxed),
        })
    }
}

/// Lets a message through at most once per interval and counts what it held back.
#[derive(Debug, Default)]
pub struct Throttle {
    last: Option<Instant>,
    pending: u64,
}

impl Throttle {
    /// Record `n` events. Returns the number of events since the last message when one may go
    /// out now (the first event, then at most one message per `every`), `None` otherwise.
    pub fn add(&mut self, n: u64, every: Duration) -> Option<u64> {
        self.pending += n;
        let now = Instant::now();
        if self.pending > 0 && self.last.is_none_or(|t| now.duration_since(t) >= every) {
            self.last = Some(now);
            return Some(std::mem::take(&mut self.pending));
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};

    use knx_rs_core::address::{DestinationAddress, GroupAddress, IndividualAddress};
    use knx_rs_core::message::MessageCode;
    use knx_rs_core::types::Priority;

    use super::*;

    fn group(dst: u16, apci: [u8; 2]) -> CemiFrame {
        CemiFrame::new_l_data(
            MessageCode::LDataInd,
            IndividualAddress::from_raw(0x1164),
            DestinationAddress::Group(GroupAddress::from_raw(dst)),
            Priority::Low,
            &apci,
        )
    }

    fn write(dst: u16, value: u8) -> CemiFrame {
        group(dst, [0x00, 0x80 | value])
    }

    fn management() -> CemiFrame {
        CemiFrame::new_l_data(
            MessageCode::LDataInd,
            IndividualAddress::from_raw(0x1164),
            DestinationAddress::Individual(IndividualAddress::from_raw(0xFFB1)),
            Priority::System,
            &[0x03, 0xD6],
        )
    }

    /// A server with `room` places that records what it took.
    struct Server {
        room: Cell<usize>,
        closed: Cell<bool>,
        taken: RefCell<Vec<CemiFrame>>,
    }

    impl Server {
        fn with_room(room: usize) -> Self {
            Self {
                room: Cell::new(room),
                closed: Cell::new(false),
                taken: RefCell::new(Vec::new()),
            }
        }

        fn destinations(&self) -> Vec<u16> {
            self.taken
                .borrow()
                .iter()
                .map(CemiFrame::destination_address_raw)
                .collect()
        }
    }

    impl FrameSink for Server {
        fn try_send(&self, frame: &CemiFrame) -> SendResult {
            if self.closed.get() {
                return SendResult::Closed;
            }
            if self.room.get() == 0 {
                return SendResult::Full;
            }
            self.room.set(self.room.get() - 1);
            self.taken.borrow_mut().push(frame.clone());
            SendResult::Sent
        }
    }

    #[tokio::test(start_paused = true)]
    async fn group_telegrams_are_paced() {
        let mut out = Outbound::new();
        let server = Server::with_room(100);
        for dst in 1..=5 {
            out.push(write(dst, 1));
        }
        let start = Instant::now();
        let mut sent_at = Vec::new();
        while !out.is_empty() {
            out.flush(&server, Instant::now());
            if server.taken.borrow().len() > sent_at.len() {
                sent_at.push(Instant::now() - start);
            }
            if let Some(wake) = out.next_wake() {
                tokio::time::sleep_until(wake).await;
            }
        }
        assert_eq!(server.destinations(), vec![1, 2, 3, 4, 5], "in order");
        for pair in sent_at.windows(2) {
            assert!(pair[1].checked_sub(pair[0]).unwrap() >= PACE, "{sent_at:?}");
        }
    }

    #[tokio::test(start_paused = true)]
    async fn the_latest_value_per_address_wins() {
        let mut out = Outbound::new();
        for v in 0..50u8 {
            out.push(write(7, v % 2));
            out.push(write(8, 1));
        }
        out.push(write(7, 1));
        assert_eq!(out.len(), 2, "one telegram per address");
        let server = Server::with_room(10);
        out.flush(&server, Instant::now());
        let first = server.taken.borrow()[0].as_group_write().unwrap();
        assert_eq!(first.0.raw(), 7);
        assert_eq!(first.1, vec![1], "the last value written to 7");
    }

    #[tokio::test(start_paused = true)]
    async fn a_response_does_not_replace_a_write() {
        let mut out = Outbound::new();
        out.push(write(7, 1));
        out.push(group(7, [0x00, 0x41])); // group value response
        assert_eq!(out.len(), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn management_frames_keep_their_order_and_are_not_paced() {
        let mut out = Outbound::new();
        let server = Server::with_room(100);
        for i in 0..5u8 {
            out.push(CemiFrame::new_l_data(
                MessageCode::LDataInd,
                IndividualAddress::from_raw(0x1164),
                DestinationAddress::Individual(IndividualAddress::from_raw(0xFFB1)),
                Priority::System,
                &[0x03, 0xD6 + i],
            ));
        }
        out.flush(&server, Instant::now());
        let order: Vec<u8> = server
            .taken
            .borrow()
            .iter()
            .map(|f| f.as_bytes()[f.as_bytes().len() - 1])
            .collect();
        assert_eq!(order, vec![0xD6, 0xD7, 0xD8, 0xD9, 0xDA]);
    }

    #[tokio::test(start_paused = true)]
    async fn queues_are_bounded_and_drop_the_oldest() {
        let mut out = Outbound::new();
        for dst in 1..=(MAX_GROUP_FRAMES as u16 + 10) {
            out.push(write(dst, 1));
        }
        assert_eq!(out.len(), MAX_GROUP_FRAMES);
        assert_eq!(out.take_dropped(), 10);
        let server = Server::with_room(1);
        out.flush(&server, Instant::now());
        assert_eq!(server.destinations(), vec![11], "the ten oldest are gone");
        for _ in 0..(MAX_MANAGEMENT_FRAMES + 3) {
            out.push(management());
        }
        assert_eq!(out.take_dropped(), 3);
    }

    #[tokio::test(start_paused = true)]
    async fn a_full_server_is_never_waited_for_and_reported_as_stalled_after_the_limit() {
        let mut out = Outbound::new();
        let server = Server::with_room(0);
        out.push(write(1, 1));
        out.push(management());
        let t0 = Instant::now();
        // The call returns at once and says the server is not closed.
        assert!(!out.flush(&server, t0));
        assert_eq!(out.poll_stall(t0), None);
        assert_eq!(out.len(), 2, "nothing is lost while the server is full");
        assert!(out.next_wake().is_some(), "it retries on its own schedule");
        tokio::time::advance(STALL_LIMIT.saturating_sub(Duration::from_millis(100))).await;
        out.flush(&server, Instant::now());
        assert_eq!(out.poll_stall(Instant::now()), None, "not yet");
        tokio::time::advance(Duration::from_millis(200)).await;
        out.flush(&server, Instant::now());
        let event = out.poll_stall(Instant::now());
        assert!(
            matches!(
                event,
                Some(StallEvent::Stalled {
                    seconds: 10,
                    waiting: 2,
                    ..
                })
            ),
            "{event:?}"
        );
        assert!(out.stats.stalled.load(Ordering::Relaxed));
        assert_eq!(out.stats.stalls.load(Ordering::Relaxed), 1);
        assert_eq!(out.stats.stalled_seconds.load(Ordering::Relaxed), 10);
    }

    /// The report comes once, then again every `STALL_REPORT_EVERY` while the stall lasts, so a
    /// server that is stuck for a day does not fill the log; `next_wake` knows when to look.
    #[tokio::test(start_paused = true)]
    async fn a_stall_is_reported_at_a_limited_rate_and_the_recovery_once() {
        let mut out = Outbound::new();
        let server = Server::with_room(0);
        out.push(write(1, 1));
        out.flush(&server, Instant::now());
        let started = Instant::now();
        assert_eq!(
            out.next_wake().map(|w| w.duration_since(started)),
            Some(PACE)
        );
        let mut reports = Vec::new();
        while started.elapsed() < Duration::from_secs(200) {
            let now = Instant::now();
            out.flush(&server, now);
            if let Some(StallEvent::Stalled { seconds, .. }) = out.poll_stall(now) {
                reports.push(seconds);
            }
            tokio::time::sleep_until(out.next_wake().unwrap()).await;
        }
        // At 10 s, 70 s, 130 s, 190 s.
        assert_eq!(reports, vec![10, 70, 130, 190]);
        // The server takes the frame: the queue is empty and the recovery is reported once.
        server.room.set(1);
        tokio::time::advance(PACE).await;
        out.flush(&server, Instant::now());
        assert!(matches!(
            out.poll_stall(Instant::now()),
            Some(StallEvent::Recovered { seconds: 200..=201 })
        ));
        assert_eq!(out.poll_stall(Instant::now()), None);
        assert!(!out.stats.stalled.load(Ordering::Relaxed));
        assert!(out.stats.stalled_seconds.load(Ordering::Relaxed) >= 200);
    }

    /// A server that takes one frame per three seconds (a tunnel client that does not
    /// acknowledge) is behind although it is not silent: the report says so, and it is not
    /// restarted by anyone.
    #[tokio::test(start_paused = true)]
    async fn a_server_that_takes_one_frame_in_three_seconds_is_reported_as_behind() {
        let mut out = Outbound::new();
        let server = Server::with_room(2);
        for dst in 1..=100 {
            out.push(write(dst, 1));
        }
        let start = Instant::now();
        let mut reported_at = None;
        while start.elapsed() < Duration::from_secs(60) {
            let now = Instant::now();
            out.flush(&server, now);
            if matches!(out.poll_stall(now), Some(StallEvent::Stalled { .. })) {
                reported_at = Some(start.elapsed());
                break;
            }
            // The server frees a place every three seconds.
            if start.elapsed().as_millis() % 3000 < 25 && server.room.get() == 0 {
                server.room.set(1);
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        let at = reported_at.expect("a server that takes a frame per 3 s is behind");
        assert!(
            at >= STALL_LIMIT && at < STALL_LIMIT + Duration::from_secs(4),
            "{at:?}"
        );
    }

    /// A server that takes a frame every 25 ms but sometimes reports full (a burst, a busy
    /// gateway) is not behind.
    #[tokio::test(start_paused = true)]
    async fn a_busy_server_is_not_reported() {
        let mut out = Outbound::new();
        let server = Server::with_room(0);
        for dst in 1..=2000u16 {
            out.push(write(dst, 1));
        }
        let start = Instant::now();
        let mut i = 0u32;
        while start.elapsed() < Duration::from_secs(40) && !out.is_empty() {
            // Full for 100 ms, then room for three frames, over and over.
            if i.is_multiple_of(4) {
                server.room.set(3);
            }
            i += 1;
            let now = Instant::now();
            out.flush(&server, now);
            assert_eq!(
                out.poll_stall(now),
                None,
                "reported at {:?}",
                start.elapsed()
            );
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        assert!(server.taken.borrow().len() > 100);
    }

    /// A short stall that heals leaves nothing behind: two minutes later an ordinary change of
    /// three telegrams is not mistaken for a server that is stuck (the old state survived the
    /// recovery and the first look after the change found it "stalled").
    #[tokio::test(start_paused = true)]
    async fn an_old_stall_that_healed_does_not_make_a_healthy_server_look_stuck() {
        let mut out = Outbound::new();
        let server = Server::with_room(0);
        out.push(write(1, 1));
        out.push(write(2, 1));
        out.flush(&server, Instant::now()); // full: behind
        tokio::time::advance(Duration::from_secs(5)).await;
        server.room.set(10); // the client acknowledges again
        for _ in 0..4 {
            tokio::time::advance(PACE).await;
            out.flush(&server, Instant::now());
        }
        assert!(out.is_empty());
        tokio::time::advance(Duration::from_secs(120)).await;
        for dst in 3..6 {
            out.push(write(dst, 1));
        }
        out.flush(&server, Instant::now());
        tokio::time::advance(PACE).await;
        assert_eq!(out.poll_stall(Instant::now()), None);
        assert_eq!(
            out.next_wake().map(|w| w.duration_since(Instant::now())),
            Some(Duration::ZERO)
        );
    }

    /// The counters count what the queue dropped and coalesced.
    #[tokio::test(start_paused = true)]
    async fn drops_and_coalesced_frames_are_counted() {
        let mut out = Outbound::new();
        for v in 0..5u8 {
            out.push(write(7, v % 2));
        }
        assert_eq!(out.stats.coalesced.load(Ordering::Relaxed), 4);
        for dst in 100..(100 + MAX_GROUP_FRAMES as u16 + 2) {
            out.push(write(dst, 1));
        }
        assert_eq!(out.stats.dropped.load(Ordering::Relaxed), 3);
    }

    /// write(A,1), response(A,1), write(A,0): the reader must end up with 0. (Replacing the
    /// write in place put it in front of the response, which then told the reader 1.)
    #[tokio::test(start_paused = true)]
    async fn a_newer_write_retires_the_queued_responses_for_its_address() {
        let mut out = Outbound::new();
        out.push(write(9, 1));
        out.push(group(9, [0x00, 0x41])); // response, value 1
        out.push(write(9, 0));
        let server = Server::with_room(10);
        out.flush(&server, Instant::now());
        tokio::time::advance(PACE).await;
        out.flush(&server, Instant::now());
        let taken = server.taken.borrow();
        assert_eq!(taken.len(), 1, "the stale response is gone");
        assert_eq!(taken[0].as_group_write().unwrap().1, vec![0]);
    }

    #[tokio::test(start_paused = true)]
    async fn a_response_queues_behind_the_write_for_its_address() {
        let mut out = Outbound::new();
        out.push(write(9, 1));
        out.push(write(10, 1));
        out.push(group(9, [0x00, 0x40])); // response, value 0
        out.push(group(9, [0x00, 0x41])); // a newer response replaces it, at the end
        let server = Server::with_room(10);
        for _ in 0..5 {
            out.flush(&server, Instant::now());
            tokio::time::advance(PACE).await;
        }
        let services: Vec<(u16, bool)> = server
            .taken
            .borrow()
            .iter()
            .map(|f| {
                let service = f.tpdu().and_then(|t| t.apdu().map(|a| a.apdu_type));
                (
                    f.destination_address_raw(),
                    service == Some(ApduType::GroupValueWrite),
                )
            })
            .collect();
        assert_eq!(services, vec![(9, true), (10, true), (9, false)]);
    }

    /// The answer to an individual address read goes to the broadcast address: it is not
    /// delayed by a burst of group telegrams.
    #[tokio::test(start_paused = true)]
    async fn a_broadcast_is_handled_like_a_management_frame() {
        let mut out = Outbound::new();
        let server = Server::with_room(100);
        for dst in 1..=30 {
            out.push(write(dst, 1));
        }
        // A_IndividualAddress_Response: to group address 0.
        out.push(group(0, [0x01, 0x40]));
        out.flush(&server, Instant::now());
        let destinations = server.destinations();
        assert!(
            destinations.contains(&0),
            "sent at once, not behind 30 paced telegrams: {destinations:?}"
        );
        assert_eq!(destinations[0], 0, "ahead of the burst");
    }

    #[tokio::test(start_paused = true)]
    async fn a_closed_server_is_reported() {
        let mut out = Outbound::new();
        let server = Server::with_room(5);
        server.closed.set(true);
        out.push(write(1, 1));
        assert!(out.flush(&server, Instant::now()));
        let mut out = Outbound::new();
        out.push(management());
        assert!(out.flush(&server, Instant::now()));
    }

    #[tokio::test(start_paused = true)]
    async fn nothing_to_do_means_no_wake_up() {
        let out = Outbound::new();
        assert_eq!(out.next_wake(), None);
        assert!(out.is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn the_throttle_lets_one_message_through_per_interval() {
        let mut t = Throttle::default();
        let every = Duration::from_secs(5);
        assert_eq!(t.add(1, every), Some(1));
        assert_eq!(t.add(1, every), None);
        assert_eq!(t.add(3, every), None);
        tokio::time::advance(every).await;
        assert_eq!(t.add(1, every), Some(5), "what was held back is counted");
        assert_eq!(t.add(0, every), None, "nothing to say");
    }
}
