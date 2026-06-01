// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! MPRIS2 D-Bus interface for snapdog-client.
//!
//! Exposes Volume (read/write) and `PlaybackStatus` (read-only).
//! Mute maps to Volume 0.0; unmute restores the previous value.
#![allow(
    clippy::unused_self,
    clippy::needless_pass_by_value,
    clippy::missing_const_for_fn,
    clippy::unnecessary_wraps,
    clippy::str_to_string,
    clippy::needless_lifetimes,
    clippy::unused_async,
    clippy::unnecessary_literal_bound
)]

use std::sync::Arc;

use anyhow::Result;
use tokio::sync::{Mutex, mpsc};
use zbus::object_server::InterfaceRef;

use snapcast_client::ClientCommand;

/// Bus name prefix for the client.
const BUS_NAME_PREFIX: &str = "org.mpris.MediaPlayer2.snapdog_client";

/// Shared state between the D-Bus interface and the event loop.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug)]
pub struct ClientDbusState {
    pub volume: u16,
    pub muted: bool,
    pub playing: bool,
    /// Remembered volume before mute (for unmute restore).
    pre_mute_volume: u16,
    // Metadata from Type 14
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_ms: i64,
    pub position_ms: i64,
    pub seekable: bool,
    pub can_next: bool,
    pub can_prev: bool,
    pub shuffle: bool,
    pub repeat: snapdog_common::RepeatMode,
    pub cover_path: Option<String>,
}

impl Default for ClientDbusState {
    fn default() -> Self {
        Self {
            volume: 100,
            muted: false,
            playing: false,
            pre_mute_volume: 100,
            title: String::new(),
            artist: String::new(),
            album: String::new(),
            duration_ms: 0,
            position_ms: 0,
            seekable: false,
            can_next: false,
            can_prev: false,
            shuffle: false,
            repeat: snapdog_common::RepeatMode::Off,
            cover_path: None,
        }
    }
}

impl ClientDbusState {
    /// Update volume/mute from server event.
    pub fn set_volume(&mut self, volume: u16, muted: bool) {
        self.volume = volume;
        self.muted = muted;
        if !muted {
            self.pre_mute_volume = volume;
        }
    }

    /// Update from Type 14 metadata.
    pub fn set_metadata(&mut self, meta: &snapdog_common::TrackMetadata) {
        self.volume = meta.volume as u16;
        self.muted = meta.muted;
        self.playing = meta.playback == "playing";
        self.title.clone_from(&meta.title);
        self.artist.clone_from(&meta.artist);
        self.album.clone_from(&meta.album);
        self.duration_ms = meta.duration_ms;
        self.position_ms = meta.position_ms;
        self.seekable = meta.seekable;
        self.can_next = meta.can_next;
        self.can_prev = meta.can_prev;
        self.shuffle = meta.shuffle;
        self.repeat = meta.repeat;
    }

    /// MPRIS2 volume: 0.0 when muted, otherwise 0.0–1.0.
    pub fn mpris_volume(&self) -> f64 {
        if self.muted {
            0.0
        } else {
            f64::from(self.volume) / 100.0
        }
    }
}

/// Shared state handle.
pub type SharedDbusState = Arc<Mutex<ClientDbusState>>;

/// MPRIS2 Root interface.
pub struct RootInterface;

#[zbus::interface(name = "org.mpris.MediaPlayer2")]
impl RootInterface {
    #[zbus(property)]
    fn identity(&self) -> &str {
        "SnapDog Client"
    }

    #[zbus(property)]
    fn can_quit(&self) -> bool {
        false
    }

    #[zbus(property)]
    fn can_raise(&self) -> bool {
        false
    }

    #[zbus(property)]
    fn has_track_list(&self) -> bool {
        false
    }

    #[zbus(property)]
    fn supported_uri_schemes(&self) -> Vec<String> {
        vec![]
    }

    #[zbus(property)]
    fn supported_mime_types(&self) -> Vec<String> {
        vec![]
    }

    fn quit(&self) {}
    fn raise(&self) {}
}

/// MPRIS2 Player interface.
pub struct PlayerInterface {
    state: SharedDbusState,
    cmd_tx: mpsc::Sender<ClientCommand>,
}

impl PlayerInterface {
    fn send_control(&self, ctrl: snapdog_common::PlaybackControl) {
        if let Ok(payload) = serde_json::to_vec(&ctrl) {
            let _ = self.cmd_tx.try_send(ClientCommand::SendCustom(
                snapcast_proto::CustomMessage::new(
                    snapdog_common::MSG_TYPE_PLAYBACK_CONTROL,
                    payload,
                ),
            ));
        }
    }
}

#[zbus::interface(name = "org.mpris.MediaPlayer2.Player")]
impl PlayerInterface {
    #[zbus(property)]
    async fn playback_status(&self) -> String {
        if self.state.lock().await.playing {
            "Playing".into()
        } else {
            "Stopped".into()
        }
    }

    #[zbus(property)]
    async fn volume(&self) -> f64 {
        self.state.lock().await.mpris_volume()
    }

    #[zbus(property)]
    async fn set_volume(&self, vol: f64) {
        let percent = (vol.clamp(0.0, 1.0) * 100.0) as u16;
        let muted = percent == 0;
        {
            let mut s = self.state.lock().await;
            if muted && !s.muted {
                s.pre_mute_volume = s.volume;
            }
            s.volume = if muted { s.pre_mute_volume } else { percent };
            s.muted = muted;
        }
        let _ = self
            .cmd_tx
            .send(ClientCommand::SetVolume {
                volume: percent,
                muted,
            })
            .await;
    }

    #[zbus(property)]
    fn can_control(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn can_play(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn can_pause(&self) -> bool {
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
    async fn set_shuffle(&self, enabled: bool) {
        self.send_control(snapdog_common::PlaybackControl::Shuffle { enabled });
    }

    #[zbus(property)]
    async fn loop_status(&self) -> String {
        let repeat = self.state.lock().await.repeat;
        match repeat {
            snapdog_common::RepeatMode::Off => "None".into(),
            snapdog_common::RepeatMode::Track => "Track".into(),
            snapdog_common::RepeatMode::Playlist => "Playlist".into(),
        }
    }

    #[zbus(property)]
    async fn set_loop_status(&self, status: &str) {
        let mode = match status {
            "Track" => snapdog_common::RepeatMode::Track,
            "Playlist" => snapdog_common::RepeatMode::Playlist,
            _ => snapdog_common::RepeatMode::Off,
        };
        self.send_control(snapdog_common::PlaybackControl::Repeat { mode });
    }

    #[zbus(property)]
    async fn metadata(&self) -> std::collections::HashMap<String, zbus::zvariant::Value<'_>> {
        let (title, artist, album, duration_us, cover_path) = {
            let s = self.state.lock().await;
            (
                s.title.clone(),
                s.artist.clone(),
                s.album.clone(),
                s.duration_ms * 1000,
                s.cover_path.clone(),
            )
        };
        let mut map = std::collections::HashMap::new();
        map.insert(
            "mpris:trackid".into(),
            zbus::zvariant::Value::from(
                zbus::zvariant::ObjectPath::try_from("/org/snapdog/track").unwrap(),
            ),
        );
        if !title.is_empty() {
            map.insert("xesam:title".into(), zbus::zvariant::Value::from(title));
        }
        if !artist.is_empty() {
            map.insert(
                "xesam:artist".into(),
                zbus::zvariant::Value::from(vec![artist]),
            );
        }
        if !album.is_empty() {
            map.insert("xesam:album".into(), zbus::zvariant::Value::from(album));
        }
        if duration_us > 0 {
            map.insert(
                "mpris:length".into(),
                zbus::zvariant::Value::from(duration_us),
            );
        }
        if let Some(path) = cover_path {
            map.insert(
                "mpris:artUrl".into(),
                zbus::zvariant::Value::from(format!("file://{path}")),
            );
        }
        map
    }

    #[zbus(property)]
    async fn position(&self) -> i64 {
        self.state.lock().await.position_ms * 1000 // µs
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

    fn play(&self) {
        self.send_control(snapdog_common::PlaybackControl::Play);
    }
    fn pause(&self) {
        self.send_control(snapdog_common::PlaybackControl::Pause);
    }
    fn play_pause(&self) {
        // Toggle based on current state
        let state = self.state.blocking_lock();
        if state.playing {
            drop(state);
            self.send_control(snapdog_common::PlaybackControl::Pause);
        } else {
            drop(state);
            self.send_control(snapdog_common::PlaybackControl::Play);
        }
    }
    fn stop(&self) {
        self.send_control(snapdog_common::PlaybackControl::Stop);
    }
    fn next(&self) {
        self.send_control(snapdog_common::PlaybackControl::Next);
    }
    fn previous(&self) {
        self.send_control(snapdog_common::PlaybackControl::Previous);
    }
    fn seek(&self, offset: i64) {
        // MPRIS2 offset is in microseconds, convert to milliseconds
        let offset_ms = offset / 1000;
        self.send_control(snapdog_common::PlaybackControl::Seek {
            position_ms: None,
            offset_ms: Some(offset_ms),
        });
    }
    fn set_position(
        &self,
        #[allow(unused)] track_id: zbus::zvariant::ObjectPath<'_>,
        #[allow(unused)] position: i64,
    ) {
    }
    fn open_uri(&self, #[allow(unused)] uri: &str) {}
}

/// Start the MPRIS2 D-Bus interface. Returns the shared state for the event loop to update.
///
/// # Errors
///
/// Returns an error if D-Bus connection fails.
pub async fn start(
    cmd_tx: mpsc::Sender<ClientCommand>,
) -> Result<(
    zbus::Connection,
    SharedDbusState,
    InterfaceRef<PlayerInterface>,
)> {
    let conn = if std::env::var("DBUS_SESSION_BUS_ADDRESS").is_ok() {
        zbus::Connection::session().await?
    } else {
        zbus::Connection::system().await?
    };

    // Acquire bus name with retry
    let base = BUS_NAME_PREFIX;
    let acquired = if conn.request_name(base).await.is_ok() {
        true
    } else {
        let mut ok = false;
        for i in 2..=10 {
            let name = format!("{base}.instance{i}");
            if conn.request_name(name.as_str()).await.is_ok() {
                ok = true;
                break;
            }
        }
        ok
    };
    if !acquired {
        anyhow::bail!("Could not acquire D-Bus bus name");
    }

    let state: SharedDbusState = Arc::new(Mutex::new(ClientDbusState::default()));

    let root = RootInterface;
    let player = PlayerInterface {
        state: Arc::clone(&state),
        cmd_tx,
    };

    conn.object_server()
        .at("/org/mpris/MediaPlayer2", root)
        .await?;
    conn.object_server()
        .at("/org/mpris/MediaPlayer2", player)
        .await?;

    let iface_ref: InterfaceRef<PlayerInterface> = conn
        .object_server()
        .interface("/org/mpris/MediaPlayer2")
        .await?;

    tracing::info!("MPRIS2 D-Bus interface registered");

    Ok((conn, state, iface_ref))
}
