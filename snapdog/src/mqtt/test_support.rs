// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! Fixtures shared by the MQTT unit tests.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::{RwLock, mpsc};

use crate::player::{SnapcastCmd, ZoneCommand, ZoneCommandSender};
use crate::state;

/// Two zones and one connected client (`snap-1`) in zone 1.
pub fn test_state_with_client() -> state::SharedState {
    let store: state::Store = serde_json::from_value(serde_json::json!({
        "zones": {
            "1": { "name": "Zone 1", "icon": "", "volume": 50, "muted": false, "playback": "stopped", "shuffle": false, "repeat": "off", "track": null, "playlist_index": null, "playlist_name": null, "playlist_total": null, "playlist_track_index": null, "playlist_track_count": null, "source": "idle", "cover_url": null, "snapcast_group_id": null, "presence": false, "presence_enabled": true, "presence_source": false, "auto_off_active": false, "auto_off_delay": 0 },
            "2": { "name": "Zone 2", "icon": "", "volume": 50, "muted": false, "playback": "stopped", "shuffle": false, "repeat": "off", "track": null, "playlist_index": null, "playlist_name": null, "playlist_total": null, "playlist_track_index": null, "playlist_track_count": null, "source": "idle", "cover_url": null, "snapcast_group_id": null, "presence": false, "presence_enabled": true, "presence_source": false, "auto_off_active": false, "auto_off_delay": 0 }
        },
        "clients": {
            "1": {
                "name": "Test", "icon": "", "mac": "", "zone_index": 1,
                "volume": 50, "base_volume": 50, "muted": false,
                "latency_ms": 0, "connected": true, "snapcast_id": "snap-1"
            }
        }
    }))
    .unwrap();
    Arc::new(RwLock::new(store))
}

/// A command channel for zone 1 with room for `capacity` commands.
pub fn zone_channels_with(
    capacity: usize,
) -> (
    HashMap<usize, ZoneCommandSender>,
    mpsc::Receiver<ZoneCommand>,
) {
    let (tx, rx) = mpsc::channel(capacity);
    let mut map = HashMap::new();
    map.insert(1, tx);
    (map, rx)
}

/// A command channel for zone 1 with room for 16 commands.
pub fn zone_channels() -> (
    HashMap<usize, ZoneCommandSender>,
    mpsc::Receiver<ZoneCommand>,
) {
    zone_channels_with(16)
}

/// A Snapcast command channel with room for 16 commands.
pub fn snap_channel() -> (mpsc::Sender<SnapcastCmd>, mpsc::Receiver<SnapcastCmd>) {
    mpsc::channel(16)
}
