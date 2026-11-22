// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! `org.mpris.MediaPlayer2.Player` interface.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::{Mutex, mpsc};
use zbus::Variant;

use crate::player::ZoneCommand;

/// The MPRIS object path for "no track".
pub const NO_TRACK: &str = "/org/mpris/MediaPlayer2/TrackList/NoTrack";

/// A position that differs from the expected one by more than this is a seek, not
/// playback drift or a decoder that runs ahead.
const SEEK_THRESHOLD_MS: i64 = 2_000;

/// Shared mutable player state updated by the notification listener.
#[derive(Debug, Clone)]
pub struct PlayerState {
    pub playback: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub cover_url: Option<String>,
    pub duration_ms: i64,
    pub position_ms: i64,
    /// When `position_ms` was last reported, to tell a seek from playback.
    pub position_seen_at: Option<std::time::Instant>,
    /// `mpris:trackid`: an object path that changes with the track, [`NO_TRACK`] without one.
    pub track_id: String,
    pub volume: i32,
    pub muted: bool,
    pub seekable: bool,
    pub can_next: bool,
    pub can_prev: bool,
    pub shuffle: bool,
    pub repeat: bool,
    pub track_repeat: bool,
}

impl Default for PlayerState {
    fn default() -> Self {
        Self {
            playback: "Stopped".into(),
            title: String::new(),
            artist: String::new(),
            album: String::new(),
            cover_url: None,
            duration_ms: 0,
            position_ms: 0,
            position_seen_at: None,
            track_id: NO_TRACK.into(),
            volume: 100,
            muted: false,
            seekable: false,
            can_next: false,
            can_prev: false,
            shuffle: false,
            repeat: false,
            track_repeat: false,
        }
    }
}

/// What identifies a track for `mpris:trackid`.
#[derive(Hash)]
pub struct TrackKey<'a> {
    pub source: &'a str,
    pub playlist_id: Option<&'a str>,
    pub track_index: Option<usize>,
    pub title: &'a str,
    pub artist: &'a str,
    pub album: &'a str,
    pub duration_ms: i64,
}

/// The `mpris:trackid` of a track: a valid object path (`[A-Za-z0-9_]` elements) that
/// differs between tracks, or [`NO_TRACK`] when nothing is loaded.
pub fn track_object_path(zone_index: usize, key: &TrackKey<'_>) -> String {
    use std::hash::{Hash, Hasher};
    if key.title.is_empty() && key.duration_ms == 0 {
        return NO_TRACK.into();
    }
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    key.hash(&mut hasher);
    format!(
        "/org/mpris/MediaPlayer2/snapdog/zone{zone_index}/track/t{:016x}",
        hasher.finish()
    )
}

/// Map a zone playback state to the MPRIS `PlaybackStatus`.
pub fn playback_status(playback: &str) -> &'static str {
    match playback {
        "playing" => "Playing",
        "paused" => "Paused",
        _ => "Stopped",
    }
}

impl PlayerState {
    /// The state of a zone as the store holds it, so the player reads right before the
    /// first notification arrives.
    pub fn from_zone(zone_index: usize, zone: &crate::state::ZoneState) -> Self {
        let track = zone.track.as_ref();
        let title = track.map_or_else(String::new, |t| t.title.clone());
        let artist = track.map_or_else(String::new, |t| t.artist.clone());
        let album = track.map_or_else(String::new, |t| t.album.clone());
        let duration_ms = track.map_or(0, |t| t.duration_ms);
        let source = zone.source.to_string();
        let track_id = track_object_path(
            zone_index,
            &TrackKey {
                source: &source,
                playlist_id: zone.playlist_id.as_deref(),
                track_index: zone.playlist_track_index,
                title: &title,
                artist: &artist,
                album: &album,
                duration_ms,
            },
        );
        Self {
            playback: playback_status(&zone.playback.to_string()).into(),
            position_ms: track.map_or(0, |t| t.position_ms),
            position_seen_at: None,
            seekable: track.is_some_and(|t| t.seekable),
            title,
            artist,
            album,
            duration_ms,
            track_id,
            cover_url: zone.cover_url.clone(),
            volume: zone.volume,
            muted: zone.muted,
            can_next: zone
                .playlist_track_count
                .is_some_and(|c| zone.playlist_track_index.is_some_and(|i| i + 1 < c)),
            can_prev: zone.playlist_track_index.is_some_and(|i| i > 0),
            shuffle: zone.shuffle,
            repeat: zone.repeat == snapdog_common::RepeatMode::Playlist,
            track_repeat: zone.repeat == snapdog_common::RepeatMode::Track,
        }
    }

    /// Take a new position report. Returns `true` when it is a seek: the same track, and
    /// a position that is not where playback since the last report would have taken it.
    pub fn report_position(&mut self, position_ms: i64, now: std::time::Instant) -> bool {
        let jumped = self.position_seen_at.is_some_and(|at| {
            let elapsed = if self.playback == "Playing" {
                i64::try_from(now.saturating_duration_since(at).as_millis()).unwrap_or(i64::MAX)
            } else {
                0
            };
            (position_ms - self.position_ms.saturating_add(elapsed)).abs() > SEEK_THRESHOLD_MS
        });
        self.position_ms = position_ms;
        self.position_seen_at = Some(now);
        jumped
    }
}

/// MPRIS2 Player interface implementation.
pub struct PlayerInterface {
    pub art_base: String,
    pub state: Arc<Mutex<PlayerState>>,
    cmd_tx: mpsc::Sender<ZoneCommand>,
}

impl PlayerInterface {
    pub const fn new(
        art_base: String,
        state: Arc<Mutex<PlayerState>>,
        cmd_tx: mpsc::Sender<ZoneCommand>,
    ) -> Self {
        Self {
            art_base,
            state,
            cmd_tx,
        }
    }

    /// Hand a command to the zone without waiting: zbus answers the calls of a connection
    /// one after the other, so a blocked send would stall every later call. A full or
    /// closed channel is an error for the caller, not a silent success.
    fn send(&self, cmd: ZoneCommand) -> zbus::fdo::Result<()> {
        self.cmd_tx.try_send(cmd).map_err(|e| match e {
            mpsc::error::TrySendError::Full(_) => {
                zbus::fdo::Error::LimitsExceeded("the zone is busy, try again".into())
            }
            mpsc::error::TrySendError::Closed(_) => {
                zbus::fdo::Error::Failed("the zone is not running".into())
            }
        })
    }

    pub fn build_metadata(&self, s: &PlayerState) -> HashMap<String, Variant<'static>> {
        let mut m: HashMap<String, Variant<'static>> = HashMap::new();
        // Type `o`, as the specification requires, not a string.
        let track_id = zbus::OwnedObjectPath::try_from(s.track_id.clone())
            .unwrap_or_else(|_| zbus::OwnedObjectPath::try_from(NO_TRACK).expect("valid path"));
        m.insert("mpris:trackid".into(), Variant::from(track_id));
        if !s.title.is_empty() {
            m.insert("xesam:title".into(), Variant::new(s.title.clone()));
        }
        if !s.artist.is_empty() {
            m.insert("xesam:artist".into(), Variant::new(vec![s.artist.clone()]));
        }
        if !s.album.is_empty() {
            m.insert("xesam:album".into(), Variant::new(s.album.clone()));
        }
        if let Some(ref url) = s.cover_url {
            let art_url = if url.starts_with("http") {
                url.clone()
            } else {
                format!("{}{url}", self.art_base)
            };
            m.insert("mpris:artUrl".into(), Variant::new(art_url));
        }
        if s.duration_ms > 0 {
            m.insert(
                "mpris:length".into(),
                Variant::new(s.duration_ms * 1000), // ms → µs
            );
        }
        m
    }
}

#[allow(
    unknown_lints,
    clippy::unused_self,
    clippy::missing_const_for_fn,
    clippy::used_underscore_binding,
    clippy::unused_async,
    clippy::unused_async_trait_impl
)]
#[zbus::interface(name = "org.mpris.MediaPlayer2.Player")]
impl PlayerInterface {
    // ── Methods ───────────────────────────────────────────────

    async fn play(&self) -> zbus::fdo::Result<()> {
        self.send(ZoneCommand::Play)
    }

    async fn pause(&self) -> zbus::fdo::Result<()> {
        self.send(ZoneCommand::Pause)
    }

    async fn play_pause(&self) -> zbus::fdo::Result<()> {
        let playing = self.state.lock().await.playback == "Playing";
        self.send(if playing {
            ZoneCommand::Pause
        } else {
            ZoneCommand::Play
        })
    }

    async fn stop(&self) -> zbus::fdo::Result<()> {
        self.send(ZoneCommand::Stop)
    }

    async fn next(&self) -> zbus::fdo::Result<()> {
        self.send(ZoneCommand::Next)
    }

    async fn previous(&self) -> zbus::fdo::Result<()> {
        self.send(ZoneCommand::Previous)
    }

    /// Relative seek; the zone clamps it to the track. No effect when the track cannot
    /// seek. `Seeked` follows from the zone's position report, not from here.
    async fn seek(&self, offset: i64) -> zbus::fdo::Result<()> {
        if !self.state.lock().await.seekable {
            return Ok(());
        }
        self.send(ZoneCommand::SeekRelative(offset / 1000))
    }

    /// Absolute seek. Ignored, as the specification says, for another track than the
    /// current one (a stale request), for a position outside the track, or when the
    /// track cannot seek.
    async fn set_position(
        &self,
        track_id: zbus::ObjectPath<'_>,
        position: i64,
    ) -> zbus::fdo::Result<()> {
        let s = self.state.lock().await;
        let valid = s.seekable
            && track_id.as_str() == s.track_id
            && s.track_id != NO_TRACK
            && position >= 0
            && position / 1000 <= s.duration_ms;
        drop(s);
        if !valid {
            return Ok(());
        }
        self.send(ZoneCommand::Seek(position / 1000))
    }

    fn open_uri(&self, #[allow(unused)] uri: &str) {}

    // ── Signals ───────────────────────────────────────────────

    #[zbus(signal)]
    pub async fn seeked(
        emitter: &zbus::object_server::SignalEmitter<'_>,
        position: i64,
    ) -> zbus::Result<()>;

    // ── Properties ────────────────────────────────────────────

    #[zbus(property)]
    async fn playback_status(&self) -> String {
        self.state.lock().await.playback.clone()
    }

    #[zbus(property)]
    async fn metadata(&self) -> HashMap<String, Variant<'static>> {
        let s = self.state.lock().await;
        self.build_metadata(&s)
    }

    #[zbus(property)]
    async fn volume(&self) -> f64 {
        let s = self.state.lock().await;
        if s.muted {
            0.0
        } else {
            f64::from(s.volume) / 100.0
        }
    }

    #[zbus(property)]
    async fn set_volume(&self, vol: f64) -> zbus::fdo::Result<()> {
        if !vol.is_finite() {
            return Err(zbus::fdo::Error::InvalidArgs(format!(
                "Volume must be a number, got {vol}"
            )));
        }
        #[allow(clippy::cast_possible_truncation)]
        let v = ((vol * 100.0).round().clamp(0.0, 100.0)) as i32;
        self.send(ZoneCommand::SetVolume(v))?;
        // zbus announces the property right after the setter: let it carry the new value.
        {
            let mut s = self.state.lock().await;
            s.volume = v;
            s.muted = false;
        }
        Ok(())
    }

    #[zbus(property)]
    async fn position(&self) -> i64 {
        self.state.lock().await.position_ms * 1000 // ms → µs
    }

    #[zbus(property)]
    async fn can_play(&self) -> bool {
        true
    }

    #[zbus(property)]
    async fn can_pause(&self) -> bool {
        true
    }

    #[zbus(property)]
    async fn can_seek(&self) -> bool {
        self.state.lock().await.seekable
    }

    #[zbus(property)]
    async fn can_go_next(&self) -> bool {
        self.state.lock().await.can_next
    }

    #[zbus(property)]
    async fn can_go_previous(&self) -> bool {
        self.state.lock().await.can_prev
    }

    #[zbus(property)]
    async fn shuffle(&self) -> bool {
        self.state.lock().await.shuffle
    }

    #[zbus(property)]
    async fn set_shuffle(&self, val: bool) -> zbus::fdo::Result<()> {
        self.send(ZoneCommand::SetShuffle(val))?;
        self.state.lock().await.shuffle = val;
        Ok(())
    }

    #[zbus(property)]
    async fn loop_status(&self) -> String {
        let s = self.state.lock().await;
        if s.track_repeat {
            "Track".into()
        } else if s.repeat {
            "Playlist".into()
        } else {
            "None".into()
        }
    }

    #[zbus(property)]
    async fn set_loop_status(&self, val: &str) -> zbus::fdo::Result<()> {
        let mode = match val {
            "Track" => snapdog_common::RepeatMode::Track,
            "Playlist" => snapdog_common::RepeatMode::Playlist,
            "None" => snapdog_common::RepeatMode::Off,
            other => {
                return Err(zbus::fdo::Error::InvalidArgs(format!(
                    "LoopStatus must be None, Track or Playlist, got {other:?}"
                )));
            }
        };
        self.send(ZoneCommand::SetRepeat(mode))?;
        {
            let mut s = self.state.lock().await;
            s.repeat = mode == snapdog_common::RepeatMode::Playlist;
            s.track_repeat = mode == snapdog_common::RepeatMode::Track;
        }
        Ok(())
    }

    #[zbus(property)]
    fn rate(&self) -> f64 {
        1.0
    }

    #[zbus(property)]
    fn minimum_rate(&self) -> f64 {
        1.0
    }

    #[zbus(property)]
    fn maximum_rate(&self) -> f64 {
        1.0
    }

    #[zbus(property)]
    fn can_control(&self) -> bool {
        true
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::dbus::testing::{
        call, call_err, fixture, get_bool, get_f64, get_string, next_command, no_command,
        no_seeked, set_property, set_property_err,
    };

    /// A seekable track of 200 s with a proper id, as the notification path sets it.
    async fn load_track(state: &Mutex<PlayerState>) -> String {
        let mut s = state.lock().await;
        s.title = "Title".into();
        s.duration_ms = 200_000;
        s.seekable = true;
        s.track_id = track_object_path(
            1,
            &TrackKey {
                source: "subsonic_playlist",
                playlist_id: Some("subsonic:1"),
                track_index: Some(0),
                title: "Title",
                artist: "",
                album: "",
                duration_ms: 200_000,
            },
        );
        s.track_id.clone()
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn play_pause_plays_unless_playing() {
        let mut f = fixture().await;
        for status in ["Stopped", "Paused"] {
            f.state.lock().await.playback = status.into();
            call(&f.caller, "PlayPause", &()).await;
            assert!(
                matches!(next_command(&mut f.commands).await, ZoneCommand::Play),
                "{status}"
            );
        }
        f.state.lock().await.playback = "Playing".into();
        call(&f.caller, "PlayPause", &()).await;
        assert!(matches!(
            next_command(&mut f.commands).await,
            ZoneCommand::Pause
        ));
    }

    /// Every method has to answer on the runtime thread, so a blocking call added to any of
    /// them shows up here. The property setters have their own test below.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn every_method_replies_and_forwards_its_command() {
        let mut f = fixture().await;
        let track = load_track(&f.state).await;
        call(&f.caller, "Play", &()).await;
        assert!(matches!(
            next_command(&mut f.commands).await,
            ZoneCommand::Play
        ));
        call(&f.caller, "Pause", &()).await;
        assert!(matches!(
            next_command(&mut f.commands).await,
            ZoneCommand::Pause
        ));
        call(&f.caller, "Stop", &()).await;
        assert!(matches!(
            next_command(&mut f.commands).await,
            ZoneCommand::Stop
        ));
        call(&f.caller, "Next", &()).await;
        assert!(matches!(
            next_command(&mut f.commands).await,
            ZoneCommand::Next
        ));
        call(&f.caller, "Previous", &()).await;
        assert!(matches!(
            next_command(&mut f.commands).await,
            ZoneCommand::Previous
        ));
        // MPRIS offsets and positions are microseconds, the zone player takes milliseconds.
        call(&f.caller, "Seek", &(2_500_000_i64,)).await;
        assert!(matches!(
            next_command(&mut f.commands).await,
            ZoneCommand::SeekRelative(2500)
        ));
        let track = zbus::ObjectPath::try_from(track.as_str()).expect("path");
        call(&f.caller, "SetPosition", &(track, 7_000_000_i64)).await;
        assert!(matches!(
            next_command(&mut f.commands).await,
            ZoneCommand::Seek(7000)
        ));
        call(&f.caller, "OpenUri", &("file:///x",)).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn property_setters_reply_and_forward_their_command() {
        let mut f = fixture().await;
        for (value, expected) in [(0.5_f64, 50), (0.0, 0), (1.0, 100), (7.0, 100), (-1.0, 0)] {
            set_property(&f.caller, "Volume", zbus::Variant::from(value)).await;
            let ZoneCommand::SetVolume(v) = next_command(&mut f.commands).await else {
                panic!("expected SetVolume for {value}");
            };
            assert_eq!(v, expected, "{value}");
        }
        for enabled in [true, false] {
            set_property(&f.caller, "Shuffle", zbus::Variant::from(enabled)).await;
            let ZoneCommand::SetShuffle(v) = next_command(&mut f.commands).await else {
                panic!("expected SetShuffle");
            };
            assert_eq!(v, enabled);
        }
        for (status, mode) in [
            ("Track", snapdog_common::RepeatMode::Track),
            ("Playlist", snapdog_common::RepeatMode::Playlist),
            ("None", snapdog_common::RepeatMode::Off),
        ] {
            set_property(&f.caller, "LoopStatus", zbus::Variant::from(status)).await;
            let ZoneCommand::SetRepeat(m) = next_command(&mut f.commands).await else {
                panic!("expected SetRepeat");
            };
            assert_eq!(m, mode, "{status}");
            assert_eq!(
                get_string(&f.caller, "LoopStatus").await,
                status,
                "the cache follows the setter"
            );
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_invalid_volume_or_loop_status_is_refused_and_sends_nothing() {
        let mut f = fixture().await;
        for value in [f64::NAN, f64::INFINITY] {
            let err = set_property_err(&f.caller, "Volume", zbus::Variant::from(value)).await;
            assert!(err.ends_with("InvalidArgs"), "{value}: {err}");
        }
        let err = set_property_err(&f.caller, "LoopStatus", zbus::Variant::from("Forever")).await;
        assert!(err.ends_with("InvalidArgs"), "{err}");
        no_command(&mut f.commands).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn properties_report_the_state() {
        let f = fixture().await;
        {
            let mut s = f.state.lock().await;
            s.playback = "Paused".into();
            s.volume = 40;
            s.repeat = true;
        }
        assert_eq!(get_string(&f.caller, "PlaybackStatus").await, "Paused");
        assert_eq!(get_string(&f.caller, "LoopStatus").await, "Playlist");
        assert!((get_f64(&f.caller, "Volume").await - 0.4).abs() < 1e-9);
        // Muted reads as 0.0, and Track repeat wins over Playlist repeat.
        {
            let mut s = f.state.lock().await;
            s.muted = true;
            s.track_repeat = true;
        }
        assert!(get_f64(&f.caller, "Volume").await.abs() < f64::EPSILON);
        assert_eq!(get_string(&f.caller, "LoopStatus").await, "Track");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn capability_properties_follow_their_own_state_flag() {
        let f = fixture().await;
        for name in ["CanSeek", "CanGoNext", "CanGoPrevious"] {
            assert!(!get_bool(&f.caller, name).await, "{name} by default");
        }
        // One flag at a time, so a swapped or constant getter shows up.
        for (set, expect) in [(0, ["CanSeek"]), (1, ["CanGoNext"]), (2, ["CanGoPrevious"])] {
            {
                let mut s = f.state.lock().await;
                s.seekable = set == 0;
                s.can_next = set == 1;
                s.can_prev = set == 2;
            }
            for name in ["CanSeek", "CanGoNext", "CanGoPrevious"] {
                assert_eq!(
                    get_bool(&f.caller, name).await,
                    expect.contains(&name),
                    "{name} with only {} set",
                    expect[0]
                );
            }
        }
        // Always true, whatever the state.
        for name in ["CanPlay", "CanPause", "CanControl"] {
            assert!(get_bool(&f.caller, name).await, "{name}");
        }
    }

    /// `Seek` is relative and left to the zone, which clamps it. It does not announce a
    /// seek itself: that comes from the zone's position report.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn seek_sends_a_relative_seek_and_no_seeked_signal() {
        let mut f = fixture().await;
        let mut signals = zbus::MessageStream::from(&f.caller);
        load_track(&f.state).await;
        call(&f.caller, "Seek", &(-60_000_000_i64,)).await;
        assert!(matches!(
            next_command(&mut f.commands).await,
            ZoneCommand::SeekRelative(-60_000)
        ));
        no_seeked(&mut signals).await;

        // Not seekable: no effect.
        f.state.lock().await.seekable = false;
        call(&f.caller, "Seek", &(5_000_000_i64,)).await;
        no_command(&mut f.commands).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn set_position_ignores_invalid_and_stale_requests() {
        let mut f = fixture().await;
        let mut signals = zbus::MessageStream::from(&f.caller);
        let current = load_track(&f.state).await;
        let path = |p: &str| zbus::ObjectPath::try_from(p.to_owned()).expect("path");
        for (track, position, why) in [
            (current.as_str(), -5_000_000_i64, "negative"),
            (current.as_str(), 201_000_000, "beyond the end"),
            (
                "/org/mpris/MediaPlayer2/snapdog/zone1/track/told",
                1_000_000,
                "stale id",
            ),
            (NO_TRACK, 1_000_000, "no track"),
        ] {
            call(&f.caller, "SetPosition", &(path(track), position)).await;
            let got =
                tokio::time::timeout(std::time::Duration::from_millis(100), f.commands.recv())
                    .await;
            assert!(got.is_err(), "{why}: sent {got:?}");
        }
        no_seeked(&mut signals).await;

        call(&f.caller, "SetPosition", &(path(&current), 12_000_000_i64)).await;
        assert!(matches!(
            next_command(&mut f.commands).await,
            ZoneCommand::Seek(12_000)
        ));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_full_or_closed_zone_channel_is_an_error_without_blocking() {
        let mut f = fixture().await;
        // The fixture's channel holds 8 commands; nobody drains it here.
        for _ in 0..8 {
            call(&f.caller, "Stop", &()).await;
        }
        let err = call_err(&f.caller, "Play", &()).await;
        assert!(err.ends_with("LimitsExceeded"), "{err}");
        f.commands.close();
        while f.commands.try_recv().is_ok() {}
        let err = call_err(&f.caller, "Play", &()).await;
        assert!(err.ends_with("Failed"), "{err}");
    }

    #[test]
    fn two_tracks_have_different_ids_and_no_track_has_the_no_track_path() {
        let key = |title| TrackKey {
            source: "radio",
            playlist_id: None,
            track_index: None,
            title,
            artist: "",
            album: "",
            duration_ms: 0,
        };
        let a = track_object_path(1, &key("A"));
        let b = track_object_path(1, &key("B"));
        assert_ne!(a, b);
        for id in [&a, &b] {
            assert!(zbus::ObjectPath::try_from(id.as_str()).is_ok(), "{id}");
        }
        assert_eq!(track_object_path(1, &key("")), NO_TRACK);
    }

    #[test]
    fn a_position_jump_is_a_seek_playback_is_not() {
        let mut s = PlayerState {
            playback: "Playing".into(),
            ..PlayerState::default()
        };
        let t0 = std::time::Instant::now();
        assert!(
            !s.report_position(10_000, t0),
            "the first report has nothing to compare"
        );
        let t1 = t0 + std::time::Duration::from_secs(1);
        assert!(!s.report_position(11_000, t1), "one second of playback");
        assert!(
            !s.report_position(12_000, t1),
            "a decoder running a second ahead"
        );
        assert!(s.report_position(60_000, t1), "forward seek");
        assert!(s.report_position(5_000, t1), "backward seek");
        s.playback = "Paused".into();
        let t2 = t1 + std::time::Duration::from_secs(30);
        assert!(
            !s.report_position(5_000, t2),
            "paused: the position stands still"
        );
    }
}
