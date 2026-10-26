// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! Status output: decides which status values are worth a telegram and where they go.
//!
//! The publisher offers a status value every time the zone or client state changes, and a
//! zone state changes about once a second during radio or Subsonic playback (the playback
//! position is part of it). Sending all status addresses on each of those changes would put
//! tens of telegrams per second on a bus that carries a few dozen. The sink remembers what it
//! sent last for every group object and passes only changes on; a transport reconnect or a
//! reprogrammed device calls [`StatusSink::invalidate`] so that everything is sent again.

use std::collections::{HashMap, HashSet};
use std::str::FromStr;
use std::time::Duration;

use knx_rs_core::address::GroupAddress;
use knx_rs_core::dpt::{self, Dpt, DptValue};
use tokio::time::Instant;

use super::group_objects::{
    CLIENT_GO_COUNT, GLOBAL_GO_COUNT, MAX_CLIENTS, MAX_ZONES, ZONE_GO_COUNT, client_asap,
    global_asap, zone_asap,
};
use super::transport::{KnxPublisher, Target};

/// Identity of a group object: the zone or client it belongs to and its index in
/// `ZONE_GOS` / `CLIENT_GOS`, or its index in `GLOBAL_GOS`.
///
/// The ASAP number is the device stack's name for the same thing, and it only exists for the
/// 10 zones and 30 clients the product database has. Client mode has no such limit, so state
/// that is kept per object (what was sent last) must not be keyed by ASAP: zone 11 would share
/// its key with client 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ObjectKey {
    /// Object `.1` of zone `.0` (both 1-based for the zone, 0-based for the object).
    Zone(usize, usize),
    /// Object `.1` of client `.0`.
    Client(usize, usize),
    /// Device-level object.
    Global(usize),
}

impl ObjectKey {
    /// The device stack's ASAP number, `None` for an object the product database does not have
    /// (a zone above 10, a client above 30).
    #[must_use]
    pub fn asap(self) -> Option<u16> {
        match self {
            Self::Zone(z, i) if (1..=MAX_ZONES).contains(&z) && i < ZONE_GO_COUNT => {
                Some(zone_asap(z, i))
            }
            Self::Client(c, i) if (1..=MAX_CLIENTS).contains(&c) && i < CLIENT_GO_COUNT => {
                Some(client_asap(c, i))
            }
            Self::Global(i) if i < GLOBAL_GO_COUNT => Some(global_asap(i)),
            _ => None,
        }
    }
}

/// Shortest time between two progress telegrams of one zone, unless the position jumps back
/// (a new track). The value itself only changes in whole percent.
pub const PROGRESS_MIN_INTERVAL: Duration = Duration::from_secs(2);

/// How a value is offered to the sink.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// Send when the value differs from the one sent last.
    OnChange,
    /// Send every time (the cyclic heartbeat).
    Always,
    /// Send on change, but not more often than [`PROGRESS_MIN_INTERVAL`].
    Progress,
}

/// What was last handed to the transport for a group object.
struct Sent {
    /// The value as it went on the wire.
    encoded: Vec<u8>,
    /// Client mode: the group address it went to.
    address: Option<GroupAddress>,
    /// For answering a read of that address.
    dpt: Dpt,
    value: DptValue,
}

/// A progress value that arrived inside the rate limit and waits for its turn.
struct Deferred {
    address: Option<String>,
    dpt: Dpt,
    value: DptValue,
}

/// Change-filtering front end of a [`KnxPublisher`].
pub struct StatusSink<'a, P> {
    transport: &'a P,
    /// What was last handed to the transport, per group object.
    sent: HashMap<ObjectKey, Sent>,
    /// When a progress value was last handed to the transport, per group object.
    progress_at: HashMap<ObjectKey, Instant>,
    /// Progress values held back by the rate limit. Without this, the last position before
    /// playback pauses would never reach the bus.
    deferred: HashMap<ObjectKey, Deferred>,
    /// Addresses somebody asked for and that had no value yet; the next offer for them goes out
    /// as a read response.
    reads: HashSet<GroupAddress>,
}

impl<'a, P: KnxPublisher> StatusSink<'a, P> {
    /// A sink over `transport` that has sent nothing yet.
    pub fn new(transport: &'a P) -> Self {
        Self {
            transport,
            sent: HashMap::new(),
            progress_at: HashMap::new(),
            deferred: HashMap::new(),
            reads: HashSet::new(),
        }
    }

    /// Forget what was sent: the next offer of every value goes out.
    pub fn invalidate(&mut self) {
        self.sent.clear();
        self.progress_at.clear();
        self.deferred.clear();
    }

    /// Answer a group read of `ga` with the value last sent to it. When nothing was sent to that
    /// address yet (the first write failed, or the object has no value yet) the address is
    /// remembered and `false` is returned: the caller offers the current values again, and the
    /// offer for `ga` goes out as the response. Call [`end_reads`](Self::end_reads) afterwards.
    pub async fn answer_read(&mut self, ga: GroupAddress) -> bool {
        let known = self
            .sent
            .values()
            .find(|s| s.address == Some(ga))
            .map(|s| (s.dpt, s.value.clone()));
        if let Some((dpt, value)) = known {
            return self.transport.respond(ga, dpt, &value).await;
        }
        self.reads.insert(ga);
        false
    }

    /// Forget the reads that the offers since [`answer_read`](Self::answer_read) did not answer
    /// (an address that has no status object).
    pub fn end_reads(&mut self) {
        self.reads.clear();
    }

    /// When the oldest deferred progress value may go out, if there is one.
    #[must_use]
    pub fn next_due(&self) -> Option<Instant> {
        self.deferred
            .keys()
            .filter_map(|k| self.progress_at.get(k))
            .map(|at| *at + PROGRESS_MIN_INTERVAL)
            .min()
    }

    /// Send the deferred progress values whose time has come.
    pub async fn flush_due(&mut self) {
        let now = Instant::now();
        let due: Vec<ObjectKey> = self
            .deferred
            .keys()
            .copied()
            .filter(|k| {
                self.progress_at
                    .get(k)
                    .is_none_or(|at| *at + PROGRESS_MIN_INTERVAL <= now)
            })
            .collect();
        for key in due {
            if let Some(d) = self.deferred.remove(&key) {
                self.offer(key, d.address.as_deref(), d.dpt, &d.value, Mode::Progress)
                    .await;
            }
        }
    }

    /// Offer a status value; it is sent when it differs from the last one sent.
    ///
    /// `key` identifies the group object. `address` is the configured group address, which
    /// only client mode uses (device mode lets the programmed associations choose).
    pub async fn write(
        &mut self,
        key: ObjectKey,
        address: Option<&str>,
        dpt: Dpt,
        value: &DptValue,
    ) {
        self.offer(key, address, dpt, value, Mode::OnChange).await;
    }

    /// Offer a value that is sent every time, changed or not.
    pub async fn write_always(
        &mut self,
        key: ObjectKey,
        address: Option<&str>,
        dpt: Dpt,
        value: &DptValue,
    ) {
        self.offer(key, address, dpt, value, Mode::Always).await;
    }

    /// Offer a playback progress value: on change, rate limited.
    pub async fn write_progress(
        &mut self,
        key: ObjectKey,
        address: Option<&str>,
        dpt: Dpt,
        value: &DptValue,
    ) {
        self.offer(key, address, dpt, value, Mode::Progress).await;
    }

    async fn offer(
        &mut self,
        key: ObjectKey,
        address: Option<&str>,
        dpt: Dpt,
        value: &DptValue,
        mode: Mode,
    ) {
        let target = if P::ADDRESSED {
            // Client mode sends only what the configuration gives an address.
            let Some(address) = address else {
                return;
            };
            match GroupAddress::from_str(address) {
                Ok(ga) => Target::Address(ga),
                Err(e) => {
                    tracing::warn!(ga = address, error = %e, "Invalid KNX GA");
                    return;
                }
            }
        } else {
            // A zone or client the product database does not have has no object to write.
            let Some(asap) = key.asap() else {
                tracing::debug!(?key, "no device object for this zone or client, not sent");
                return;
            };
            Target::Object(asap)
        };
        let encoded = match dpt::encode(dpt, value) {
            Ok(bytes) => bytes,
            Err(e) => {
                tracing::warn!(?key, error = %e, "KNX value not encodable");
                return;
            }
        };

        let reading = matches!(target, Target::Address(ga) if self.reads.contains(&ga));
        if mode != Mode::Always
            && !reading
            && self.sent.get(&key).is_some_and(|s| s.encoded == encoded)
        {
            self.deferred.remove(&key);
            return;
        }
        if mode == Mode::Progress && !reading && !self.progress_due(key, &encoded) {
            self.deferred.insert(
                key,
                Deferred {
                    address: address.map(str::to_owned),
                    dpt,
                    value: value.clone(),
                },
            );
            return;
        }

        // Count only what the transport took, so a failed write is offered again next time.
        let taken = match target {
            Target::Address(ga) if reading => {
                let ok = self.transport.respond(ga, dpt, value).await;
                if ok {
                    self.reads.remove(&ga);
                }
                ok
            }
            _ => self.transport.write(target, dpt, value).await,
        };
        if taken {
            self.deferred.remove(&key);
            self.sent.insert(
                key,
                Sent {
                    encoded,
                    address: match target {
                        Target::Address(ga) => Some(ga),
                        Target::Object(_) => None,
                    },
                    dpt,
                    value: value.clone(),
                },
            );
            if mode == Mode::Progress {
                self.progress_at.insert(key, Instant::now());
            }
        }
    }

    /// Whether a progress telegram may go out now: it has been long enough, or the value
    /// went backwards (the next track started).
    fn progress_due(&self, key: ObjectKey, encoded: &[u8]) -> bool {
        let Some(at) = self.progress_at.get(&key) else {
            return true;
        };
        let went_back = self
            .sent
            .get(&key)
            .is_some_and(|last| encoded.first() < last.encoded.first());
        went_back || at.elapsed() >= PROGRESS_MIN_INTERVAL
    }
}

#[cfg(test)]
pub mod testing {
    //! A recording transport for the publisher tests.

    use std::sync::Mutex;

    use knx_rs_core::dpt::{Dpt, DptValue};

    use super::super::transport::{KnxPublisher, Target};

    /// Records every write; reports success unless told otherwise.
    pub struct Recorder<const ADDRESSED: bool> {
        /// `(target, dpt, value)` of every write, in order.
        pub writes: Mutex<Vec<(Target, Dpt, DptValue)>>,
        /// Whether writes succeed.
        pub accept: std::sync::atomic::AtomicBool,
        /// `(address, value)` of every read response, in order.
        pub responses: Mutex<Vec<(knx_rs_core::address::GroupAddress, DptValue)>>,
    }

    impl<const A: bool> Default for Recorder<A> {
        fn default() -> Self {
            Self {
                writes: Mutex::new(Vec::new()),
                accept: std::sync::atomic::AtomicBool::new(true),
                responses: Mutex::new(Vec::new()),
            }
        }
    }

    impl<const A: bool> Recorder<A> {
        /// Writes recorded so far.
        pub fn count(&self) -> usize {
            self.writes.lock().unwrap().len()
        }
    }

    impl<const A: bool> KnxPublisher for Recorder<A> {
        const ADDRESSED: bool = A;

        fn write(
            &self,
            target: Target,
            dpt: Dpt,
            value: &DptValue,
        ) -> impl std::future::Future<Output = bool> + Send {
            let ok = self.accept.load(std::sync::atomic::Ordering::Relaxed);
            if ok {
                self.writes
                    .lock()
                    .unwrap()
                    .push((target, dpt, value.clone()));
            }
            std::future::ready(ok)
        }

        fn respond(
            &self,
            ga: knx_rs_core::address::GroupAddress,
            _dpt: Dpt,
            value: &DptValue,
        ) -> impl std::future::Future<Output = bool> + Send {
            let ok = self.accept.load(std::sync::atomic::Ordering::Relaxed);
            if ok {
                self.responses.lock().unwrap().push((ga, value.clone()));
            }
            std::future::ready(ok)
        }
    }
}

#[cfg(test)]
mod tests {
    use knx_rs_core::dpt::{DPT_SCALING, DPT_SWITCH};

    use super::testing::Recorder;
    use super::*;

    /// Object `n` of zone 1.
    const fn k(n: usize) -> ObjectKey {
        ObjectKey::Zone(1, n)
    }

    #[tokio::test]
    async fn unchanged_values_are_not_sent_again() {
        let rec = Recorder::<true>::default();
        let mut sink = StatusSink::new(&rec);
        for _ in 0..5 {
            sink.write(k(1), Some("1/1/1"), DPT_SWITCH, &true.into())
                .await;
        }
        assert_eq!(rec.count(), 1);
        sink.write(k(1), Some("1/1/1"), DPT_SWITCH, &false.into())
            .await;
        assert_eq!(rec.count(), 2);
    }

    #[tokio::test]
    async fn invalidate_sends_everything_again() {
        let rec = Recorder::<true>::default();
        let mut sink = StatusSink::new(&rec);
        sink.write(k(1), Some("1/1/1"), DPT_SWITCH, &true.into())
            .await;
        sink.invalidate();
        sink.write(k(1), Some("1/1/1"), DPT_SWITCH, &true.into())
            .await;
        assert_eq!(rec.count(), 2);
    }

    #[tokio::test]
    async fn heartbeat_is_sent_every_time() {
        let rec = Recorder::<true>::default();
        let mut sink = StatusSink::new(&rec);
        for _ in 0..3 {
            sink.write_always(k(1), Some("1/1/1"), DPT_SWITCH, &true.into())
                .await;
        }
        assert_eq!(rec.count(), 3);
    }

    #[tokio::test]
    async fn client_mode_needs_a_configured_address() {
        let rec = Recorder::<true>::default();
        let mut sink = StatusSink::new(&rec);
        sink.write(k(1), None, DPT_SWITCH, &true.into()).await;
        sink.write(k(2), Some("not-an-address"), DPT_SWITCH, &true.into())
            .await;
        assert_eq!(rec.count(), 0);
        sink.write(k(3), Some("1/2047"), DPT_SWITCH, &true.into())
            .await;
        let writes = rec.writes.lock().unwrap();
        assert_eq!(
            writes[0].0,
            Target::Address(GroupAddress::from_str("1/7/255").unwrap())
        );
    }

    #[tokio::test]
    async fn device_mode_addresses_the_object_and_ignores_configured_addresses() {
        let rec = Recorder::<false>::default();
        let mut sink = StatusSink::new(&rec);
        sink.write(k(3), None, DPT_SWITCH, &true.into()).await;
        sink.write(k(4), Some("garbage"), DPT_SWITCH, &true.into())
            .await;
        let writes = rec.writes.lock().unwrap();
        assert_eq!(writes[0].0, Target::Object(zone_asap(1, 3)));
        assert_eq!(writes[1].0, Target::Object(zone_asap(1, 4)));
    }

    #[tokio::test]
    async fn rejected_writes_are_offered_again() {
        let rec = Recorder::<true>::default();
        rec.accept
            .store(false, std::sync::atomic::Ordering::Relaxed);
        let mut sink = StatusSink::new(&rec);
        sink.write(k(1), Some("1/1/1"), DPT_SWITCH, &true.into())
            .await;
        rec.accept.store(true, std::sync::atomic::Ordering::Relaxed);
        sink.write(k(1), Some("1/1/1"), DPT_SWITCH, &true.into())
            .await;
        assert_eq!(rec.count(), 1);
    }

    #[tokio::test]
    async fn progress_is_rate_limited_but_a_new_track_goes_through() {
        let rec = Recorder::<true>::default();
        let mut sink = StatusSink::new(&rec);
        let pct = |p: f64| DptValue::Float(p);
        sink.write_progress(k(1), Some("1/1/8"), DPT_SCALING, &pct(10.0))
            .await;
        // Within the interval: held back although the value changed.
        sink.write_progress(k(1), Some("1/1/8"), DPT_SCALING, &pct(11.0))
            .await;
        assert_eq!(rec.count(), 1);
        // The next track starts: the value goes back and is sent immediately.
        sink.write_progress(k(1), Some("1/1/8"), DPT_SCALING, &pct(0.0))
            .await;
        assert_eq!(rec.count(), 2);
        // Same value again: nothing.
        sink.write_progress(k(1), Some("1/1/8"), DPT_SCALING, &pct(0.0))
            .await;
        assert_eq!(rec.count(), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn progress_resumes_after_the_interval() {
        let rec = Recorder::<true>::default();
        let mut sink = StatusSink::new(&rec);
        sink.write_progress(k(1), Some("1/1/8"), DPT_SCALING, &DptValue::Float(10.0))
            .await;
        tokio::time::advance(PROGRESS_MIN_INTERVAL).await;
        sink.write_progress(k(1), Some("1/1/8"), DPT_SCALING, &DptValue::Float(11.0))
            .await;
        assert_eq!(rec.count(), 2);
    }

    #[test]
    fn asap_exists_only_for_the_objects_of_the_product_database() {
        assert_eq!(ObjectKey::Zone(1, 0).asap(), Some(1));
        assert_eq!(
            ObjectKey::Zone(MAX_ZONES, ZONE_GO_COUNT - 1).asap(),
            Some(330)
        );
        assert_eq!(ObjectKey::Zone(MAX_ZONES + 1, 0).asap(), None);
        assert_eq!(ObjectKey::Zone(0, 0).asap(), None);
        assert_eq!(ObjectKey::Zone(1, ZONE_GO_COUNT).asap(), None);
        assert_eq!(ObjectKey::Client(1, 0).asap(), Some(331));
        assert_eq!(ObjectKey::Client(MAX_CLIENTS + 1, 0).asap(), None);
        assert_eq!(ObjectKey::Global(0).asap(), Some(661));
        assert_eq!(ObjectKey::Global(GLOBAL_GO_COUNT).asap(), None);
        // The numbers that collide when two objects are told apart by ASAP only.
        assert_eq!(
            zone_asap(MAX_ZONES + 1, 8),
            client_asap(1, 8),
            "zone 11 shares ASAPs with client 1"
        );
    }

    #[tokio::test]
    async fn client_mode_zone_11_does_not_share_memory_with_client_1() {
        // Client mode has no limit on zones. Zone 11's mute status (object 9) and client 1's
        // zone status (object 8) are different objects whose ASAP numbers are both 340.
        let rec = Recorder::<true>::default();
        let mut sink = StatusSink::new(&rec);
        let zone_11_mute = ObjectKey::Zone(11, 9);
        let client_1_zone = ObjectKey::Client(1, 8);
        sink.write(zone_11_mute, Some("3/0/1"), DPT_SWITCH, &true.into())
            .await;
        // Same encoded byte, different object: must still go out.
        sink.write(client_1_zone, Some("3/0/2"), DPT_SWITCH, &true.into())
            .await;
        assert_eq!(rec.count(), 2);
    }

    #[tokio::test]
    async fn device_mode_has_no_object_for_zone_11() {
        // Writing zone 11 to ASAP 340 would overwrite client 1's object.
        let rec = Recorder::<false>::default();
        let mut sink = StatusSink::new(&rec);
        sink.write(ObjectKey::Zone(11, 9), None, DPT_SWITCH, &true.into())
            .await;
        assert_eq!(rec.count(), 0);
    }

    // ── Reads ───────────────────────────────────────────────────────

    #[tokio::test]
    async fn a_read_is_answered_with_the_last_value_sent_to_the_address() {
        let rec = Recorder::<true>::default();
        let mut sink = StatusSink::new(&rec);
        sink.write(k(1), Some("1/1/1"), DPT_SCALING, &DptValue::Float(40.0))
            .await;
        sink.write(k(1), Some("1/1/1"), DPT_SCALING, &DptValue::Float(55.0))
            .await;
        let asked = GroupAddress::from_str("1/1/1").unwrap();
        assert!(sink.answer_read(asked).await);
        let responses = rec.responses.lock().unwrap();
        assert_eq!(responses.len(), 1);
        assert_eq!(responses[0].0, asked);
        assert_eq!(
            responses[0].1,
            DptValue::Float(55.0),
            "the current value, not the first"
        );
    }

    #[tokio::test]
    async fn a_read_of_an_address_without_a_value_is_answered_by_the_next_offer() {
        // The first write failed, so nothing was ever sent to the address.
        let rec = Recorder::<true>::default();
        rec.accept
            .store(false, std::sync::atomic::Ordering::Relaxed);
        let mut sink = StatusSink::new(&rec);
        sink.write(k(1), Some("1/1/1"), DPT_SCALING, &DptValue::Float(40.0))
            .await;
        rec.accept.store(true, std::sync::atomic::Ordering::Relaxed);

        let asked = GroupAddress::from_str("1/1/1").unwrap();
        assert!(!sink.answer_read(asked).await, "nothing to answer with yet");
        // The caller offers the current values again; the one for the address is the response.
        sink.write(k(1), Some("1/1/1"), DPT_SCALING, &DptValue::Float(40.0))
            .await;
        sink.write(k(2), Some("1/1/2"), DPT_SCALING, &DptValue::Float(10.0))
            .await;
        sink.end_reads();
        assert_eq!(
            rec.responses.lock().unwrap().len(),
            1,
            "a response for 1/1/1"
        );
        assert_eq!(rec.count(), 1, "and an ordinary write for 1/1/2");
        // The read is over: later changes are writes again.
        sink.write(k(1), Some("1/1/1"), DPT_SCALING, &DptValue::Float(41.0))
            .await;
        assert_eq!(rec.responses.lock().unwrap().len(), 1);
        assert_eq!(rec.count(), 2);
    }

    #[tokio::test]
    async fn reads_of_addresses_without_status_stay_unanswered() {
        let rec = Recorder::<true>::default();
        let mut sink = StatusSink::new(&rec);
        sink.write(k(1), Some("1/1/1"), DPT_SCALING, &DptValue::Float(40.0))
            .await;
        let asked = GroupAddress::from_str("9/7/9").unwrap();
        assert!(!sink.answer_read(asked).await);
        sink.end_reads();
        assert!(rec.responses.lock().unwrap().is_empty());
    }

    // ── Deferred progress ───────────────────────────────────────────

    #[tokio::test(start_paused = true)]
    async fn the_last_progress_inside_the_window_is_sent_when_the_window_ends() {
        let rec = Recorder::<true>::default();
        let mut sink = StatusSink::new(&rec);
        let pct = |p: f64| DptValue::Float(p);
        sink.write_progress(k(1), Some("1/1/8"), DPT_SCALING, &pct(36.0))
            .await;
        assert_eq!(sink.next_due(), None);
        // 37 % comes inside the rate limit and playback pauses: it must not be lost.
        sink.write_progress(k(1), Some("1/1/8"), DPT_SCALING, &pct(37.0))
            .await;
        assert_eq!(rec.count(), 1);
        let due = sink.next_due().expect("the held back value has a time");
        assert!(due > Instant::now());
        // Not yet.
        sink.flush_due().await;
        assert_eq!(rec.count(), 1);
        tokio::time::advance(PROGRESS_MIN_INTERVAL).await;
        sink.flush_due().await;
        assert_eq!(rec.count(), 2, "37 % goes out");
        assert_eq!(sink.next_due(), None);
        // A value that returns to what was sent cancels the wait.
        sink.write_progress(k(1), Some("1/1/8"), DPT_SCALING, &pct(38.0))
            .await;
        sink.write_progress(k(1), Some("1/1/8"), DPT_SCALING, &pct(37.0))
            .await;
        assert_eq!(sink.next_due(), None);
    }
}
