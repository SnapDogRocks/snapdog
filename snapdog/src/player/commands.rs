// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! `ZonePlayer` command definitions.

/// Commands sent to a `ZonePlayer` from API, MQTT, or KNX.
#[derive(Debug)]
pub enum ZoneCommand {
    // ── Source selection ───────────────────────────────────────
    /// Start playing a Subsonic playlist from the given track index.
    PlaySubsonicPlaylist(String, usize),
    /// Start playing a single Subsonic track by ID.
    PlaySubsonicTrack(String),
    /// Start playing an arbitrary audio URL.
    PlayUrl(String),
    /// Start a unified playlist by index, beginning at the given track.
    PlayPlaylist(usize, usize),
    /// Jump to track N in the current playlist.
    SetTrack(usize),

    // ── Transport ─────────────────────────────────────────────
    /// Resume playback or restart the current source.
    Play,
    /// Pause playback (keeps source loaded).
    Pause,
    /// Stop playback and return to idle.
    Stop,
    /// Next track (playlist) or next station (radio).
    Next,
    /// Previous track (playlist) or previous station (radio).
    Previous,

    // ── Playlist navigation ───────────────────────────────────
    /// Switch to the next playlist in the unified playlist index.
    NextPlaylist,
    /// Switch to the previous playlist in the unified playlist index.
    PreviousPlaylist,
    /// Switch to a specific playlist by index, starting at the given track.
    SetPlaylist(usize, usize),
    /// Start a playlist by `playlist_id`, beginning at the given track. Unknown ids are ignored.
    PlayPlaylistById(String, usize),
    /// Switch to a playlist by `playlist_id`, starting at the given track. Unknown ids are ignored.
    SetPlaylistById(String, usize),

    // ── Seek ──────────────────────────────────────────────────
    /// Seek to an absolute position in milliseconds.
    Seek(i64),
    /// Seek by a relative offset in milliseconds (positive = forward, negative = backward).
    SeekRelative(i64),
    /// Seek to a relative position (0.0–1.0 of total duration).
    SeekProgress(f64),
    /// Seek by one step of the zone's seek interval (`true` = forward, `false` = back).
    SeekStep(bool),

    // ── Zone settings ─────────────────────────────────────────
    /// Set the zone volume (0–100).
    SetVolume(i32),
    /// Adjust the zone volume by a relative delta (from DPT 3.007 dimming).
    AdjustVolume(i32),
    /// Set the zone mute state.
    SetMute(bool),
    /// Toggle the zone mute state.
    ToggleMute,
    /// Set playlist shuffle on or off.
    SetShuffle(bool),
    /// Toggle playlist shuffle.
    ToggleShuffle,
    /// Set repeat mode.
    SetRepeat(snapdog_common::RepeatMode),
    /// Cycle repeat mode: Off → Playlist → Track → Off.
    CycleRepeat,
    /// Switch between Off and the given mode (see [`toggled_repeat`]).
    ToggleRepeatMode(snapdog_common::RepeatMode),
    /// Switch repeat off, but only while the given mode is active (see [`cleared_repeat`]).
    ClearRepeatMode(snapdog_common::RepeatMode),

    // ── Presence ──────────────────────────────────────────────
    /// Set presence state (from KNX occupancy sensor, MQTT, or API).
    SetPresence(bool),
    /// Enable or disable presence-triggered playback.
    SetPresenceEnabled(bool),
    /// Set auto-off delay in seconds.
    SetAutoOffDelay(u16),

    // ── DSP ───────────────────────────────────────────────────
    /// Apply a new parametric EQ configuration to the zone.
    SetEq(crate::audio::eq::EqConfig),
}

impl ZoneCommand {
    /// A seek of one step of `interval_secs` seconds, forward or back.
    #[must_use]
    pub fn seek_by_interval(interval_secs: u8, forward: bool) -> Self {
        let step_ms = i64::from(interval_secs) * 1000;
        Self::SeekRelative(if forward { step_ms } else { -step_ms })
    }
}

/// The repeat mode after toggling `mode`: `mode` when it is not active, Off when it is.
///
/// Toggling one mode while the other is active switches to the toggled one, as a press on a
/// "Track Repeat" button does while the playlist repeats.
#[must_use]
pub fn toggled_repeat(
    current: snapdog_common::RepeatMode,
    mode: snapdog_common::RepeatMode,
) -> snapdog_common::RepeatMode {
    if current == mode {
        snapdog_common::RepeatMode::Off
    } else {
        mode
    }
}

/// The repeat mode after switching `mode` off: Off when `mode` is active, otherwise unchanged.
#[must_use]
pub fn cleared_repeat(
    current: snapdog_common::RepeatMode,
    mode: snapdog_common::RepeatMode,
) -> snapdog_common::RepeatMode {
    if current == mode {
        snapdog_common::RepeatMode::Off
    } else {
        current
    }
}

#[cfg(test)]
mod tests {
    use super::{ZoneCommand, cleared_repeat, toggled_repeat};

    #[test]
    fn toggling_a_repeat_mode_switches_between_off_and_that_mode() {
        use snapdog_common::RepeatMode::{Off, Playlist, Track};
        // (current, toggled mode, result)
        for (current, mode, expected) in [
            (Off, Playlist, Playlist),
            (Playlist, Playlist, Off),
            (Track, Playlist, Playlist),
            (Off, Track, Track),
            (Track, Track, Off),
            (Playlist, Track, Track),
        ] {
            assert_eq!(
                toggled_repeat(current, mode),
                expected,
                "{current:?} toggle {mode:?}"
            );
        }
    }

    #[test]
    fn clearing_a_repeat_mode_leaves_the_other_mode_alone() {
        use snapdog_common::RepeatMode::{Off, Playlist, Track};
        for (current, mode, expected) in [
            (Playlist, Playlist, Off),
            (Track, Playlist, Track),
            (Off, Playlist, Off),
            (Track, Track, Off),
            (Playlist, Track, Playlist),
            (Off, Track, Off),
        ] {
            assert_eq!(
                cleared_repeat(current, mode),
                expected,
                "{current:?} clear {mode:?}"
            );
        }
    }

    #[test]
    fn a_seek_step_is_the_interval_in_milliseconds_forward_or_back() {
        assert!(matches!(
            ZoneCommand::seek_by_interval(10, true),
            ZoneCommand::SeekRelative(10_000)
        ));
        assert!(matches!(
            ZoneCommand::seek_by_interval(60, false),
            ZoneCommand::SeekRelative(-60_000)
        ));
    }
}

/// What the `ZonePlayer` is currently doing.
#[derive(Debug, Clone)]
pub enum ActiveSource {
    /// No active source — zone is silent.
    Idle,
    /// Playing an internet radio station.
    Radio {
        /// Index into the resolved radio station list.
        index: usize,
    },
    /// Playing tracks from a Subsonic playlist.
    SubsonicPlaylist {
        /// Subsonic playlist ID.
        playlist_id: String,
        /// Current track position within the playlist.
        track_index: usize,
        /// Total number of tracks in the playlist.
        track_count: usize,
    },
    /// Playing a single Subsonic track (not part of a playlist).
    SubsonicTrack {
        /// Subsonic track ID.
        track_id: String,
    },
    /// Playing an arbitrary URL.
    Url {
        /// The URL, kept so that Play after Pause can start it again.
        url: String,
    },
    /// Receiving audio from an `AirPlay` client.
    AirPlay,
    /// Receiving audio from a Spotify Connect client.
    Spotify,
}
