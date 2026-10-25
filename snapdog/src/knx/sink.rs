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

use std::collections::HashMap;
use std::str::FromStr;
use std::time::Duration;

use knx_rs_core::address::GroupAddress;
use knx_rs_core::dpt::{self, Dpt, DptValue};
use tokio::time::Instant;

use super::transport::{KnxPublisher, Target};

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

/// Change-filtering front end of a [`KnxPublisher`].
pub struct StatusSink<'a, P> {
    transport: &'a P,
    /// Encoded value last handed to the transport, per group object (ASAP).
    sent: HashMap<u16, Vec<u8>>,
    /// When a progress value was last handed to the transport, per group object.
    progress_at: HashMap<u16, Instant>,
}

impl<'a, P: KnxPublisher> StatusSink<'a, P> {
    /// A sink over `transport` that has sent nothing yet.
    pub fn new(transport: &'a P) -> Self {
        Self {
            transport,
            sent: HashMap::new(),
            progress_at: HashMap::new(),
        }
    }

    /// Forget what was sent: the next offer of every value goes out.
    pub fn invalidate(&mut self) {
        self.sent.clear();
        self.progress_at.clear();
    }

    /// Offer a status value; it is sent when it differs from the last one sent.
    ///
    /// `asap` identifies the group object. `address` is the configured group address, which
    /// only client mode uses (device mode lets the programmed associations choose).
    pub async fn write(&mut self, asap: u16, address: Option<&str>, dpt: Dpt, value: &DptValue) {
        self.offer(asap, address, dpt, value, Mode::OnChange).await;
    }

    /// Offer a value that is sent every time, changed or not.
    pub async fn write_always(
        &mut self,
        asap: u16,
        address: Option<&str>,
        dpt: Dpt,
        value: &DptValue,
    ) {
        self.offer(asap, address, dpt, value, Mode::Always).await;
    }

    /// Offer a playback progress value: on change, rate limited.
    pub async fn write_progress(
        &mut self,
        asap: u16,
        address: Option<&str>,
        dpt: Dpt,
        value: &DptValue,
    ) {
        self.offer(asap, address, dpt, value, Mode::Progress).await;
    }

    async fn offer(
        &mut self,
        asap: u16,
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
            Target::Object(asap)
        };
        let encoded = match dpt::encode(dpt, value) {
            Ok(bytes) => bytes,
            Err(e) => {
                tracing::warn!(asap, error = %e, "KNX value not encodable");
                return;
            }
        };

        if mode != Mode::Always && self.sent.get(&asap) == Some(&encoded) {
            return;
        }
        if mode == Mode::Progress && !self.progress_due(asap, &encoded) {
            return;
        }

        // Count only what the transport took, so a failed write is offered again next time.
        if self.transport.write(target, dpt, value).await {
            self.sent.insert(asap, encoded);
            if mode == Mode::Progress {
                self.progress_at.insert(asap, Instant::now());
            }
        }
    }

    /// Whether a progress telegram may go out now: it has been long enough, or the value
    /// went backwards (the next track started).
    fn progress_due(&self, asap: u16, encoded: &[u8]) -> bool {
        let Some(at) = self.progress_at.get(&asap) else {
            return true;
        };
        let went_back = self
            .sent
            .get(&asap)
            .is_some_and(|last| encoded.first() < last.first());
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
    }

    impl<const A: bool> Default for Recorder<A> {
        fn default() -> Self {
            Self {
                writes: Mutex::new(Vec::new()),
                accept: std::sync::atomic::AtomicBool::new(true),
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
    }
}

#[cfg(test)]
mod tests {
    use knx_rs_core::dpt::{DPT_SCALING, DPT_SWITCH};

    use super::testing::Recorder;
    use super::*;

    #[tokio::test]
    async fn unchanged_values_are_not_sent_again() {
        let rec = Recorder::<true>::default();
        let mut sink = StatusSink::new(&rec);
        for _ in 0..5 {
            sink.write(1, Some("1/1/1"), DPT_SWITCH, &true.into()).await;
        }
        assert_eq!(rec.count(), 1);
        sink.write(1, Some("1/1/1"), DPT_SWITCH, &false.into())
            .await;
        assert_eq!(rec.count(), 2);
    }

    #[tokio::test]
    async fn invalidate_sends_everything_again() {
        let rec = Recorder::<true>::default();
        let mut sink = StatusSink::new(&rec);
        sink.write(1, Some("1/1/1"), DPT_SWITCH, &true.into()).await;
        sink.invalidate();
        sink.write(1, Some("1/1/1"), DPT_SWITCH, &true.into()).await;
        assert_eq!(rec.count(), 2);
    }

    #[tokio::test]
    async fn heartbeat_is_sent_every_time() {
        let rec = Recorder::<true>::default();
        let mut sink = StatusSink::new(&rec);
        for _ in 0..3 {
            sink.write_always(1, Some("1/1/1"), DPT_SWITCH, &true.into())
                .await;
        }
        assert_eq!(rec.count(), 3);
    }

    #[tokio::test]
    async fn client_mode_needs_a_configured_address() {
        let rec = Recorder::<true>::default();
        let mut sink = StatusSink::new(&rec);
        sink.write(1, None, DPT_SWITCH, &true.into()).await;
        sink.write(2, Some("not-an-address"), DPT_SWITCH, &true.into())
            .await;
        assert_eq!(rec.count(), 0);
        sink.write(3, Some("1/2047"), DPT_SWITCH, &true.into())
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
        sink.write(42, None, DPT_SWITCH, &true.into()).await;
        sink.write(43, Some("garbage"), DPT_SWITCH, &true.into())
            .await;
        let writes = rec.writes.lock().unwrap();
        assert_eq!(writes[0].0, Target::Object(42));
        assert_eq!(writes[1].0, Target::Object(43));
    }

    #[tokio::test]
    async fn rejected_writes_are_offered_again() {
        let rec = Recorder::<true>::default();
        rec.accept
            .store(false, std::sync::atomic::Ordering::Relaxed);
        let mut sink = StatusSink::new(&rec);
        sink.write(1, Some("1/1/1"), DPT_SWITCH, &true.into()).await;
        rec.accept.store(true, std::sync::atomic::Ordering::Relaxed);
        sink.write(1, Some("1/1/1"), DPT_SWITCH, &true.into()).await;
        assert_eq!(rec.count(), 1);
    }

    #[tokio::test]
    async fn progress_is_rate_limited_but_a_new_track_goes_through() {
        let rec = Recorder::<true>::default();
        let mut sink = StatusSink::new(&rec);
        let pct = |p: f64| DptValue::Float(p);
        sink.write_progress(1, Some("1/1/8"), DPT_SCALING, &pct(10.0))
            .await;
        // Within the interval: held back although the value changed.
        sink.write_progress(1, Some("1/1/8"), DPT_SCALING, &pct(11.0))
            .await;
        assert_eq!(rec.count(), 1);
        // The next track starts: the value goes back and is sent immediately.
        sink.write_progress(1, Some("1/1/8"), DPT_SCALING, &pct(0.0))
            .await;
        assert_eq!(rec.count(), 2);
        // Same value again: nothing.
        sink.write_progress(1, Some("1/1/8"), DPT_SCALING, &pct(0.0))
            .await;
        assert_eq!(rec.count(), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn progress_resumes_after_the_interval() {
        let rec = Recorder::<true>::default();
        let mut sink = StatusSink::new(&rec);
        sink.write_progress(1, Some("1/1/8"), DPT_SCALING, &DptValue::Float(10.0))
            .await;
        tokio::time::advance(PROGRESS_MIN_INTERVAL).await;
        sink.write_progress(1, Some("1/1/8"), DPT_SCALING, &DptValue::Float(11.0))
            .await;
        assert_eq!(rec.count(), 2);
    }
}
