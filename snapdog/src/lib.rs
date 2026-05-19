// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! `SnapDog` library — re-exports all modules for integration tests.

// Pedantic lints allowed crate-wide: audio/KNX code uses intentional numeric casts,
// float comparisons are acceptable for gain/volume, long functions are unavoidable in
// protocol handlers, must_use on every helper is noise, future_not_send is expected
// with tokio::spawn, struct_excessive_bools reflects protocol state, and
// implicit_hasher is fixed at public API boundaries but not for all internal helpers.
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_sign_loss)]
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::float_cmp)]
#![allow(clippy::too_many_lines)]
#![allow(clippy::must_use_candidate)]
#![allow(clippy::future_not_send)]
#![allow(clippy::struct_excessive_bools)]
#![allow(clippy::implicit_hasher)]
#![allow(clippy::branches_sharing_code)]
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
