// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! `org.mpris.MediaPlayer2.Player` interface.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::{Mutex, mpsc};
use zbus::zvariant::Value;

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

    fn build_metadata(&self, s: &PlayerState) -> HashMap<String, Value<'static>> {
        let mut m: HashMap<String, Value<'static>> = HashMap::new();
        let track_id = format!(
            "/org/mpris/MediaPlayer2/snapdog/zone{}/track",
            self.zone_index
        );
        m.insert("mpris:trackid".into(), Value::new(track_id));
        if !s.title.is_empty() {
            m.insert("xesam:title".into(), Value::new(s.title.clone()));
        }
        if !s.artist.is_empty() {
            m.insert("xesam:artist".into(), Value::new(vec![s.artist.clone()]));
        }
        if !s.album.is_empty() {
            m.insert("xesam:album".into(), Value::new(s.album.clone()));
        }
        if let Some(ref url) = s.cover_url {
            let art_url = if url.starts_with("http") {
                url.clone()
            } else {
                format!("{}{url}", self.art_base)
            };
            m.insert("mpris:artUrl".into(), Value::new(art_url));
        }
        if s.duration_ms > 0 {
            m.insert(
                "mpris:length".into(),
                Value::new(s.duration_ms * 1000), // ms → µs
            );
        }
        m
    }
}

#[allow(
    clippy::unused_self,
    clippy::missing_const_for_fn,
    clippy::used_underscore_binding,
    clippy::unused_async
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
        #[allow(unused)] track_id: zbus::zvariant::ObjectPath<'_>,
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
    async fn metadata(&self) -> HashMap<String, Value<'static>> {
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
        match val {
            "Track" => {
                let _ = self.cmd_tx.send(ZoneCommand::SetTrackRepeat(true)).await;
                let _ = self.cmd_tx.send(ZoneCommand::SetRepeat(false)).await;
            }
            "Playlist" => {
                let _ = self.cmd_tx.send(ZoneCommand::SetTrackRepeat(false)).await;
                let _ = self.cmd_tx.send(ZoneCommand::SetRepeat(true)).await;
            }
            _ => {
                let _ = self.cmd_tx.send(ZoneCommand::SetTrackRepeat(false)).await;
                let _ = self.cmd_tx.send(ZoneCommand::SetRepeat(false)).await;
            }
        }
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
