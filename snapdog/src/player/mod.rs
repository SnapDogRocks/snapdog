// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! `ZonePlayer` — per-zone audio pipeline with command channel.

mod commands;
pub mod context;
mod helpers;
mod nav;
mod runner;

pub use commands::ZoneCommand;
pub use context::{
    ClientAction, GroupAction, SnapcastCmd, SnapcastCmdSender, ZoneCommandSender, ZonePlayerContext,
};
pub use runner::spawn_zone_players;
