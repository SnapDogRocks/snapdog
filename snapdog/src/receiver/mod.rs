// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! Receiver provider abstraction for passive audio sources.
//!
//! A [`ReceiverProvider`] is a network-advertised audio sink that external apps
//! (iPhone, Spotify, etc.) can stream to. Each zone spawns its own set of receivers.
//!
//! All receivers produce the same output:
//! - **F32 interleaved PCM** via [`AudioSender`] (any sample rate, any channel count)
//! - **Events** via [`ReceiverEventTx`] (metadata, cover art, progress, volume, remote control)
//!
//! The zone player resamples F32 audio to the target rate and converts to S16LE
//! at the output boundary.

pub mod airplay;
#[cfg(feature = "spotify")]
pub mod spotify;

use std::sync::Arc;

use anyhow::Result;
use tokio::sync::mpsc;

// ── Audio channel (F32 interleaved) ───────────────────────────

/// F32 interleaved PCM samples.
pub type AudioSender = mpsc::Sender<Vec<f32>>;
/// F32 interleaved PCM samples.
pub type AudioReceiver = mpsc::Receiver<Vec<f32>>;

/// Create a bounded F32 audio channel pair with the given buffer capacity.
pub fn audio_channel(buffer: usize) -> (AudioSender, AudioReceiver) {
    mpsc::channel(buffer)
}

// ── Audio format ──────────────────────────────────────────────

/// Audio format descriptor reported by a receiver session.
#[derive(Debug, Clone, Copy)]
pub struct AudioFormat {
    /// Sample rate in Hz (e.g., 44100, 48000).
    pub sample_rate: u32,
    /// Number of audio channels (e.g., 2 for stereo).
    pub channels: u16,
}

// ── Events (receiver → zone player) ──────────────────────────

/// Events emitted by a receiver to the zone player.
pub enum ReceiverEvent {
    /// A new audio session started with the given format.
    SessionStarted {
        /// Audio format of the session.
        format: AudioFormat,
    },
    /// The audio session ended (client disconnected).
    SessionEnded,
    /// Track metadata changed.
    Metadata {
        /// Track title.
        title: String,
        /// Track artist.
        artist: String,
        /// Track album.
        album: String,
    },
    /// Cover art received (JPEG or PNG bytes).
    CoverArt {
        /// Raw image bytes.
        bytes: Vec<u8>,
    },
    /// Playback progress update.
    Progress {
        /// Current position in milliseconds.
        position_ms: u64,
        /// Total duration in milliseconds.
        duration_ms: u64,
    },
    /// Volume change (0–100).
    Volume {
        /// Volume percentage.
        percent: i32,
    },
    /// A remote control interface became available.
    RemoteAvailable {
        /// Remote control handle.
        remote: Arc<dyn RemoteControl>,
    },
}

/// Channel sender for receiver events (receiver → zone player).
pub type ReceiverEventTx = mpsc::Sender<ReceiverEvent>;
/// Channel receiver for receiver events (receiver → zone player).
pub type ReceiverEventRx = mpsc::Receiver<ReceiverEvent>;

// ── Remote control (zone player → source device) ─────────────

/// Command sent from `SnapDog` to the source device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemoteCommand {
    /// Resume playback.
    Play,
    /// Pause playback.
    Pause,
    /// Skip to the next track.
    NextTrack,
    /// Skip to the previous track.
    PreviousTrack,
    /// Stop playback.
    Stop,
    /// Set the source device volume (0–100).
    SetVolume(u8),
    /// Toggle shuffle mode on the source device.
    ToggleShuffle,
    /// Toggle repeat mode on the source device.
    ToggleRepeat,
}

/// Trait for controlling the source device.
///
/// Delivered via [`ReceiverEvent::RemoteAvailable`] when a client connects.
/// Implementations bridge to protocol-specific control channels
/// (DACP for `AirPlay` 1, `MediaRemote` for `AirPlay` 2, Spotify Connect API, etc.).
pub trait RemoteControl: Send + Sync {
    /// Send a playback command to the source device.
    ///
    /// # Errors
    ///
    /// Returns an error if the command cannot be delivered.
    fn send_command(&self, cmd: RemoteCommand) -> Result<()>;
}

use std::future::Future;

// ── Provider trait ────────────────────────────────────────────

/// A passive audio receiver that advertises itself on the network.
///
/// Each zone spawns one instance per provider type. The provider writes
/// F32 PCM audio and events to the channels supplied at startup.
pub trait ReceiverProvider: Send {
    /// Human-readable provider name (e.g., "`AirPlay`", "Spotify Connect").
    fn name(&self) -> &'static str;

    /// Start the receiver. Audio and events flow to the provided channels.
    fn start(
        &mut self,
        audio_tx: AudioSender,
        event_tx: ReceiverEventTx,
    ) -> impl Future<Output = Result<()>> + Send;

    /// Stop the receiver and release network resources.
    fn stop(&mut self) -> impl Future<Output = ()> + Send;

    /// Whether the receiver is currently running.
    fn is_running(&self) -> bool;
}
