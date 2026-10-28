// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! `org.mpris.MediaPlayer2.Player` interface.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::{Mutex, mpsc};
use zbus::Variant;

use crate::player::ZoneCommand;

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

/// MPRIS2 Player interface implementation.
pub struct PlayerInterface {
    pub zone_index: usize,
    pub art_base: String,
    pub state: Arc<Mutex<PlayerState>>,
    cmd_tx: mpsc::Sender<ZoneCommand>,
}

impl PlayerInterface {
    pub const fn new(
        zone_index: usize,
        art_base: String,
        state: Arc<Mutex<PlayerState>>,
        cmd_tx: mpsc::Sender<ZoneCommand>,
    ) -> Self {
        Self {
            zone_index,
            art_base,
            state,
            cmd_tx,
        }
    }

    fn build_metadata(&self, s: &PlayerState) -> HashMap<String, Variant<'static>> {
        let mut m: HashMap<String, Variant<'static>> = HashMap::new();
        let track_id = format!(
            "/org/mpris/MediaPlayer2/snapdog/zone{}/track",
            self.zone_index
        );
        m.insert("mpris:trackid".into(), Variant::new(track_id));
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

    async fn play(&self) {
        let _ = self.cmd_tx.send(ZoneCommand::Play).await;
    }

    async fn pause(&self) {
        let _ = self.cmd_tx.send(ZoneCommand::Pause).await;
    }

    async fn play_pause(&self) {
        let s = self.state.lock().await;
        let cmd = if s.playback == "Playing" {
            ZoneCommand::Pause
        } else {
            ZoneCommand::Play
        };
        drop(s);
        let _ = self.cmd_tx.send(cmd).await;
    }

    async fn stop(&self) {
        let _ = self.cmd_tx.send(ZoneCommand::Stop).await;
    }

    async fn next(&self) {
        let _ = self.cmd_tx.send(ZoneCommand::Next).await;
    }

    async fn previous(&self) {
        let _ = self.cmd_tx.send(ZoneCommand::Previous).await;
    }

    async fn seek(
        &self,
        #[zbus(signal_emitter)] emitter: zbus::object_server::SignalEmitter<'_>,
        offset: i64,
    ) {
        let mut s = self.state.lock().await;
        let new_pos_ms = (s.position_ms + offset / 1000).max(0);
        s.position_ms = new_pos_ms;
        drop(s);
        let _ = self.cmd_tx.send(ZoneCommand::Seek(new_pos_ms)).await;
        let _ = Self::seeked(&emitter, new_pos_ms * 1000).await;
    }

    async fn set_position(
        &self,
        #[zbus(signal_emitter)] emitter: zbus::object_server::SignalEmitter<'_>,
        #[allow(unused)] track_id: zbus::ObjectPath<'_>,
        position: i64,
    ) {
        let pos_ms = position / 1000;
        {
            let mut s = self.state.lock().await;
            s.position_ms = pos_ms;
        }
        let _ = self.cmd_tx.send(ZoneCommand::Seek(pos_ms)).await;
        let _ = Self::seeked(&emitter, position).await;
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
    async fn set_volume(&self, vol: f64) {
        #[allow(clippy::cast_possible_truncation)]
        let v = (vol * 100.0).round() as i32;
        let _ = self
            .cmd_tx
            .send(ZoneCommand::SetVolume(v.clamp(0, 100)))
            .await;
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
    async fn set_shuffle(&self, val: bool) {
        let _ = self.cmd_tx.send(ZoneCommand::SetShuffle(val)).await;
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
    async fn set_loop_status(&self, val: &str) {
        let mode = match val {
            "Track" => snapdog_common::RepeatMode::Track,
            "Playlist" => snapdog_common::RepeatMode::Playlist,
            _ => snapdog_common::RepeatMode::Off,
        };
        let _ = self.cmd_tx.send(ZoneCommand::SetRepeat(mode)).await;
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
        call, fixture, get_bool, get_f64, get_i64, get_string, next_command, next_seeked,
        set_property,
    };

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
            ZoneCommand::Seek(2500)
        ));
        let track = zbus::ObjectPath::try_from("/org/snapdog/track").expect("path");
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
        }
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

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn seek_is_relative_in_milliseconds_clamped_at_zero_and_emits_seeked() {
        let mut f = fixture().await;
        let mut signals = zbus::MessageStream::from(&f.caller);
        f.state.lock().await.position_ms = 5_000;

        call(&f.caller, "Seek", &(2_500_000_i64,)).await;
        assert!(matches!(
            next_command(&mut f.commands).await,
            ZoneCommand::Seek(7_500)
        ));
        // The signal carries microseconds, like the property.
        assert_eq!(next_seeked(&mut signals).await, 7_500_000);
        assert_eq!(get_i64(&f.caller, "Position").await, 7_500_000);

        // Back past the start: the position stops at zero.
        call(&f.caller, "Seek", &(-60_000_000_i64,)).await;
        assert!(matches!(
            next_command(&mut f.commands).await,
            ZoneCommand::Seek(0)
        ));
        assert_eq!(next_seeked(&mut signals).await, 0);
        assert_eq!(get_i64(&f.caller, "Position").await, 0);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn set_position_is_absolute_and_emits_seeked_in_microseconds() {
        let mut f = fixture().await;
        let mut signals = zbus::MessageStream::from(&f.caller);
        f.state.lock().await.position_ms = 5_000;
        let track = zbus::ObjectPath::try_from("/org/snapdog/track").expect("path");
        call(&f.caller, "SetPosition", &(track, 12_000_000_i64)).await;
        assert!(matches!(
            next_command(&mut f.commands).await,
            ZoneCommand::Seek(12_000)
        ));
        assert_eq!(next_seeked(&mut signals).await, 12_000_000);
        assert_eq!(get_i64(&f.caller, "Position").await, 12_000_000);
    }
}
