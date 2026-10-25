// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! Client-mode KNX transport — connects to an external KNX/IP gateway.

use knx_rs_core::dpt::{Dpt, DptValue};
use knx_rs_ip::multiplex::MultiplexHandle;
use knx_rs_ip::ops::GroupOps;

use super::transport::{Incoming, KnxListener, KnxPublisher, Target};

/// Client-mode publisher: sends group writes via a `MultiplexHandle`.
pub struct ClientPublisher {
    handle: MultiplexHandle,
}

impl ClientPublisher {
    pub(crate) const fn new(handle: MultiplexHandle) -> Self {
        Self { handle }
    }
}

impl KnxPublisher for ClientPublisher {
    const ADDRESSED: bool = true;

    async fn write(&self, target: Target, dpt: Dpt, value: &DptValue) -> bool {
        let Target::Address(ga) = target else {
            return false;
        };
        if let Err(e) = self.handle.group_write_value(ga, dpt, value).await {
            tracing::warn!(ga = %ga, error = %e, "KNX write failed");
            return false;
        }
        true
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
