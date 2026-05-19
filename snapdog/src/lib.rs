// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! `SnapDog` library — re-exports all modules for integration tests.

#![forbid(unsafe_code)]
#![warn(clippy::redundant_closure)]
#![warn(clippy::implicit_clone)]
#![warn(clippy::uninlined_format_args)]
#![warn(missing_docs)]

pub mod api;
pub mod audio;
pub mod config;
pub mod knx;
pub mod mqtt;
pub mod player;
pub mod process;
pub mod receiver;
pub mod snapcast;
pub mod spinorama;

/// Shared HTTP User-Agent string for external requests (cover art, streams, Subsonic).
pub const USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

/// Client name used by `SnapDog` clients to identify themselves to the server.
pub use snapdog_common::CLIENT_NAME as SNAPDOG_CLIENT_NAME;
pub mod state;
pub mod subsonic;
