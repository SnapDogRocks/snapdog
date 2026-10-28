// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! Client-mode KNX transport — connects to an external KNX/IP gateway.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use knx_rs_core::address::GroupAddress;
use knx_rs_core::cemi::CemiFrame;
use knx_rs_core::dpt::{Dpt, DptValue};
use knx_rs_core::message::{ApduType, MessageCode};
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

    async fn respond(&self, ga: GroupAddress, dpt: Dpt, value: &DptValue) -> bool {
        match self.handle.group_respond_value(ga, dpt, value).await {
            Ok(()) => true,
            Err(e) => {
                tracing::warn!(ga = %ga, error = %e, "KNX read response failed");
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

/// What a telegram from the bus asks of us.
#[derive(Debug, PartialEq, Eq)]
enum BusRequest {
    /// A group write.
    Write(GroupAddress, Vec<u8>),
    /// A group read.
    Read(GroupAddress),
}

/// The request carried by `frame`, if it is a group write or read the bus sent to us.
///
/// Only `L_Data.ind` is a telegram from the bus. The gateway also returns `L_Data.con` for
/// every telegram this client sent. Treating those confirmations as commands would make
/// every status write that shares its group address with a command object (a trigger such
/// as `play` and a status such as `control_status`, say) execute that command again.
/// A group value response answers somebody else's read and is not a command either.
fn bus_request(frame: &CemiFrame) -> Option<BusRequest> {
    if frame.message_code_raw() != MessageCode::LDataInd as u8 {
        return None;
    }
    if let Some(ga) = frame.as_group_read() {
        return Some(BusRequest::Read(ga));
    }
    let apdu_type = frame.tpdu()?.apdu()?.apdu_type;
    if apdu_type != ApduType::GroupValueWrite {
        return None;
    }
    let (ga, data) = frame.as_group_write()?;
    Some(BusRequest::Write(ga, data))
}

impl KnxListener for ClientListener {
    async fn recv(&mut self) -> Option<Incoming> {
        loop {
            let cemi = self.handle.recv().await?;
            match bus_request(&cemi) {
                Some(BusRequest::Write(ga, data)) => return Some(Incoming::Address(ga, data)),
                Some(BusRequest::Read(ga)) => return Some(Incoming::Read(ga)),
                None => {}
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

    fn group_telegram(code: MessageCode, apci: [u8; 2]) -> CemiFrame {
        CemiFrame::new_l_data(
            code,
            IndividualAddress::from_raw(0x1101),
            DestinationAddress::Group(GroupAddress::from_str("1/0/1").unwrap()),
            Priority::Low,
            &apci,
        )
    }

    fn group_write(code: MessageCode) -> CemiFrame {
        // TPCI/APCI for a 6-bit group value write of 1: 0x00 0x81.
        group_telegram(code, [0x00, 0x81])
    }

    #[test]
    fn bus_telegrams_are_commands() {
        let Some(BusRequest::Write(ga, data)) = bus_request(&group_write(MessageCode::LDataInd))
        else {
            panic!("a bus write is a request");
        };
        assert_eq!(ga.to_string(), "1/0/1");
        assert_eq!(data, vec![0x01]);
    }

    #[test]
    fn own_confirmations_and_requests_are_not_commands() {
        assert!(bus_request(&group_write(MessageCode::LDataCon)).is_none());
        assert!(bus_request(&group_write(MessageCode::LDataReq)).is_none());
    }

    #[test]
    fn reads_are_requests_and_responses_are_not_commands() {
        let read = group_telegram(MessageCode::LDataInd, [0x00, 0x00]);
        assert!(
            matches!(bus_request(&read), Some(BusRequest::Read(ga)) if ga.to_string() == "1/0/1")
        );
        // Somebody else's answer to a read must not execute a command.
        let response = group_telegram(MessageCode::LDataInd, [0x00, 0x41]);
        assert_eq!(bus_request(&response), None);
        // Our own read, confirmed by the gateway, is not a request either.
        assert_eq!(
            bus_request(&group_telegram(MessageCode::LDataCon, [0x00, 0x00])),
            None
        );
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
