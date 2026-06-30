// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! Pure track-navigation index arithmetic for the zone player.
//!
//! Lifted out of the async handlers in [`super::helpers`] so the Next / Previous /
//! track-complete index logic (repeat modes, end-of-playlist, the CD-player ">3s
//! restarts the current track" Previous rule, and the shuffle draw) is a total
//! function of its inputs — unit-testable with no store, no decode task, and no
//! network. The handlers keep all side effects and call these for the decision only.

use snapdog_common::RepeatMode;

/// CD-player "previous" threshold: a Previous press past this position restarts the
/// current track instead of stepping back one.
pub const RESTART_THRESHOLD_MS: i64 = 3000;

/// Next-track index for an explicit Next on a Subsonic playlist.
///
/// `None` means stop (end of playlist with `repeat != Playlist`). Shuffle does not
/// affect an explicit Next, and `RepeatMode::Track` does NOT replay here — only
/// `Playlist` wraps to 0; `Off`/`Track` stop at the end.
#[must_use]
pub const fn next_index(current: usize, count: usize, repeat: RepeatMode) -> Option<usize> {
    let next = current + 1;
    if next < count {
        Some(next)
    } else if matches!(repeat, RepeatMode::Playlist) {
        Some(0)
    } else {
        None
    }
}

/// Previous-track index for a Subsonic playlist (CD-player rule).
///
/// `Some(current)` restarts the same track (position strictly past
/// [`RESTART_THRESHOLD_MS`]); `Some(current - 1)` steps back; `None` is a no-op
/// (already at index 0 within the threshold — no wrap-around, repeat not honored).
#[must_use]
pub const fn prev_index(current: usize, position_ms: i64) -> Option<usize> {
    if position_ms > RESTART_THRESHOLD_MS {
        Some(current)
    } else if current > 0 {
        Some(current - 1)
    } else {
        None
    }
}

/// Index selected when a track ends naturally, for a Subsonic playlist.
///
/// `draw` is the random index the caller supplies for the shuffle case (production
/// passes `fastrand::usize(..count)`), keeping this function deterministic. `None`
/// means "behave like an explicit [`next_index`]" — the caller delegates to Next,
/// which applies the end-of-playlist stop/wrap. Precedence: repeat-Track replays the
/// current track and wins over shuffle.
#[must_use]
pub const fn complete_index(
    current: usize,
    repeat: RepeatMode,
    shuffle: bool,
    draw: usize,
) -> Option<usize> {
    if matches!(repeat, RepeatMode::Track) {
        Some(current)
    } else if shuffle {
        Some(draw)
    } else {
        None
    }
}

/// Next station index for radio (modular wrap). `len` must be non-zero.
#[must_use]
pub const fn radio_next_index(current: usize, len: usize) -> usize {
    (current + 1) % len
}

/// Previous station index for radio (wraps to the last station at index 0).
/// `len` must be non-zero.
#[must_use]
pub const fn radio_prev_index(current: usize, len: usize) -> usize {
    if current == 0 { len - 1 } else { current - 1 }
}

#[cfg(test)]
mod tests {
    use super::{RESTART_THRESHOLD_MS, complete_index, next_index, prev_index};
    use super::{radio_next_index, radio_prev_index};
    use snapdog_common::RepeatMode::{Off, Playlist, Track};

    // ── next_index ────────────────────────────────────────────────
    #[test]
    fn next_advances_mid_playlist_regardless_of_repeat() {
        assert_eq!(next_index(2, 5, Off), Some(3));
        assert_eq!(next_index(2, 5, Track), Some(3));
        assert_eq!(next_index(2, 5, Playlist), Some(3));
    }

    #[test]
    fn next_at_end_stops_unless_playlist_repeat() {
        assert_eq!(next_index(4, 5, Off), None); // end + Off → stop
        assert_eq!(next_index(4, 5, Track), None); // Track does NOT replay on explicit Next
        assert_eq!(next_index(4, 5, Playlist), Some(0)); // wrap to start
    }

    #[test]
    fn next_single_track() {
        assert_eq!(next_index(0, 1, Off), None);
        assert_eq!(next_index(0, 1, Playlist), Some(0));
    }

    // ── prev_index (CD-player >3s rule) ───────────────────────────
    #[test]
    fn prev_restarts_past_threshold_else_steps_back() {
        assert_eq!(prev_index(3, RESTART_THRESHOLD_MS + 1), Some(3)); // 3001 → restart
        assert_eq!(prev_index(3, RESTART_THRESHOLD_MS), Some(2)); // exactly 3000 → back
        assert_eq!(prev_index(3, 2999), Some(2));
        assert_eq!(prev_index(3, 0), Some(2));
    }

    #[test]
    fn prev_at_index_zero() {
        assert_eq!(prev_index(0, 0), None); // no wrap, no-op
        assert_eq!(prev_index(0, 5000), Some(0)); // restart still works at index 0
    }

    // ── complete_index (Complete vs Next asymmetry) ───────────────
    #[test]
    fn complete_repeat_track_replays_current() {
        assert_eq!(complete_index(2, Track, false, 0), Some(2));
        assert_eq!(complete_index(4, Track, false, 0), Some(4)); // end: replays (Next would stop)
    }

    #[test]
    fn complete_shuffle_returns_draw_verbatim() {
        for k in 0..5 {
            assert_eq!(complete_index(2, Off, true, k), Some(k));
        }
        assert_eq!(complete_index(2, Off, true, 2), Some(2)); // may repeat current (no exclusion)
    }

    #[test]
    fn complete_repeat_track_wins_over_shuffle() {
        assert_eq!(complete_index(4, Track, true, 1), Some(4));
    }

    #[test]
    fn complete_otherwise_delegates_to_next() {
        // None == "delegate to Next" — the caller runs next_index (stop/wrap).
        assert_eq!(complete_index(2, Off, false, 0), None);
        assert_eq!(complete_index(4, Playlist, false, 0), None);
    }

    // ── radio wrap ────────────────────────────────────────────────
    #[test]
    fn radio_next_and_prev_wrap() {
        assert_eq!(radio_next_index(0, 3), 1);
        assert_eq!(radio_next_index(2, 3), 0); // wrap forward
        assert_eq!(radio_prev_index(0, 3), 2); // wrap back
        assert_eq!(radio_prev_index(2, 3), 1);
    }
}
