// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! Client-mode KNX transport — connects to an external KNX/IP gateway.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use knx_rs_core::dpt::{Dpt, DptValue};
use knx_rs_ip::KnxIpError;
use knx_rs_ip::multiplex::MultiplexHandle;
use knx_rs_ip::ops::GroupOps;
use tokio::sync::Notify;

use super::transport::{Incoming, KnxListener, KnxPublisher, Target};

/// Unacknowledged writes in a row after which the publisher declares the connection dead.
///
/// The tunnel notices a vanished gateway only through its heartbeat (60 s interval, three
/// misses), and the status values are sent on change only, so there may be no write for a
/// long time. A write whose acknowledgement does not arrive after the transport's own three
/// retries is a strong signal, two in a row are taken as proof.
const MAX_UNACKNOWLEDGED_WRITES: u32 = 2;

/// What a failed write says about the connection.
#[derive(Debug, PartialEq, Eq)]
enum Verdict {
    /// Nothing: the value could not be encoded.
    Unrelated,
    /// No acknowledgement. Could be a busy gateway.
    Suspect,
    /// The connection is gone: the socket failed (the gateway's port is closed, the network is
    /// unreachable), the gateway rejected our channel or the connection was closed.
    Dead,
}

const fn judge(error: &KnxIpError) -> Verdict {
    match error {
        KnxIpError::Timeout(_) => Verdict::Suspect,
        KnxIpError::Io(_)
        | KnxIpError::Protocol(_)
        | KnxIpError::ConnectionRejected(_)
        | KnxIpError::Closed => Verdict::Dead,
        _ => Verdict::Unrelated,
    }
}

/// Client-mode publisher: sends group writes via a `MultiplexHandle`.
pub struct ClientPublisher {
    handle: MultiplexHandle,
    failures: AtomicU32,
    broken: Arc<Notify>,
}

impl ClientPublisher {
    /// Returns the publisher and a signal that fires once the connection looks dead.
    pub(crate) fn new(handle: MultiplexHandle) -> (Self, Arc<Notify>) {
        let broken = Arc::new(Notify::new());
        (
            Self {
                handle,
                failures: AtomicU32::new(0),
                broken: Arc::clone(&broken),
            },
            broken,
        )
    }
}

impl KnxPublisher for ClientPublisher {
    const ADDRESSED: bool = true;

    async fn write(&self, target: Target, dpt: Dpt, value: &DptValue) -> bool {
        let Target::Address(ga) = target else {
            return false;
        };
        match self.handle.group_write_value(ga, dpt, value).await {
            Ok(()) => {
                self.failures.store(0, Ordering::Relaxed);
                true
            }
            Err(e) => {
                tracing::warn!(ga = %ga, error = %e, "KNX write failed");
                let dead = match judge(&e) {
                    Verdict::Dead => true,
                    Verdict::Suspect => {
                        self.failures.fetch_add(1, Ordering::Relaxed) + 1
                            >= MAX_UNACKNOWLEDGED_WRITES
                    }
                    Verdict::Unrelated => false,
                };
                if dead {
                    self.broken.notify_one();
                }
                false
            }
        }
    }
}

/// Client-mode listener: receives group writes via a `MultiplexHandle`.
pub struct ClientListener {
    handle: MultiplexHandle,
}

impl ClientListener {
    pub(crate) const fn new(handle: MultiplexHandle) -> Self {
        Self { handle }
    }
}

impl KnxListener for ClientListener {
    async fn recv(&mut self) -> Option<Incoming> {
        loop {
            let cemi = self.handle.recv().await?;
            if let Some((ga, data)) = cemi.as_group_write() {
                return Some(Incoming::Address(ga, data));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_errors_are_judged_by_what_they_say_about_the_link() {
        let io = || KnxIpError::Io(std::io::ErrorKind::ConnectionRefused.into());
        assert_eq!(judge(&io()), Verdict::Dead);
        assert_eq!(judge(&KnxIpError::Closed), Verdict::Dead);
        assert_eq!(
            judge(&KnxIpError::Protocol("tunneling ack error: 0x21".into())),
            Verdict::Dead
        );
        assert_eq!(
            judge(&KnxIpError::Timeout("tunneling ack after max retries")),
            Verdict::Suspect
        );
        assert_eq!(
            judge(&KnxIpError::Dpt(knx_rs_core::dpt::DptError::NoDpt)),
            Verdict::Unrelated
        );
    }
}
