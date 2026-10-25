// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! Client-mode KNX transport — connects to an external KNX/IP gateway.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use knx_rs_core::address::GroupAddress;
use knx_rs_core::cemi::CemiFrame;
use knx_rs_core::dpt::{Dpt, DptValue};
use knx_rs_core::message::MessageCode;
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

/// The group write carried by `frame`, if it is one the bus sent to us.
///
/// Only `L_Data.ind` is a telegram from the bus. The gateway also returns `L_Data.con` for
/// every telegram this client sent. Treating those confirmations as commands would make
/// every status write that shares its group address with a command object (a trigger such
/// as `play` and a status such as `control_status`, say) execute that command again.
fn bus_group_write(frame: &CemiFrame) -> Option<(GroupAddress, Vec<u8>)> {
    if frame.message_code_raw() != MessageCode::LDataInd as u8 {
        return None;
    }
    frame.as_group_write()
}

impl KnxListener for ClientListener {
    async fn recv(&mut self) -> Option<Incoming> {
        loop {
            let cemi = self.handle.recv().await?;
            if let Some((ga, data)) = bus_group_write(&cemi) {
                return Some(Incoming::Address(ga, data));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use knx_rs_core::address::{DestinationAddress, IndividualAddress};
    use knx_rs_core::types::Priority;

    use super::*;

    fn group_write(code: MessageCode) -> CemiFrame {
        // TPCI/APCI for a 6-bit group value write of 1: 0x00 0x81.
        CemiFrame::new_l_data(
            code,
            IndividualAddress::from_raw(0x1101),
            DestinationAddress::Group(GroupAddress::from_str("1/0/1").unwrap()),
            Priority::Low,
            &[0x00, 0x81],
        )
    }

    #[test]
    fn bus_telegrams_are_commands() {
        let (ga, data) = bus_group_write(&group_write(MessageCode::LDataInd)).unwrap();
        assert_eq!(ga.to_string(), "1/0/1");
        assert_eq!(data, vec![0x01]);
    }

    #[test]
    fn own_confirmations_and_requests_are_not_commands() {
        assert!(bus_group_write(&group_write(MessageCode::LDataCon)).is_none());
        assert!(bus_group_write(&group_write(MessageCode::LDataReq)).is_none());
    }

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
