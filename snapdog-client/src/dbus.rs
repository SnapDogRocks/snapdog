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

use snapcast_client::{ClientCommand, ClientEvent};

/// Bus name prefix for the client.
const BUS_NAME_PREFIX: &str = "org.mpris.MediaPlayer2.snapdog_client";

/// Highest `.instanceN` suffix tried when the plain name is taken.
const MAX_INSTANCE: u32 = 10;

/// Bus names to request in order: the plain name, then `.instance2` to `.instance10`.
///
/// `dbus/snapdog-client.conf` has to allow sending to every one of these, name by name
/// (see the test `policy_file_is_exactly_what_bus_names_requires`).
fn bus_names() -> impl Iterator<Item = String> {
    std::iter::once(BUS_NAME_PREFIX.to_owned())
        .chain((2..=MAX_INSTANCE).map(|i| format!("{BUS_NAME_PREFIX}.instance{i}")))
}

/// Shared state between the D-Bus interface and the event loop.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, PartialEq, Eq)]
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

    /// Update from Type 14 metadata. The metadata carries the zone's volume and mute; the
    /// MPRIS player is this one speaker, so `Volume` keeps following the client's own volume
    /// (`VolumeChanged` events) and is not taken from here.
    pub fn set_metadata(&mut self, meta: &snapdog_common::TrackMetadata) {
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

/// Which properties an event changed, so their `PropertiesChanged` signals can be sent.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Changed {
    pub volume: bool,
    pub playback_status: bool,
    pub metadata: bool,
    pub shuffle: bool,
    pub loop_status: bool,
    pub can_seek: bool,
    pub can_go_next: bool,
    pub can_go_previous: bool,
}

impl Changed {
    /// The properties that differ between two states. `Position` is not among them: MPRIS
    /// forbids signalling it.
    fn between(before: &ClientDbusState, after: &ClientDbusState) -> Self {
        Self {
            volume: before.mpris_volume().to_bits() != after.mpris_volume().to_bits(),
            playback_status: before.playing != after.playing,
            metadata: (
                &before.title,
                &before.artist,
                &before.album,
                before.duration_ms,
                &before.cover_path,
            ) != (
                &after.title,
                &after.artist,
                &after.album,
                after.duration_ms,
                &after.cover_path,
            ),
            shuffle: before.shuffle != after.shuffle,
            loop_status: before.repeat != after.repeat,
            can_seek: before.seekable != after.seekable,
            can_go_next: before.can_next != after.can_next,
            can_go_previous: before.can_prev != after.can_prev,
        }
    }
}

/// Change `state` with `f` and report which properties it changed.
pub async fn update(state: &SharedDbusState, f: impl FnOnce(&mut ClientDbusState)) -> Changed {
    let mut s = state.lock().await;
    let before = s.clone();
    f(&mut s);
    let changed = Changed::between(&before, &s);
    drop(s);
    changed
}

/// Send `PropertiesChanged` for every property in `changed`. A consumer such as
/// `playerctl --follow` or a desktop widget only learns about changes from the server this way:
/// zbus signals a property by itself only after a setter called over D-Bus.
#[allow(
    clippy::significant_drop_tightening,
    reason = "the generated `*_changed` methods are on the interface, so it is held while they run"
)]
pub async fn announce(iface: &InterfaceRef<PlayerInterface>, changed: Changed) {
    if changed == Changed::default() {
        return;
    }
    let player = iface.get().await;
    let emitter = iface.signal_emitter();
    macro_rules! signal {
        ($flag:expr, $method:ident, $name:literal) => {
            if $flag && let Err(e) = player.$method(emitter).await {
                tracing::debug!(property = $name, error = %e, "PropertiesChanged not sent");
            }
        };
    }
    signal!(changed.volume, volume_changed, "Volume");
    signal!(
        changed.playback_status,
        playback_status_changed,
        "PlaybackStatus"
    );
    signal!(changed.metadata, metadata_changed, "Metadata");
    signal!(changed.shuffle, shuffle_changed, "Shuffle");
    signal!(changed.loop_status, loop_status_changed, "LoopStatus");
    signal!(changed.can_seek, can_seek_changed, "CanSeek");
    signal!(changed.can_go_next, can_go_next_changed, "CanGoNext");
    signal!(
        changed.can_go_previous,
        can_go_previous_changed,
        "CanGoPrevious"
    );
}

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

    /// MPRIS forbids signalling `Position`: consumers extrapolate it from `Rate`.
    #[zbus(property(emits_changed_signal = "false"))]
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

/// Feed a client event into the D-Bus state, the way the event loop in `main` does for
/// every event it receives: a volume change from the server or a Type 14 metadata message.
/// Anything else, and a metadata payload that does not parse, leaves the state alone.
/// Returns the properties that changed, for [`announce`].
pub async fn handle_event(state: &SharedDbusState, event: &ClientEvent) -> Changed {
    match event {
        ClientEvent::VolumeChanged { volume, muted } => {
            update(state, |s| s.set_volume(*volume, *muted)).await
        }
        #[cfg(feature = "custom-protocol")]
        ClientEvent::CustomMessage(msg)
            if msg.type_id == snapdog_common::MSG_TYPE_TRACK_METADATA =>
        {
            match serde_json::from_slice::<snapdog_common::TrackMetadata>(&msg.payload) {
                Ok(meta) => update(state, |s| s.set_metadata(&meta)).await,
                Err(_) => Changed::default(),
            }
        }
        _ => Changed::default(),
    }
}

/// Where the cover art for MPRIS consumers is kept: systemd's `RuntimeDirectory`
/// (`/run/snapdog-client` under the shipped unit, readable by other processes), else
/// `$XDG_RUNTIME_DIR/snapdog-client`, else the temp directory.
pub fn cover_dir() -> std::path::PathBuf {
    if let Some(dir) = std::env::var_os("RUNTIME_DIRECTORY").filter(|d| !d.is_empty()) {
        // systemd may list several directories separated by ':'; the first is ours.
        let first = std::ffi::OsString::from(
            dir.to_string_lossy()
                .split(':')
                .next()
                .unwrap_or_default()
                .to_owned(),
        );
        return first.into();
    }
    std::env::var_os("XDG_RUNTIME_DIR")
        .filter(|d| !d.is_empty())
        .map_or_else(std::env::temp_dir, |d| {
            std::path::Path::new(&d).join("snapdog-client")
        })
}

/// Store a cover as the one cover file of this client instance and return its path.
///
/// The file is replaced atomically (write to a temporary name, then rename), so a consumer
/// never reads half an image and there is never more than one cover per instance. It is
/// readable by everyone, so a session-bus consumer can open it.
///
/// # Errors
///
/// Returns an error if the directory or the file cannot be written.
pub fn store_cover(
    dir: &std::path::Path,
    instance: u32,
    bytes: &[u8],
) -> std::io::Result<std::path::PathBuf> {
    use std::io::Write;
    std::fs::create_dir_all(dir)?;
    let path = dir.join(format!("cover-{instance}"));
    let tmp = dir.join(format!(".cover-{instance}.tmp"));
    {
        let mut file = std::fs::File::create(&tmp)?;
        file.write_all(bytes)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(std::fs::Permissions::from_mode(0o644))?;
        }
    }
    std::fs::rename(&tmp, &path)?;
    Ok(path)
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

/// A full Type 14 metadata message for the tests, with every field set to a value that
/// differs from the `ClientDbusState` default, so a field that is not copied shows up.
/// Checks shared with the server's policy test (well-formed XML, comments).
#[cfg(test)]
#[path = "../../snapdog-common/test-support/policy_xml.rs"]
mod policy_xml;

#[cfg(test)]
fn sample_metadata(playback: &str) -> snapdog_common::TrackMetadata {
    snapdog_common::TrackMetadata {
        playback: playback.to_owned(),
        source: "airplay".to_owned(),
        shuffle: true,
        repeat: snapdog_common::RepeatMode::Playlist,
        title: "Title".to_owned(),
        artist: "Artist".to_owned(),
        album: "Album".to_owned(),
        album_artist: None,
        genre: None,
        year: None,
        track_number: None,
        disc_number: None,
        duration_ms: 240_000,
        position_ms: 61_000,
        seekable: true,
        cover_url: None,
        bitrate_kbps: None,
        content_type: None,
        track_index: None,
        track_count: None,
        playlist: None,
        playlist_name: None,
        playlist_total: None,
        can_playlist_next: false,
        can_playlist_prev: false,
        can_next: true,
        can_prev: true,
        volume: 42,
        muted: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The state the event loop feeds in decides what `PlayPause` does and what the
    // properties report, so these are tested on their own, not only through the bus.

    /// The event loop is inside `main()` and cannot be driven from a test, so check the
    /// source instead: it has to hand every event to `handle_event` before it matches on it,
    /// or the MPRIS2 state never sees a volume or metadata change.
    #[test]
    fn the_event_loop_hands_every_event_to_handle_event() {
        let main = include_str!("main.rs");
        let call = main
            .find("dbus::handle_event(state, &event).await")
            .expect("main.rs no longer calls dbus::handle_event for the events it receives");
        let loop_start = main
            .find("while let Some(event) = events.recv().await")
            .expect("the event loop moved");
        let dispatch = main[loop_start..]
            .find("match event {")
            .expect("the event dispatch moved")
            + loop_start;
        assert!(
            loop_start < call && call < dispatch,
            "handle_event has to run for every event, before the match on it"
        );
    }

    #[test]
    fn set_metadata_maps_playback_to_the_playing_flag() {
        let mut state = ClientDbusState::default();
        assert!(!state.playing);
        state.set_metadata(&sample_metadata("playing"));
        assert!(state.playing);
        for other in ["paused", "stopped", ""] {
            state.set_metadata(&sample_metadata("playing"));
            state.set_metadata(&sample_metadata(other));
            assert!(!state.playing, "{other:?} must not count as playing");
        }
    }

    #[test]
    fn set_metadata_copies_every_field_the_properties_report() {
        let mut state = ClientDbusState::default();
        state.set_volume(40, false);
        state.set_volume(40, true);
        state.set_volume(55, true);
        let volume = (state.volume, state.muted, state.pre_mute_volume);
        state.set_metadata(&sample_metadata("playing"));
        // The metadata carries the zone's volume (42, muted). The MPRIS player is this one
        // speaker, so its own volume, mute and the volume to restore stay untouched.
        assert_eq!((state.volume, state.muted, state.pre_mute_volume), volume);
        assert_eq!(state.title, "Title");
        assert_eq!(state.artist, "Artist");
        assert_eq!(state.album, "Album");
        assert_eq!((state.duration_ms, state.position_ms), (240_000, 61_000));
        assert!(state.seekable && state.can_next && state.can_prev && state.shuffle);
        assert_eq!(state.repeat, snapdog_common::RepeatMode::Playlist);
    }

    #[test]
    fn changed_lists_the_properties_that_differ_and_never_position() {
        let before = ClientDbusState::default();
        let mut after = before.clone();
        assert_eq!(Changed::between(&before, &after), Changed::default());
        after.position_ms = 5_000;
        assert_eq!(Changed::between(&before, &after), Changed::default());
        after.set_metadata(&sample_metadata("playing"));
        let c = Changed::between(&before, &after);
        assert!(c.metadata && c.playback_status && c.shuffle && c.loop_status);
        assert!(c.can_seek && c.can_go_next && c.can_go_previous);
        assert!(!c.volume, "metadata does not carry the client volume");
        let mut cover = after.clone();
        cover.cover_path = Some("/run/snapdog-client/cover-1".into());
        assert!(Changed::between(&after, &cover).metadata);
    }

    #[test]
    fn a_new_cover_replaces_the_old_one_and_is_readable() {
        let dir = std::env::temp_dir().join(format!("snapdog-cover-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let first = store_cover(&dir, 1, b"\xFF\xD8first").unwrap();
        let second = store_cover(&dir, 1, b"\x89PNGsecond").unwrap();
        assert_eq!(first, second, "one fixed file per instance");
        assert_eq!(std::fs::read(&second).unwrap(), b"\x89PNGsecond");
        let files: Vec<_> = std::fs::read_dir(&dir).unwrap().collect();
        assert_eq!(files.len(), 1, "no old covers and no temporary file left");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&second).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o644);
        }
        let other = store_cover(&dir, 2, b"x").unwrap();
        assert_ne!(other, second, "instances do not share a file");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn set_volume_follows_the_server_and_remembers_the_unmuted_volume() {
        let mut state = ClientDbusState::default();
        state.set_volume(30, false);
        assert_eq!((state.volume, state.muted), (30, false));
        assert!((state.mpris_volume() - 0.3).abs() < 1e-9);
        // A mute keeps the volume to restore, and MPRIS reports 0.0 while muted.
        state.set_volume(30, true);
        assert_eq!((state.volume, state.muted), (30, true));
        assert!(state.mpris_volume().abs() < f64::EPSILON);
        assert_eq!(state.pre_mute_volume, 30);
        // A new volume while muted must not overwrite the one to restore.
        state.set_volume(80, true);
        assert_eq!(state.pre_mute_volume, 30);
        state.set_volume(80, false);
        assert_eq!(state.pre_mute_volume, 80);
    }

    /// The policy file with its comments removed, as trimmed, non-empty lines.
    fn policy_lines(policy: &str) -> Vec<String> {
        policy_xml::strip_comments(policy)
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(str::to_owned)
            .collect()
    }

    /// Every line the policy file is allowed to contain, in order. The file grants exactly
    /// this and nothing else.
    fn expected_policy() -> Vec<String> {
        let mut lines: Vec<String> = [
            r#"<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-BUS Bus Configuration 1.0//EN""#,
            r#""http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">"#,
            "<busconfig>",
            r#"<policy context="default">"#,
        ]
        .map(str::to_owned)
        .into();
        lines.push(format!(r#"<allow own_prefix="{BUS_NAME_PREFIX}"/>"#));
        lines.extend(bus_names().map(|n| format!(r#"<allow send_destination="{n}"/>"#)));
        lines.extend(["</policy>", "</busconfig>"].map(str::to_owned));
        lines
    }

    /// The bus policy has to allow sending to every name `start` can end up owning, and
    /// nothing besides.
    ///
    /// It lists them one by one. A `send_destination_prefix` rule would be shorter, but
    /// dbus-broker ignores it ("Unknown attribute", so every non-root caller is denied) and
    /// dbus-daemon 1.12 rejects the whole file.
    ///
    /// The whole file is compared, not only the rules that should be there: an extra
    /// `<allow>` (a wildcard `own`, a `send_type`, a prefix rule), a `<deny>`, a second or
    /// different `<policy>` context or a duplicated line each grant or change something
    /// that nobody reviewed, and each fails here.
    #[test]
    fn policy_file_is_exactly_what_bus_names_requires() {
        let policy = include_str!("../dbus/snapdog-client.conf");
        policy_xml::assert_well_formed(policy);
        let actual = policy_lines(policy);
        let expected = expected_policy();

        let missing: Vec<_> = expected
            .iter()
            .filter(|l| !actual.contains(l))
            .take(5)
            .collect();
        let unexpected: Vec<_> = actual
            .iter()
            .filter(|l| !expected.contains(l))
            .take(5)
            .collect();
        assert!(
            missing.is_empty() && unexpected.is_empty(),
            "dbus/snapdog-client.conf and bus_names() disagree. Missing (first 5): {missing:?}. \
             Not allowed to be there (first 5): {unexpected:?}"
        );
        assert_eq!(
            actual.len(),
            expected.len(),
            "a rule is duplicated, or a line is missing or extra"
        );
        assert_eq!(actual, expected, "same lines, different order or structure");

        // The same facts as named checks, for a readable failure.
        let rules = actual.join("\n");
        assert_eq!(
            rules.matches("<policy").count(),
            1,
            "one policy context only"
        );
        assert!(
            !rules.contains("<deny"),
            "everything not allowed stays denied by default"
        );
        assert!(
            !rules.contains("own=\""),
            "no `own` wildcard or name outside the prefix"
        );
        assert!(
            !rules.contains("send_destination_prefix=\""),
            "send_destination_prefix is not understood by dbus-broker or dbus-daemon 1.12"
        );
        assert!(
            !rules.contains("send_type") && !rules.contains("send_interface"),
            "rules that match on type or interface instead of destination are broader"
        );
        assert!(policy.ends_with('\n'), "the file ends with a newline");
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
        server: zbus::Connection,
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
            server,
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

    async fn next_command(commands: &mut mpsc::Receiver<ClientCommand>) -> ClientCommand {
        tokio::time::timeout(DEADLINE, commands.recv())
            .await
            .expect("no command was forwarded")
            .expect("command channel closed")
    }

    async fn next_control(commands: &mut mpsc::Receiver<ClientCommand>) -> PlaybackControl {
        let ClientCommand::SendCustom(msg) = next_command(commands).await else {
            panic!("expected a custom message");
        };
        assert_eq!(msg.type_id, snapdog_common::MSG_TYPE_PLAYBACK_CONTROL);
        serde_json::from_slice(&msg.payload).expect("payload is a PlaybackControl")
    }

    async fn next_volume(commands: &mut mpsc::Receiver<ClientCommand>) -> (u16, bool) {
        let ClientCommand::SetVolume { volume, muted } = next_command(commands).await else {
            panic!("expected a volume command");
        };
        (volume, muted)
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
    /// them shows up here. The property setters have their own tests below.
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

    const PROPERTIES: &str = "org.freedesktop.DBus.Properties";

    /// `Properties.Get` on the Player interface, with a deadline.
    async fn get_property(caller: &zbus::Connection, name: &str) -> zbus::OwnedVariant {
        let reply = tokio::time::timeout(
            DEADLINE,
            caller.call_method(None::<&str>, PATH, Some(PROPERTIES), "Get", &(PLAYER, name)),
        )
        .await
        .unwrap_or_else(|_| panic!("Get {name} got no reply within {DEADLINE:?}"))
        .unwrap_or_else(|e| panic!("Get {name} failed: {e}"));
        reply.body().deserialize().expect("variant body")
    }

    async fn get_string(caller: &zbus::Connection, name: &str) -> String {
        String::try_from(get_property(caller, name).await).expect("string property")
    }

    async fn get_bool(caller: &zbus::Connection, name: &str) -> bool {
        bool::try_from(get_property(caller, name).await).expect("bool property")
    }

    async fn get_i64(caller: &zbus::Connection, name: &str) -> i64 {
        i64::try_from(get_property(caller, name).await).expect("int64 property")
    }

    async fn get_metadata(
        caller: &zbus::Connection,
    ) -> std::collections::HashMap<String, zbus::OwnedVariant> {
        get_property(caller, "Metadata")
            .await
            .try_into()
            .expect("a{sv}")
    }

    async fn get_f64(caller: &zbus::Connection, name: &str) -> f64 {
        f64::try_from(get_property(caller, name).await).expect("double property")
    }

    /// `Properties.Set` on the Player interface, with a deadline. The setters run on the
    /// runtime thread like the methods do, so a blocking call in one of them shows up as a
    /// missing reply.
    async fn set_property(caller: &zbus::Connection, name: &str, value: zbus::Variant<'_>) {
        tokio::time::timeout(
            DEADLINE,
            caller.call_method(
                None::<&str>,
                PATH,
                Some(PROPERTIES),
                "Set",
                &(PLAYER, name, value),
            ),
        )
        .await
        .unwrap_or_else(|_| {
            panic!("Set {name} got no reply within {DEADLINE:?}: the setter most likely panicked")
        })
        .unwrap_or_else(|e| panic!("Set {name} failed: {e}"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn playback_status_property_follows_the_state() {
        let f = fixture().await;
        assert_eq!(get_string(&f.caller, "PlaybackStatus").await, "Stopped");
        f.state.lock().await.playing = true;
        assert_eq!(get_string(&f.caller, "PlaybackStatus").await, "Playing");
    }

    /// The state comes from the event loop (`set_metadata`, Type 14), never from the bus.
    /// Feed it the way the event loop does and check what the bus then does with it.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn metadata_from_the_event_loop_drives_the_toggle_and_the_properties() {
        let mut f = fixture().await;
        f.state
            .lock()
            .await
            .set_metadata(&sample_metadata("playing"));
        assert_eq!(get_string(&f.caller, "PlaybackStatus").await, "Playing");
        call(&f.caller, "PlayPause", &()).await;
        assert_eq!(next_control(&mut f.commands).await, PlaybackControl::Pause);

        f.state
            .lock()
            .await
            .set_metadata(&sample_metadata("paused"));
        // The client does not report Paused to MPRIS, only Playing or Stopped.
        assert_eq!(get_string(&f.caller, "PlaybackStatus").await, "Stopped");
        call(&f.caller, "PlayPause", &()).await;
        assert_eq!(next_control(&mut f.commands).await, PlaybackControl::Play);

        // The rest of the message reaches the properties. Its zone volume (42 %, muted) does
        // not: `Volume` stays this speaker's own volume (100 % by default).
        assert!((get_f64(&f.caller, "Volume").await - 1.0).abs() < f64::EPSILON);
        assert!(get_bool(&f.caller, "Shuffle").await);
        assert_eq!(get_string(&f.caller, "LoopStatus").await, "Playlist");
        assert!(get_bool(&f.caller, "CanSeek").await);
        assert!(get_bool(&f.caller, "CanGoNext").await);
        assert!(get_bool(&f.caller, "CanGoPrevious").await);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn volume_setter_replies_forwards_and_remembers_the_unmuted_volume() {
        let mut f = fixture().await;
        set_property(&f.caller, "Volume", zbus::Variant::from(0.5_f64)).await;
        assert_eq!(next_volume(&mut f.commands).await, (50, false));
        assert_eq!(get_f64(&f.caller, "Volume").await, 0.5);

        // Mute: the volume to restore stays in the state, the property reads 0.0.
        set_property(&f.caller, "Volume", zbus::Variant::from(0.0_f64)).await;
        assert_eq!(next_volume(&mut f.commands).await, (0, true));
        {
            let s = f.state.lock().await;
            assert_eq!((s.volume, s.muted), (50, true));
        }
        assert!(get_f64(&f.caller, "Volume").await.abs() < f64::EPSILON);

        set_property(&f.caller, "Volume", zbus::Variant::from(0.8_f64)).await;
        assert_eq!(next_volume(&mut f.commands).await, (80, false));
        {
            let s = f.state.lock().await;
            assert_eq!((s.volume, s.muted), (80, false));
        }

        // Out-of-range values are clamped.
        set_property(&f.caller, "Volume", zbus::Variant::from(1.7_f64)).await;
        assert_eq!(next_volume(&mut f.commands).await, (100, false));
        set_property(&f.caller, "Volume", zbus::Variant::from(-1.0_f64)).await;
        assert_eq!(next_volume(&mut f.commands).await, (0, true));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn shuffle_and_loop_status_setters_reply_and_forward() {
        let mut f = fixture().await;
        for enabled in [true, false] {
            set_property(&f.caller, "Shuffle", zbus::Variant::from(enabled)).await;
            assert_eq!(
                next_control(&mut f.commands).await,
                PlaybackControl::Shuffle { enabled }
            );
        }
        for (status, mode) in [
            ("Track", snapdog_common::RepeatMode::Track),
            ("Playlist", snapdog_common::RepeatMode::Playlist),
            ("None", snapdog_common::RepeatMode::Off),
            ("anything else", snapdog_common::RepeatMode::Off),
        ] {
            set_property(&f.caller, "LoopStatus", zbus::Variant::from(status)).await;
            assert_eq!(
                next_control(&mut f.commands).await,
                PlaybackControl::Repeat { mode },
                "{status}"
            );
        }
    }

    fn metadata_event(playback: &str) -> ClientEvent {
        let payload = serde_json::to_vec(&sample_metadata(playback)).expect("serialize");
        ClientEvent::CustomMessage(snapcast_proto::CustomMessage::new(
            snapdog_common::MSG_TYPE_TRACK_METADATA,
            payload,
        ))
    }

    /// `handle_event` is what the event loop in `main` calls for every event it receives.
    /// Drive it with real events and read the result over the bus.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn volume_events_reach_the_volume_property() {
        let f = fixture().await;
        handle_event(
            &f.state,
            &ClientEvent::VolumeChanged {
                volume: 30,
                muted: false,
            },
        )
        .await;
        assert!((get_f64(&f.caller, "Volume").await - 0.3).abs() < 1e-9);
        handle_event(
            &f.state,
            &ClientEvent::VolumeChanged {
                volume: 30,
                muted: true,
            },
        )
        .await;
        assert!(get_f64(&f.caller, "Volume").await.abs() < f64::EPSILON);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn metadata_messages_drive_the_toggle_and_the_properties() {
        let mut f = fixture().await;
        handle_event(&f.state, &metadata_event("playing")).await;
        assert_eq!(get_string(&f.caller, "PlaybackStatus").await, "Playing");
        assert_eq!(get_i64(&f.caller, "Position").await, 61_000_000);
        call(&f.caller, "PlayPause", &()).await;
        assert_eq!(next_control(&mut f.commands).await, PlaybackControl::Pause);

        handle_event(&f.state, &metadata_event("stopped")).await;
        assert_eq!(get_string(&f.caller, "PlaybackStatus").await, "Stopped");
        call(&f.caller, "PlayPause", &()).await;
        assert_eq!(next_control(&mut f.commands).await, PlaybackControl::Play);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn other_events_and_bad_payloads_leave_the_state_alone() {
        let f = fixture().await;
        handle_event(&f.state, &metadata_event("playing")).await;
        let before = format!("{:?}", *f.state.lock().await);

        // The same JSON under another message type, and a payload that is not metadata.
        let payload = serde_json::to_vec(&sample_metadata("stopped")).expect("serialize");
        for event in [
            ClientEvent::CustomMessage(snapcast_proto::CustomMessage::new(
                snapdog_common::MSG_TYPE_COVER_ART,
                payload,
            )),
            ClientEvent::CustomMessage(snapcast_proto::CustomMessage::new(
                snapdog_common::MSG_TYPE_TRACK_METADATA,
                b"not json".to_vec(),
            )),
            ClientEvent::Disconnected {
                reason: "test".into(),
            },
        ] {
            handle_event(&f.state, &event).await;
            assert_eq!(format!("{:?}", *f.state.lock().await), before);
        }
    }

    /// Wait for the next `PropertiesChanged` on the caller's end and return the names of the
    /// properties it carries (changed and invalidated).
    async fn next_properties_changed(stream: &mut zbus::MessageStream) -> Vec<String> {
        use futures_util::StreamExt;
        tokio::time::timeout(DEADLINE, async {
            loop {
                let msg = stream.next().await.expect("stream open").expect("message");
                let header = msg.header();
                if header.member().map(zbus::names::MemberName::as_str) != Some("PropertiesChanged")
                {
                    continue;
                }
                let (iface, changed, invalidated): (
                    String,
                    std::collections::HashMap<String, zbus::OwnedVariant>,
                    Vec<String>,
                ) = msg.body().deserialize().expect("PropertiesChanged body");
                assert_eq!(iface, PLAYER);
                let mut names: Vec<String> = changed.into_keys().chain(invalidated).collect();
                names.sort();
                return names;
            }
        })
        .await
        .expect("no PropertiesChanged within the deadline")
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn server_events_are_announced_with_properties_changed() {
        let f = fixture().await;
        let iface: InterfaceRef<PlayerInterface> = f
            .server
            .object_server()
            .interface(PATH)
            .await
            .expect("player interface");
        let mut signals = zbus::MessageStream::from(&f.caller);

        let changed = handle_event(
            &f.state,
            &ClientEvent::VolumeChanged {
                volume: 30,
                muted: false,
            },
        )
        .await;
        announce(&iface, changed).await;
        assert_eq!(next_properties_changed(&mut signals).await, ["Volume"]);

        let changed = handle_event(&f.state, &metadata_event("playing")).await;
        assert!(
            !changed.volume,
            "the zone volume in the metadata is not the client's"
        );
        announce(&iface, changed).await;
        let expected = [
            "CanGoNext",
            "CanGoPrevious",
            "CanSeek",
            "LoopStatus",
            "Metadata",
            "PlaybackStatus",
            "Shuffle",
        ];
        let mut seen = Vec::new();
        while seen.len() < expected.len() {
            seen.extend(next_properties_changed(&mut signals).await);
        }
        seen.sort();
        assert_eq!(seen, expected);

        // Only the position moved: nothing is signalled. A volume event afterwards is the
        // next signal the caller sees.
        let changed = update(&f.state, |s| s.position_ms += 1_000).await;
        assert_eq!(changed, Changed::default());
        announce(&iface, changed).await;
        let changed = handle_event(
            &f.state,
            &ClientEvent::VolumeChanged {
                volume: 31,
                muted: false,
            },
        )
        .await;
        announce(&iface, changed).await;
        assert_eq!(next_properties_changed(&mut signals).await, ["Volume"]);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn position_property_is_in_microseconds() {
        let f = fixture().await;
        assert_eq!(get_i64(&f.caller, "Position").await, 0);
        f.state.lock().await.position_ms = 1_500;
        assert_eq!(get_i64(&f.caller, "Position").await, 1_500_000);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn metadata_property_reports_track_fields_in_mpris_units() {
        let f = fixture().await;
        // Nothing known yet: only the track id.
        let empty = get_metadata(&f.caller).await;
        assert_eq!(empty.len(), 1, "{:?}", empty.keys().collect::<Vec<_>>());
        assert!(empty.contains_key("mpris:trackid"));

        handle_event(&f.state, &metadata_event("playing")).await;
        f.state.lock().await.cover_path = Some("/tmp/snapdog-cover-1.jpg".into());
        let m = get_metadata(&f.caller).await;
        let string = |key: &str| String::try_from(m[key].try_clone().expect("clone")).expect(key);
        assert_eq!(string("xesam:title"), "Title");
        assert_eq!(string("xesam:album"), "Album");
        assert_eq!(
            Vec::<String>::try_from(m["xesam:artist"].try_clone().expect("clone")).expect("array"),
            ["Artist"]
        );
        // MPRIS lengths are microseconds, the message carries milliseconds.
        assert_eq!(
            i64::try_from(m["mpris:length"].try_clone().expect("clone")).expect("int64"),
            240_000_000
        );
        assert_eq!(string("mpris:artUrl"), "file:///tmp/snapdog-cover-1.jpg");
    }
}
