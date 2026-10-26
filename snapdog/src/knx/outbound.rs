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
//! - Both queues are bounded. When one is full, the oldest frame is dropped.
//! - When the server keeps its command channel full for [`STALL_LIMIT`] and takes fewer than
//!   [`STALL_MIN_SENT`] frames in that time, it is considered stuck and the caller restarts it,
//!   which also drops its tunnel clients. (A tunnel client that does not acknowledge costs the
//!   server three seconds per frame: slow, not stopped, and never dropped by the server itself.)

use std::collections::VecDeque;
use std::time::Duration;

use knx_rs_core::cemi::CemiFrame;
use knx_rs_core::message::ApduType;
use tokio::time::Instant;

/// Shortest time between two group telegrams handed to the server (40 per second).
pub const PACE: Duration = Duration::from_millis(25);

/// How long the server may keep its command channel full while it takes almost nothing before it
/// counts as stuck.
pub const STALL_LIMIT: Duration = Duration::from_secs(10);

/// Frames the server must take in [`STALL_LIMIT`] of a full channel to count as slow, not stuck.
pub const STALL_MIN_SENT: u32 = 10;

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

/// What distinguishes two group telegrams that may replace each other.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct GroupKey {
    destination: u16,
    service: Option<ApduType>,
}

fn group_key(frame: &CemiFrame) -> Option<GroupKey> {
    use knx_rs_core::types::AddressType;
    if frame.address_type() != AddressType::Group {
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
    /// Since when the server's channel has been full, while it took fewer than the minimum.
    pressure_since: Option<Instant>,
    /// Frames the server took since `pressure_since`.
    pressure_sent: u32,
    /// Hand-overs in a row without finding the channel full.
    consecutive_sent: u32,
    /// Frames dropped because a queue was full, since the last report.
    dropped: u64,
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
        Self {
            management: VecDeque::new(),
            groups: VecDeque::new(),
            next_group_at: Instant::now(),
            blocked_until: None,
            pressure_since: None,
            pressure_sent: 0,
            consecutive_sent: 0,
            dropped: 0,
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

    /// Queue `frame`. A group telegram replaces a queued one for the same address and service.
    pub fn push(&mut self, frame: CemiFrame) {
        let Some(key) = group_key(&frame) else {
            if self.management.len() >= MAX_MANAGEMENT_FRAMES {
                self.management.pop_front();
                self.dropped += 1;
            }
            self.management.push_back(frame);
            return;
        };
        if let Some(queued) = self.groups.iter_mut().find(|f| group_key(f) == Some(key)) {
            *queued = frame;
            return;
        }
        if self.groups.len() >= MAX_GROUP_FRAMES {
            self.groups.pop_front();
            self.dropped += 1;
        }
        self.groups.push_back(frame);
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
        false
    }

    fn sent(&mut self, now: Instant) {
        self.blocked_until = None;
        self.consecutive_sent += 1;
        if self.pressure_since.is_some() {
            self.pressure_sent += 1;
            if self.consecutive_sent >= RECOVERED_AFTER {
                self.pressure_since = None;
                self.pressure_sent = 0;
            } else if self
                .pressure_since
                .is_some_and(|since| now.duration_since(since) >= STALL_LIMIT)
                && self.pressure_sent >= STALL_MIN_SENT
            {
                // Busy, not stuck: start measuring again.
                self.pressure_since = Some(now);
                self.pressure_sent = 0;
            }
        }
    }

    fn block(&mut self, now: Instant) {
        self.consecutive_sent = 0;
        self.pressure_since.get_or_insert(now);
        self.blocked_until = Some(now + PACE);
    }

    /// When [`flush`](Self::flush) has something to do next, `None` when nothing waits.
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
        wake
    }

    /// Whether the server kept its channel full for [`STALL_LIMIT`] and took almost nothing.
    #[must_use]
    pub fn stalled(&self, now: Instant) -> bool {
        self.pressure_since.is_some_and(|since| {
            now.duration_since(since) >= STALL_LIMIT && self.pressure_sent < STALL_MIN_SENT
        })
    }
}

/// Lets a message through at most once per interval.
#[derive(Debug, Default)]
pub struct Throttle {
    last: Option<Instant>,
}

impl Throttle {
    /// `true` when the last allowed call is at least `every` ago (or there was none).
    pub fn allow(&mut self, every: Duration) -> bool {
        let now = Instant::now();
        if self.last.is_none_or(|t| now.duration_since(t) >= every) {
            self.last = Some(now);
            return true;
        }
        false
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
        for dst in 0..(MAX_GROUP_FRAMES as u16 + 10) {
            out.push(write(dst, 1));
        }
        assert_eq!(out.len(), MAX_GROUP_FRAMES);
        assert_eq!(out.take_dropped(), 10);
        let server = Server::with_room(1);
        out.flush(&server, Instant::now());
        assert_eq!(server.destinations(), vec![10], "the ten oldest are gone");
        for _ in 0..(MAX_MANAGEMENT_FRAMES + 3) {
            out.push(management());
        }
        assert_eq!(out.take_dropped(), 3);
    }

    #[tokio::test(start_paused = true)]
    async fn a_full_server_is_never_waited_for_and_counts_as_stuck_after_the_limit() {
        let mut out = Outbound::new();
        let server = Server::with_room(0);
        out.push(write(1, 1));
        out.push(management());
        let t0 = Instant::now();
        // The call returns at once and says the server is not closed.
        assert!(!out.flush(&server, t0));
        assert!(!out.stalled(t0));
        assert_eq!(out.len(), 2, "nothing is lost while the server is full");
        assert!(out.next_wake().is_some(), "it retries on its own schedule");
        tokio::time::advance(STALL_LIMIT.saturating_sub(Duration::from_millis(100))).await;
        out.flush(&server, Instant::now());
        assert!(!out.stalled(Instant::now()));
        tokio::time::advance(Duration::from_millis(200)).await;
        out.flush(&server, Instant::now());
        assert!(
            out.stalled(Instant::now()),
            "full for the whole limit, nothing taken"
        );
    }

    /// A server that takes one frame per three seconds (a tunnel client that does not
    /// acknowledge) is stuck for our purposes, although it is not silent.
    #[tokio::test(start_paused = true)]
    async fn a_server_that_takes_one_frame_in_three_seconds_counts_as_stuck() {
        let mut out = Outbound::new();
        let server = Server::with_room(2);
        for dst in 0..100 {
            out.push(write(dst, 1));
        }
        let start = Instant::now();
        let mut stalled_at = None;
        while start.elapsed() < Duration::from_secs(60) {
            let now = Instant::now();
            out.flush(&server, now);
            if out.stalled(now) {
                stalled_at = Some(start.elapsed());
                break;
            }
            // The server frees a place every three seconds.
            if start.elapsed().as_millis() % 3000 < 25 && server.room.get() == 0 {
                server.room.set(1);
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        let at = stalled_at.expect("a server that takes a frame per 3 s is stuck");
        assert!(
            at >= STALL_LIMIT && at < STALL_LIMIT + Duration::from_secs(4),
            "{at:?}"
        );
    }

    /// A server that takes a frame every 25 ms but sometimes reports full (a burst, a busy
    /// gateway) is not stuck.
    #[tokio::test(start_paused = true)]
    async fn a_busy_server_is_not_stuck() {
        let mut out = Outbound::new();
        let server = Server::with_room(0);
        for dst in 0..2000u16 {
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
            assert!(!out.stalled(now), "stuck at {:?}", start.elapsed());
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        assert!(server.taken.borrow().len() > 100);
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
        assert!(t.allow(Duration::from_secs(5)));
        assert!(!t.allow(Duration::from_secs(5)));
        tokio::time::advance(Duration::from_secs(5)).await;
        assert!(t.allow(Duration::from_secs(5)));
    }
}
