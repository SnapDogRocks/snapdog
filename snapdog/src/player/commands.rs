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

#[cfg(test)]
mod tests {
    use super::ZoneCommand;

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
    Url,
    /// Receiving audio from an `AirPlay` client.
    AirPlay,
    /// Receiving audio from a Spotify Connect client.
    Spotify,
}
