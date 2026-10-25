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

/// Highest `.instanceN` suffix tried when the plain name is taken.
const MAX_INSTANCE: u32 = 10;

/// Bus names to request in order: the plain name, then `.instance2` to `.instance10`.
///
/// `dbus/snapdog-client.conf` has to allow sending to every one of these, name by name
/// (see the test `policy_file_lists_every_bus_name`).
fn bus_names() -> impl Iterator<Item = String> {
    std::iter::once(BUS_NAME_PREFIX.to_owned())
        .chain((2..=MAX_INSTANCE).map(|i| format!("{BUS_NAME_PREFIX}.instance{i}")))
}

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

// zbus runs every method and property on the tokio runtime that serves the connection.
// Nothing in here may block the thread: `blocking_lock`, `blocking_send`, `block_on` and
// `std::thread::sleep` either panic ("Cannot block the current thread from within a
// runtime") or stall every other call on the connection. Take the state with
// `.lock().await` and send with `try_send` or `.send().await`.
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
    async fn metadata(&self) -> std::collections::HashMap<String, zbus::Variant<'_>> {
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
            zbus::Variant::from(zbus::ObjectPath::try_from("/org/snapdog/track").unwrap()),
        );
        if !title.is_empty() {
            map.insert("xesam:title".into(), zbus::Variant::from(title));
        }
        if !artist.is_empty() {
            map.insert("xesam:artist".into(), zbus::Variant::from(vec![artist]));
        }
        if !album.is_empty() {
            map.insert("xesam:album".into(), zbus::Variant::from(album));
        }
        if duration_us > 0 {
            map.insert("mpris:length".into(), zbus::Variant::from(duration_us));
        }
        if let Some(path) = cover_path {
            map.insert(
                "mpris:artUrl".into(),
                zbus::Variant::from(format!("file://{path}")),
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
    async fn play_pause(&self) {
        // Toggle on the state the event loop last reported. This has to be async: the
        // method runs on the tokio runtime that serves the connection, and
        // `Mutex::blocking_lock` panics there.
        let playing = self.state.lock().await.playing;
        if playing {
            self.send_control(snapdog_common::PlaybackControl::Pause);
        } else {
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
        #[allow(unused)] track_id: zbus::ObjectPath<'_>,
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

    // Take the name only now that the interfaces answer calls to it; retry with an
    // instance suffix if another client holds it.
    let mut acquired = false;
    for name in bus_names() {
        if conn.request_name(name.as_str()).await.is_ok() {
            acquired = true;
            break;
        }
    }
    if !acquired {
        anyhow::bail!("Could not acquire D-Bus bus name");
    }

    let iface_ref: InterfaceRef<PlayerInterface> = conn
        .object_server()
        .interface("/org/mpris/MediaPlayer2")
        .await?;

    tracing::info!("MPRIS2 D-Bus interface registered");

    Ok((conn, state, iface_ref))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Values of the `<allow ATTR="...">` rules in a policy file.
    fn allow_rules(policy: &str, attr: &str) -> Vec<String> {
        let open = format!("<allow {attr}=\"");
        policy
            .lines()
            .filter_map(|line| {
                line.trim()
                    .strip_prefix(open.as_str())?
                    .strip_suffix("\"/>")
                    .map(str::to_owned)
            })
            .collect()
    }

    /// The bus policy has to allow sending to every name `start` can end up owning.
    ///
    /// It lists them one by one. A `send_destination_prefix` rule would be shorter, but
    /// dbus-broker ignores it ("Unknown attribute", so every non-root caller is denied) and
    /// dbus-daemon 1.12 rejects the whole file.
    #[test]
    fn policy_file_lists_every_bus_name() {
        let policy = include_str!("../dbus/snapdog-client.conf");
        let expected: Vec<String> = bus_names().collect();
        let allowed = allow_rules(policy, "send_destination");
        let missing: Vec<_> = expected
            .iter()
            .filter(|n| !allowed.contains(n))
            .take(5)
            .collect();
        let unused: Vec<_> = allowed
            .iter()
            .filter(|n| !expected.contains(n))
            .take(5)
            .collect();
        assert!(
            missing.is_empty() && unused.is_empty(),
            "dbus/snapdog-client.conf and bus_names() disagree. Not in the file (first 5): \
             {missing:?}. In the file but never requested (first 5): {unused:?}"
        );
        assert_eq!(allow_rules(policy, "own_prefix"), [BUS_NAME_PREFIX]);
        assert!(
            !policy.contains("send_destination_prefix=\""),
            "send_destination_prefix is not understood by dbus-broker or dbus-daemon 1.12"
        );
    }
}

/// The interface served over a peer-to-peer connection needs a Unix socket pair.
#[cfg(all(test, unix))]
mod p2p_tests {
    use std::os::unix::net::UnixStream;
    use std::time::Duration;

    use snapdog_common::PlaybackControl;
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
        state: SharedDbusState,
        commands: mpsc::Receiver<ClientCommand>,
    }

    async fn fixture() -> Fixture {
        let (server_end, caller_end) = UnixStream::pair().expect("socketpair");
        let (cmd_tx, commands) = mpsc::channel(8);
        let state: SharedDbusState = Arc::new(Mutex::new(ClientDbusState::default()));
        let player = PlayerInterface {
            state: Arc::clone(&state),
            cmd_tx,
        };
        let (server, caller) = tokio::try_join!(
            async {
                connection::Builder::unix_stream(server_end)
                    .server(Guid::generate())
                    .p2p()
                    .serve_at(PATH, RootInterface)
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

    async fn call<B>(caller: &zbus::Connection, method: &str, body: &B) -> zbus::Message
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
        .unwrap_or_else(|e| panic!("{method} failed: {e}"))
    }

    async fn next_control(commands: &mut mpsc::Receiver<ClientCommand>) -> PlaybackControl {
        let cmd = tokio::time::timeout(DEADLINE, commands.recv())
            .await
            .expect("no command was forwarded")
            .expect("command channel closed");
        let ClientCommand::SendCustom(msg) = cmd else {
            panic!("expected a custom message");
        };
        assert_eq!(msg.type_id, snapdog_common::MSG_TYPE_PLAYBACK_CONTROL);
        serde_json::from_slice(&msg.payload).expect("payload is a PlaybackControl")
    }

    // The old `play_pause` took the state lock with `blocking_lock`, which panics inside
    // the tokio runtime that zbus runs the method on: the caller never got a reply.

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn play_pause_replies_and_plays_when_not_playing() {
        let mut f = fixture().await;
        f.state.lock().await.playing = false;
        call(&f.caller, "PlayPause", &()).await;
        assert_eq!(next_control(&mut f.commands).await, PlaybackControl::Play);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn play_pause_replies_and_pauses_when_playing() {
        let mut f = fixture().await;
        f.state.lock().await.playing = true;
        call(&f.caller, "PlayPause", &()).await;
        assert_eq!(next_control(&mut f.commands).await, PlaybackControl::Pause);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn play_pause_follows_the_state_between_calls() {
        let mut f = fixture().await;
        call(&f.caller, "PlayPause", &()).await;
        assert_eq!(next_control(&mut f.commands).await, PlaybackControl::Play);
        f.state.lock().await.playing = true;
        call(&f.caller, "PlayPause", &()).await;
        assert_eq!(next_control(&mut f.commands).await, PlaybackControl::Pause);
        f.state.lock().await.playing = false;
        call(&f.caller, "PlayPause", &()).await;
        assert_eq!(next_control(&mut f.commands).await, PlaybackControl::Play);
    }

    /// Every method has to answer on the runtime thread, so a blocking call added to any of
    /// them shows up here.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn every_method_replies_and_forwards_its_command() {
        let mut f = fixture().await;
        for (method, expected) in [
            ("Play", PlaybackControl::Play),
            ("Pause", PlaybackControl::Pause),
            ("Stop", PlaybackControl::Stop),
            ("Next", PlaybackControl::Next),
            ("Previous", PlaybackControl::Previous),
        ] {
            call(&f.caller, method, &()).await;
            assert_eq!(next_control(&mut f.commands).await, expected, "{method}");
        }
        // MPRIS offsets are microseconds, the control message carries milliseconds.
        call(&f.caller, "Seek", &(2_500_000_i64,)).await;
        assert_eq!(
            next_control(&mut f.commands).await,
            PlaybackControl::Seek {
                position_ms: None,
                offset_ms: Some(2500),
            }
        );
        // Accepted and ignored, but the caller still gets its reply.
        let track = zbus::ObjectPath::try_from("/org/snapdog/track").expect("path");
        call(&f.caller, "SetPosition", &(track, 1_000_000_i64)).await;
        call(&f.caller, "OpenUri", &("file:///x",)).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn playback_status_property_follows_the_state() {
        let f = fixture().await;
        let get = |name: &'static str| {
            let caller = f.caller.clone();
            async move {
                let reply = tokio::time::timeout(
                    DEADLINE,
                    caller.call_method(
                        None::<&str>,
                        PATH,
                        Some("org.freedesktop.DBus.Properties"),
                        "Get",
                        &(PLAYER, name),
                    ),
                )
                .await
                .expect("Properties.Get got no reply")
                .expect("Properties.Get failed");
                let value: zbus::OwnedVariant = reply.body().deserialize().expect("body");
                String::try_from(value).expect("string property")
            }
        };
        assert_eq!(get("PlaybackStatus").await, "Stopped");
        f.state.lock().await.playing = true;
        assert_eq!(get("PlaybackStatus").await, "Playing");
    }
}
