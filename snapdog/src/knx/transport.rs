// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! KNX transport abstractions — shared interfaces for client and device modes.
//!
//! The two modes differ in who decides the group address of a value:
//!
//! - **Client mode** talks to a gateway and knows no address programming. The configured
//!   `[[zone]].knx` / `[[client]].knx` addresses are the only association between a
//!   group object and a group address, so both directions work on [`GroupAddress`]es.
//! - **Device mode** is an ETS-programmable device. ETS (or, for a device ETS never
//!   programmed, the TOML addresses) fills the address and association tables, and the
//!   device stack resolves between group addresses and group objects. Both directions
//!   work on group objects, identified by their ASAP number
//!   ([`group_objects::zone_asap`](super::group_objects::zone_asap) and friends).

use knx_rs_core::address::GroupAddress;
use knx_rs_core::dpt::{Dpt, DptValue};

/// Where a status value goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// Write to this group address (client mode).
    Address(GroupAddress),
    /// Write this group object; the programmed associations pick the address (device mode).
    Object(u16),
}

/// A group telegram or group object update received from the bus.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Incoming {
    /// A group write seen on the bus (client mode). The configuration decides what it means.
    Address(GroupAddress, Vec<u8>),
    /// A group object that was updated from the bus (device mode), by ASAP number.
    Object(u16, Vec<u8>),
    /// A group read request on the bus (client mode). Device mode answers reads inside the
    /// device stack, by the object flags.
    Read(GroupAddress),
}

/// Write-only interface to the KNX bus.
///
/// Used by the publisher task to send group value writes.
/// Immutable (`&self`) — can be shared across tasks via `Arc`.
pub trait KnxPublisher: Send + Sync + 'static {
    /// `true` when values are written to configured group addresses (client mode),
    /// `false` when they are written to group objects (device mode).
    const ADDRESSED: bool;

    /// Write a typed value. Returns whether the transport took it; a rejected value is offered
    /// again with the next state change.
    fn write(
        &self,
        target: Target,
        dpt: Dpt,
        value: &DptValue,
    ) -> impl std::future::Future<Output = bool> + Send;

    /// Answer a group read with a group value response. Returns whether the transport took it.
    /// Device mode answers inside the device stack, so only client mode implements this.
    fn respond(
        &self,
        _ga: GroupAddress,
        _dpt: Dpt,
        _value: &DptValue,
    ) -> impl std::future::Future<Output = bool> + Send {
        std::future::ready(false)
    }

    /// Completes whenever the transport lost the state its peers hold (for example a device
    /// stack that restarted), so the caller re-sends every status value.
    ///
    /// Transports that are rebuilt after such an event never complete this.
    fn resync(&self) -> impl std::future::Future<Output = ()> + Send {
        std::future::pending()
    }
}

/// Read-only interface to the KNX bus.
///
/// Used by the listener task to receive group value writes from the bus.
/// Mutable (`&mut self`) — owned by a single task.
pub trait KnxListener: Send + 'static {
    /// Receive the next group write from the bus.
    /// Returns `None` if the connection is closed.
    fn recv(&mut self) -> impl std::future::Future<Output = Option<Incoming>> + Send;
}

/// The device task did not answer in time (it is stuck or gone).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceUnavailable;

impl std::fmt::Display for DeviceUnavailable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the KNX device task did not answer")
    }
}

impl std::error::Error for DeviceUnavailable {}

/// Device management interface — ETS programming mode, device info.
///
/// Only available in device mode. Client mode has no device to manage. A call returns an error
/// after a couple of seconds instead of waiting for a device task that does not answer.
pub trait KnxDeviceControl: Send + Sync + 'static {
    /// Enable or disable KNX programming mode.
    fn set_prog_mode(
        &self,
        enabled: bool,
    ) -> impl std::future::Future<Output = Result<(), DeviceUnavailable>> + Send;

    /// Get current programming mode state.
    fn get_prog_mode(
        &self,
    ) -> impl std::future::Future<Output = Result<bool, DeviceUnavailable>> + Send;
}
