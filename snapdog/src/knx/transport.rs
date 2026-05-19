// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! KNX transport abstractions — shared interfaces for client and device modes.

use knx_rs_core::address::GroupAddress;
use knx_rs_core::dpt::{Dpt, DptValue};

/// Write-only interface to the KNX bus.
///
/// Used by the publisher task to send group value writes.
/// Immutable (`&self`) — can be shared across tasks via `Arc`.
pub trait KnxPublisher: Send + Sync + 'static {
    /// Write a typed value to a group address.
    fn write(
        &self,
        ga: GroupAddress,
        dpt: Dpt,
        value: &DptValue,
    ) -> impl std::future::Future<Output = ()> + Send;
}

/// Read-only interface to the KNX bus.
///
/// Used by the listener task to receive group value writes from the bus.
/// Mutable (`&mut self`) — owned by a single task.
pub trait KnxListener: Send + 'static {
    /// Receive the next group write from the bus.
    /// Returns `None` if the connection is closed.
    fn recv_group_write(
        &mut self,
    ) -> impl std::future::Future<Output = Option<(GroupAddress, Vec<u8>)>> + Send;
}

/// Device management interface — ETS programming mode, device info.
///
/// Only available in device mode. Client mode has no device to manage.
pub trait KnxDeviceControl: Send + Sync + 'static {
    /// Enable or disable KNX programming mode.
    fn set_prog_mode(&self, enabled: bool) -> impl std::future::Future<Output = ()> + Send;

    /// Get current programming mode state.
    fn get_prog_mode(&self) -> impl std::future::Future<Output = bool> + Send;
}
