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

/// Served over a peer-to-peer connection, which needs a Unix socket pair.
#[cfg(all(test, unix))]
mod tests {
    use std::os::unix::net::UnixStream;
    use std::time::Duration;

    use zbus::{Guid, connection};

    use super::*;

    const PATH: &str = "/org/mpris/MediaPlayer2";
    const PLAYER: &str = "org.mpris.MediaPlayer2.Player";
    /// A reply that never arrives is the failure these tests exist to catch, so every wait
    /// has a deadline.
    const DEADLINE: Duration = Duration::from_secs(5);

    /// The Player interface served over a peer-to-peer connection (a socketpair), so no bus
    /// daemon is needed, and the calling end of it.
    struct Fixture {
        caller: zbus::Connection,
        /// Keeps the serving end open.
        _server: zbus::Connection,
        state: Arc<Mutex<PlayerState>>,
        commands: mpsc::Receiver<ZoneCommand>,
    }

    async fn fixture() -> Fixture {
        let (server_end, caller_end) = UnixStream::pair().expect("socketpair");
        let (cmd_tx, commands) = mpsc::channel(8);
        let state = Arc::new(Mutex::new(PlayerState::default()));
        let player = PlayerInterface::new(
            1,
            "http://localhost:5555".into(),
            Arc::clone(&state),
            cmd_tx,
        );
        let (server, caller) = tokio::try_join!(
            async {
                connection::Builder::unix_stream(server_end)
                    .server(Guid::generate())
                    .p2p()
                    .serve_at(PATH, player)
                    .build()
                    .await
            },
            connection::Builder::unix_stream(caller_end).p2p().build(),
        )
        .expect("peer-to-peer handshake");
        Fixture {
            caller,
            _server: server,
            state,
            commands,
        }
    }

    async fn call<B>(caller: &zbus::Connection, method: &str, body: &B)
    where
        B: zbus::export::serde::Serialize + zbus::zvariant::DynamicType,
    {
        tokio::time::timeout(
            DEADLINE,
            caller.call_method(None::<&str>, PATH, Some(PLAYER), method, body),
        )
        .await
        .unwrap_or_else(|_| {
            panic!("{method} got no reply within {DEADLINE:?}: the method most likely panicked")
        })
        .unwrap_or_else(|e| panic!("{method} failed: {e}"));
    }

    async fn next_command(commands: &mut mpsc::Receiver<ZoneCommand>) -> ZoneCommand {
        tokio::time::timeout(DEADLINE, commands.recv())
            .await
            .expect("no command was forwarded")
            .expect("command channel closed")
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
    /// them shows up here.
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
}
