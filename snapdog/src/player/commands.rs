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

    // ── Seek ──────────────────────────────────────────────────
    /// Seek to an absolute position in milliseconds.
    Seek(i64),
    /// Seek by a relative offset in milliseconds (positive = forward, negative = backward).
    SeekRelative(i64),
    /// Seek to a relative position (0.0–1.0 of total duration).
    SeekProgress(f64),

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
    /// Set playlist repeat on or off.
    SetRepeat(bool),
    /// Toggle playlist repeat.
    ToggleRepeat,
    /// Set single-track repeat on or off.
    SetTrackRepeat(bool),
    /// Toggle single-track repeat.
    ToggleTrackRepeat,

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
