//! The link between configuration keys, group objects and actions, one table per object kind.
//!
//! Client mode (address → action map, status read set), device mode (association table built
//! from the configuration) and the bus-written objects of device mode (object → action) are all
//! derived from these tables. Adding an object takes one row here, its config key, its entry in
//! the group object list and its behaviour in `execute` or the publishers. The tests at the end
//! fail when a row and a config key, or a row and the object's flags, disagree.

use super::group_objects::{
    CGO_CONNECTED, CGO_LATENCY, CGO_LATENCY_STATUS, CGO_MUTE, CGO_MUTE_STATUS, CGO_MUTE_TOGGLE,
    CGO_VOLUME, CGO_VOLUME_DIM, CGO_VOLUME_STATUS, CGO_ZONE, CGO_ZONE_STATUS, GGO_ALL_MUTE,
    GGO_ALL_STOP, GGO_KNX_TIME, GGO_SERVER_ONLINE, GGO_SYSTEM_FAULT, ZGO_CONTROL_STATUS, ZGO_MUTE,
    ZGO_MUTE_STATUS, ZGO_MUTE_TOGGLE, ZGO_PAUSE, ZGO_PLAY, ZGO_PLAY_PAUSE, ZGO_PLAYLIST,
    ZGO_PLAYLIST_NAME, ZGO_PLAYLIST_NEXT, ZGO_PLAYLIST_PREVIOUS, ZGO_PLAYLIST_STATUS,
    ZGO_PLAYLIST_STEP, ZGO_PLAYLIST_STEP_UPDOWN, ZGO_PRESENCE, ZGO_PRESENCE_ENABLE,
    ZGO_PRESENCE_TIMER_ACTIVE, ZGO_REPEAT, ZGO_REPEAT_STATUS, ZGO_REPEAT_TOGGLE, ZGO_SHUFFLE,
    ZGO_SHUFFLE_STATUS, ZGO_SHUFFLE_TOGGLE, ZGO_STOP, ZGO_TRACK_ALBUM, ZGO_TRACK_ARTIST,
    ZGO_TRACK_NEXT, ZGO_TRACK_NUMBER, ZGO_TRACK_NUMBER_STATUS, ZGO_TRACK_PLAYING,
    ZGO_TRACK_POSITION, ZGO_TRACK_PREVIOUS, ZGO_TRACK_PROGRESS, ZGO_TRACK_REPEAT,
    ZGO_TRACK_REPEAT_STATUS, ZGO_TRACK_REPEAT_TOGGLE, ZGO_TRACK_SEEK, ZGO_TRACK_SEEK_UPDOWN,
    ZGO_TRACK_STEP, ZGO_TRACK_STEP_UPDOWN, ZGO_TRACK_TITLE, ZGO_VOLUME, ZGO_VOLUME_DIM,
    ZGO_VOLUME_STATUS,
};

/// One configuration key: the object it is linked to and, for a key the bus writes, the action
/// a telegram on it triggers. A key without an action is a status address the server sends on
/// and answers reads for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyedObject {
    /// The configuration key (`[zone.knx]`, `[client.knx]` or `[knx]`).
    pub key: &'static str,
    /// Object number inside its kind (`ZGO_*`, `CGO_*`, `GGO_*`).
    pub go: usize,
    /// Action name for `execute`, `None` for a status key.
    pub action: Option<&'static str>,
}

impl KeyedObject {
    /// Whether the server sends on this key and answers reads for it.
    #[must_use]
    pub const fn is_status(&self) -> bool {
        self.action.is_none()
    }
}

const fn command(key: &'static str, go: usize, action: &'static str) -> KeyedObject {
    KeyedObject {
        key,
        go,
        action: Some(action),
    }
}

const fn status(key: &'static str, go: usize) -> KeyedObject {
    KeyedObject {
        key,
        go,
        action: None,
    }
}

/// Zone keys. Where a command and its status share one object, the status row comes first: the
/// device stack sends on the first association of an object.
pub const ZONE_OBJECTS: &[KeyedObject] = &[
    command("play", ZGO_PLAY, "play"),
    command("pause", ZGO_PAUSE, "pause"),
    command("stop", ZGO_STOP, "stop"),
    command("track_next", ZGO_TRACK_NEXT, "next"),
    command("track_previous", ZGO_TRACK_PREVIOUS, "previous"),
    command("volume", ZGO_VOLUME, "volume"),
    status("volume_status", ZGO_VOLUME_STATUS),
    command("volume_dim", ZGO_VOLUME_DIM, "volume_dim"),
    command("mute", ZGO_MUTE, "mute"),
    status("mute_status", ZGO_MUTE_STATUS),
    command("mute_toggle", ZGO_MUTE_TOGGLE, "mute_toggle"),
    status("control_status", ZGO_CONTROL_STATUS),
    status("track_playing_status", ZGO_TRACK_PLAYING),
    command("shuffle", ZGO_SHUFFLE, "shuffle"),
    status("shuffle_status", ZGO_SHUFFLE_STATUS),
    command("shuffle_toggle", ZGO_SHUFFLE_TOGGLE, "shuffle_toggle"),
    command("repeat", ZGO_REPEAT, "repeat"),
    status("repeat_status", ZGO_REPEAT_STATUS),
    command("repeat_toggle", ZGO_REPEAT_TOGGLE, "repeat_toggle"),
    command("track_repeat", ZGO_TRACK_REPEAT, "track_repeat"),
    status("track_repeat_status", ZGO_TRACK_REPEAT_STATUS),
    command(
        "track_repeat_toggle",
        ZGO_TRACK_REPEAT_TOGGLE,
        "track_repeat_toggle",
    ),
    command("playlist", ZGO_PLAYLIST, "playlist"),
    status("playlist_status", ZGO_PLAYLIST_STATUS),
    command("playlist_next", ZGO_PLAYLIST_NEXT, "playlist_next"),
    command(
        "playlist_previous",
        ZGO_PLAYLIST_PREVIOUS,
        "playlist_previous",
    ),
    command("playlist_step", ZGO_PLAYLIST_STEP, "playlist_step"),
    command(
        "playlist_step_updown",
        ZGO_PLAYLIST_STEP_UPDOWN,
        "playlist_step_updown",
    ),
    command("track_number", ZGO_TRACK_NUMBER, "track_number"),
    status("track_number_status", ZGO_TRACK_NUMBER_STATUS),
    status("playlist_name_status", ZGO_PLAYLIST_NAME),
    command("play_pause", ZGO_PLAY_PAUSE, "play_pause"),
    command("track_step", ZGO_TRACK_STEP, "track_step"),
    command(
        "track_step_updown",
        ZGO_TRACK_STEP_UPDOWN,
        "track_step_updown",
    ),
    command("track_position", ZGO_TRACK_POSITION, "track_position"),
    command("track_seek", ZGO_TRACK_SEEK, "track_seek"),
    command(
        "track_seek_updown",
        ZGO_TRACK_SEEK_UPDOWN,
        "track_seek_updown",
    ),
    status("track_title_status", ZGO_TRACK_TITLE),
    status("track_artist_status", ZGO_TRACK_ARTIST),
    status("track_album_status", ZGO_TRACK_ALBUM),
    status("track_progress_status", ZGO_TRACK_PROGRESS),
    command("presence", ZGO_PRESENCE, "presence"),
    status("presence_enable_status", ZGO_PRESENCE_ENABLE),
    command("presence_enable", ZGO_PRESENCE_ENABLE, "presence_enable"),
    status("presence_timer_status", ZGO_PRESENCE_TIMER_ACTIVE),
];

/// Client keys, ordered like [`ZONE_OBJECTS`].
pub const CLIENT_OBJECTS: &[KeyedObject] = &[
    command("volume", CGO_VOLUME, "volume"),
    status("volume_status", CGO_VOLUME_STATUS),
    command("volume_dim", CGO_VOLUME_DIM, "volume_dim"),
    command("mute", CGO_MUTE, "mute"),
    status("mute_status", CGO_MUTE_STATUS),
    command("mute_toggle", CGO_MUTE_TOGGLE, "mute_toggle"),
    command("latency", CGO_LATENCY, "latency"),
    status("latency_status", CGO_LATENCY_STATUS),
    command("zone", CGO_ZONE, "zone"),
    status("zone_status", CGO_ZONE_STATUS),
    status("connected_status", CGO_CONNECTED),
];

/// Device-level keys under `[knx]`, ordered like [`ZONE_OBJECTS`].
pub const GLOBAL_OBJECTS: &[KeyedObject] = &[
    status("server_online", GGO_SERVER_ONLINE),
    command("all_stop", GGO_ALL_STOP, "all_stop"),
    status("all_mute_status", GGO_ALL_MUTE),
    command("all_mute", GGO_ALL_MUTE, "all_mute"),
    status("system_fault", GGO_SYSTEM_FAULT),
    command("knx_time", GGO_KNX_TIME, "knx_time"),
];

/// The configured keys of one zone, client or `[knx]` section, in table order, with their
/// address. `keys` is the section's own key list (`group_addresses()`); a key the table does
/// not know is skipped here and fails the tests below.
pub fn configured<'a>(
    table: &'static [KeyedObject],
    keys: &[(&'static str, Option<&'a str>)],
) -> Vec<(&'static KeyedObject, &'a str)> {
    table
        .iter()
        .filter_map(|row| {
            keys.iter()
                .find(|(key, _)| *key == row.key)
                .and_then(|(_, ga)| *ga)
                .map(|ga| (row, ga))
        })
        .collect()
}

/// The action of the object `go`, or `None` for an object the bus does not write.
pub fn action_of(table: &'static [KeyedObject], go: usize) -> Option<&'static str> {
    table
        .iter()
        .find(|row| row.go == go && row.action.is_some())
        .and_then(|row| row.action)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ClientKnxAddresses, KnxConfig, ZoneKnxAddresses};
    use crate::knx::group_objects::{CLIENT_GOS, GLOBAL_GOS, GoDefinition, ZONE_GOS};

    fn keys_of(list: &[(&'static str, Option<&str>)]) -> Vec<&'static str> {
        list.iter().map(|(k, _)| *k).collect()
    }

    /// Every config key has exactly one row and every row a config key; every object has at
    /// least one row; a command row is an object the bus writes, a status row one the server
    /// sends on.
    fn check(table: &[KeyedObject], keys: &[&'static str], gos: &[GoDefinition], kind: &str) {
        for key in keys {
            let rows = table.iter().filter(|r| r.key == *key).count();
            assert_eq!(rows, 1, "{kind} key {key} has {rows} rows");
        }
        for row in table {
            assert!(
                keys.contains(&row.key),
                "{kind} row {} is no config key",
                row.key
            );
            let flags = &gos[row.go].flags;
            if row.is_status() {
                assert!(
                    flags.transmit,
                    "{kind} {} is a status key on a receive-only object",
                    row.key
                );
            } else {
                assert!(
                    flags.write,
                    "{kind} {} has an action on an object the bus cannot write",
                    row.key
                );
            }
        }
        for (go, def) in gos.iter().enumerate() {
            assert!(
                table.iter().any(|r| r.go == go),
                "{kind} object {} has no config key",
                def.name
            );
            let has_action = table.iter().any(|r| r.go == go && r.action.is_some());
            assert_eq!(
                has_action, def.flags.write,
                "{kind} object {}: an action exists exactly for objects the bus writes",
                def.name
            );
        }
    }

    #[test]
    fn the_zone_table_matches_the_config_keys_and_the_objects() {
        let keys = keys_of(&ZoneKnxAddresses::default().group_addresses());
        check(ZONE_OBJECTS, &keys, ZONE_GOS, "zone");
    }

    #[test]
    fn the_client_table_matches_the_config_keys_and_the_objects() {
        let keys = keys_of(&ClientKnxAddresses::default().group_addresses());
        check(CLIENT_OBJECTS, &keys, CLIENT_GOS, "client");
    }

    #[test]
    fn the_global_table_matches_the_config_keys_and_the_objects() {
        let keys = KnxConfig::GROUP_ADDRESS_KEYS.to_vec();
        check(GLOBAL_OBJECTS, &keys, GLOBAL_GOS, "global");
    }

    #[test]
    fn a_shared_object_lists_its_status_key_first() {
        for table in [ZONE_OBJECTS, CLIENT_OBJECTS, GLOBAL_OBJECTS] {
            for (i, row) in table.iter().enumerate() {
                if row.is_status() {
                    continue;
                }
                if let Some(status_at) = table.iter().position(|r| r.go == row.go && r.is_status())
                {
                    assert!(status_at < i, "{} comes before its status key", row.key);
                }
            }
        }
    }

    #[test]
    fn configured_keeps_table_order_and_skips_unset_keys() {
        let keys = [
            ("mute", Some("1/1/2")),
            ("volume", Some("1/1/1")),
            ("zone", None),
        ];
        let got: Vec<_> = configured(CLIENT_OBJECTS, &keys)
            .into_iter()
            .map(|(row, ga)| (row.key, ga))
            .collect();
        assert_eq!(got, [("volume", "1/1/1"), ("mute", "1/1/2")]);
        assert_eq!(action_of(CLIENT_OBJECTS, CGO_ZONE), Some("zone"));
        assert_eq!(action_of(CLIENT_OBJECTS, CGO_ZONE_STATUS), None);
    }
}
