//! Audio output — cpal callback reads from Stream directly.

use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use snapcast_client::AudioFrame;
use snapcast_client::connection::now_usec;
use snapcast_client::stream::Stream;
use snapcast_client::time_provider::TimeProvider;
use tokio::sync::mpsc;

use crate::eq::ZoneEq;

/// Maximum MIDI CC value (7-bit).
const MIDI_CC_MAX: u8 = 127;
/// Polling interval while waiting for audio format from the stream.
const FORMAT_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(100);
/// Amplitude below which audio is considered silence (for fade-in detection).
const SILENCE_THRESHOLD: f32 = 1e-6;

/// Shared EQ processor, updated from the event loop, read from the audio thread.
pub type SharedEq = Arc<Mutex<ZoneEq>>;

/// Shared fade state for crossfade on zone switch.
///
/// Lock-free (atomics only) for safe use in the real-time audio callback.
pub struct FadeState {
    /// Total fade duration in samples (set when fade-out is triggered).
    fade_samples: AtomicU32,
    /// Remaining samples in the current fade (counts down).
    remaining: AtomicU32,
    /// true = fading out, false = fading in (or idle).
    fading_out: AtomicBool,
    /// true = fully faded out, waiting for new stream to trigger fade-in.
    faded_out: AtomicBool,
}

impl FadeState {
    /// Create idle fade state.
    pub const fn new() -> Self {
        Self {
            fade_samples: AtomicU32::new(0),
            remaining: AtomicU32::new(0),
            fading_out: AtomicBool::new(false),
            faded_out: AtomicBool::new(false),
        }
    }

    /// Trigger a fade-out over the given duration at the given sample rate.
    #[allow(dead_code)]
    pub fn trigger_fade_out(&self, duration_ms: u16, sample_rate: u32) {
        let samples = (u64::from(sample_rate) * u64::from(duration_ms) / 1000) as u32;
        self.fade_samples.store(samples, Ordering::Relaxed);
        self.remaining.store(samples, Ordering::Relaxed);
        self.fading_out.store(true, Ordering::Release);
        self.faded_out.store(false, Ordering::Relaxed);
    }

    /// Apply fade gain to a buffer. Called from the audio callback.
    /// Returns the gain applied to the last sample (for diagnostics).
    pub fn process(&self, data: &mut [f32], channels: usize) {
        if self.faded_out.load(Ordering::Relaxed) {
            // Fully faded out — check if new audio arrived (non-silent)
            let has_audio = data.iter().any(|&s| s.abs() > SILENCE_THRESHOLD);
            if has_audio {
                // New stream detected — start fade-in
                let total = self.fade_samples.load(Ordering::Relaxed);
                self.remaining.store(total, Ordering::Relaxed);
                self.fading_out.store(false, Ordering::Release);
                self.faded_out.store(false, Ordering::Relaxed);
            } else {
                data.fill(0.0);
                return;
            }
        }

        let total = self.fade_samples.load(Ordering::Relaxed);
        if total == 0 {
            return;
        }

        let fading_out = self.fading_out.load(Ordering::Acquire);
        let mut remaining = self.remaining.load(Ordering::Relaxed);

        if remaining == 0 && fading_out {
            // Fade-out complete
            self.faded_out.store(true, Ordering::Release);
            data.fill(0.0);
            return;
        }
        if remaining == 0 && !fading_out {
            // Fade-in complete — reset state
            self.fade_samples.store(0, Ordering::Relaxed);
            return;
        }

        let num_frames = data.len() / channels;
        for frame in 0..num_frames {
            let gain = snapdog_common::fade_gain(remaining, total, fading_out);
            for ch in 0..channels {
                data[frame * channels + ch] *= gain;
            }
            remaining = remaining.saturating_sub(1);
        }
        self.remaining.store(remaining, Ordering::Relaxed);
    }
}

/// Shared volume state, updated from the event loop, read from the audio thread.
pub struct VolumeState {
    pub percent: AtomicU8,
    pub muted: AtomicBool,
}

impl VolumeState {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            percent: AtomicU8::new(100),
            muted: AtomicBool::new(false),
        })
    }

    pub fn gain(&self) -> f32 {
        if self.muted.load(Ordering::Relaxed) {
            return 0.0;
        }
        snapdog_common::perceptual_volume(self.percent.load(Ordering::Relaxed))
    }
}

/// Mixer implementation dispatched by mode.
pub enum Mixer {
    /// PCM amplitude scaling in the audio callback.
    Software(Arc<VolumeState>),
    /// ALSA hardware mixer control (Linux only).
    #[cfg(target_os = "linux")]
    Hardware {
        control: String,
        volume: Arc<VolumeState>,
    },
    /// MIDI CC volume control.
    Midi {
        conn: Mutex<midir::MidiOutputConnection>,
        channel: u8,
        cc: u8,
    },
    /// No volume control.
    None,
}

impl Mixer {
    pub fn from_cli(raw: &str, volume: Arc<VolumeState>) -> Self {
        let (mode, param) = raw.split_once(':').unwrap_or((raw, ""));
        match mode {
            "software" | "" => Self::Software(volume),
            #[cfg(target_os = "linux")]
            "hardware" => {
                let control = if param.is_empty() {
                    // Auto-detect: try Master, then first available control
                    detect_alsa_control().unwrap_or_else(|| "Master".to_string())
                } else {
                    param.to_string()
                };
                // Validate the control exists at startup
                if validate_alsa_control(&control) {
                    tracing::info!(control, "Hardware mixer initialized");
                } else {
                    tracing::warn!(
                        control,
                        "ALSA mixer control not found — volume changes will fail. \
                         Available controls: {}",
                        list_alsa_controls().unwrap_or_else(|| "none".into())
                    );
                }
                Self::Hardware { control, volume }
            }
            #[cfg(not(target_os = "linux"))]
            "hardware" => {
                tracing::warn!(
                    "Hardware mixer not supported on this platform, falling back to software"
                );
                Self::Software(volume)
            }
            "midi" => match parse_midi_param(param) {
                Ok((conn, channel, cc)) => {
                    tracing::info!(channel = channel + 1, cc, "MIDI mixer connected");
                    Self::Midi {
                        conn: Mutex::new(conn),
                        channel,
                        cc,
                    }
                }
                Err(e) => {
                    tracing::error!(error = %e, "MIDI mixer init failed, falling back to software");
                    Self::Software(volume)
                }
            },
            "none" => Self::None,
            other => {
                tracing::warn!(mode = other, "Unknown mixer mode, using software");
                Self::Software(volume)
            }
        }
    }

    /// Apply a volume change.
    pub fn set_volume(&self, percent: u8, muted: bool) {
        match self {
            Self::Software(vol) => {
                vol.percent.store(percent, Ordering::Relaxed);
                vol.muted.store(muted, Ordering::Relaxed);
            }
            #[cfg(target_os = "linux")]
            Self::Hardware {
                control, volume, ..
            } => {
                volume.percent.store(percent, Ordering::Relaxed);
                volume.muted.store(muted, Ordering::Relaxed);
                set_alsa_volume(control, percent, muted);
            }
            Self::Midi { conn, channel, cc } => {
                let value = if muted {
                    0
                } else {
                    (u16::from(percent) * u16::from(MIDI_CC_MAX) / 100).min(u16::from(MIDI_CC_MAX))
                        as u8
                };
                if let Ok(mut conn) = conn.lock() {
                    // CC message: 0xB0 | channel, cc, value
                    if let Err(e) = conn.send(&[0xB0 | channel, *cc, value]) {
                        tracing::warn!(error = %e, "MIDI send failed");
                    }
                }
            }
            Self::None => {}
        }
    }

    /// Get the software gain to apply in the audio callback.
    /// Returns 1.0 for hardware/midi/none (volume is handled elsewhere).
    pub fn software_gain(&self) -> f32 {
        match self {
            Self::Software(vol) => vol.gain(),
            _ => 1.0,
        }
    }
}

/// Parse `interface:ch[:cc]` → (`MidiOutputConnection`, `channel_0based`, cc)
fn parse_midi_param(param: &str) -> anyhow::Result<(midir::MidiOutputConnection, u8, u8)> {
    let parts: Vec<&str> = param.rsplitn(3, ':').collect();
    // rsplitn reverses: "IAC Driver:1:7" → ["7", "1", "IAC Driver"]
    let (interface, channel_1, cc) = match parts.len() {
        3 => (parts[2], parts[1], parts[0].parse::<u8>().unwrap_or(7)),
        2 => (parts[1], parts[0], 7u8),
        _ => anyhow::bail!("expected midi:interface:ch[:cc]"),
    };
    let channel: u8 = channel_1
        .parse::<u8>()
        .ok()
        .filter(|&c| (1..=16).contains(&c))
        .map(|c| c - 1)
        .ok_or_else(|| anyhow::anyhow!("MIDI channel must be 1-16, got '{channel_1}'"))?;

    let midi_out = midir::MidiOutput::new("snapdog-client").map_err(|e| anyhow::anyhow!("{e}"))?;
    let port = midi_out
        .ports()
        .into_iter()
        .find(|p| {
            midi_out
                .port_name(p)
                .is_ok_and(|n| n.to_lowercase().contains(&interface.to_lowercase()))
        })
        .ok_or_else(|| {
            let available: Vec<_> = midir::MidiOutput::new("probe")
                .ok()
                .map(|m| {
                    m.ports()
                        .iter()
                        .filter_map(|p| m.port_name(p).ok())
                        .collect()
                })
                .unwrap_or_default();
            anyhow::anyhow!("MIDI interface '{interface}' not found. Available: {available:?}")
        })?;
    let conn = midi_out
        .connect(&port, "snapdog-mixer")
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    Ok((conn, channel, cc))
}

#[cfg(target_os = "linux")]
fn set_alsa_volume(control: &str, percent: u8, muted: bool) {
    let vol = if muted { 0 } else { percent };
    if let Err(e) = set_alsa_volume_inner(control, vol) {
        tracing::warn!(control, error = %e, "Failed to set ALSA volume");
    } else {
        tracing::debug!(control, percent, muted, "Hardware volume set");
    }
}

#[cfg(target_os = "linux")]
fn set_alsa_volume_inner(control: &str, percent: u8) -> anyhow::Result<()> {
    use alsa::mixer::{Mixer, SelemId};
    let mixer = Mixer::new("default", false)?;
    let selem_id = SelemId::new(control, 0);
    let selem = mixer
        .find_selem(&selem_id)
        .ok_or_else(|| anyhow::anyhow!("ALSA control '{control}' not found"))?;
    let (min, max) = selem.get_playback_volume_range();
    // Perceptual volume curve (quadratic) — balances between linear (too quiet
    // at low values on DACs) and cubic (too quiet on amplifiers).
    // 50% → 25% raw, 70% → 49% raw, 90% → 81% raw.
    let normalized = f64::from(percent) / 100.0;
    let curved = normalized * normalized; // quadratic curve
    let vol = min + ((max - min) as f64 * curved) as i64;
    selem.set_playback_volume_all(vol)?;
    if selem.has_playback_switch() {
        selem.set_playback_switch_all(i32::from(percent != 0))?;
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn validate_alsa_control(control: &str) -> bool {
    use alsa::mixer::{Mixer, SelemId};
    let Ok(mixer) = Mixer::new("default", false) else {
        return false;
    };
    let selem_id = SelemId::new(control, 0);
    mixer.find_selem(&selem_id).is_some()
}

#[cfg(target_os = "linux")]
fn list_alsa_controls() -> Option<String> {
    use alsa::mixer::Mixer;
    let mixer = Mixer::new("default", false).ok()?;
    let names: Vec<String> = mixer
        .iter()
        .filter_map(|elem| {
            use alsa::mixer::Selem;
            let selem = Selem::new(elem)?;
            let id = selem.get_id();
            Some(id.get_name().ok()?.to_string())
        })
        .collect();
    Some(names.join(", "))
}

#[cfg(target_os = "linux")]
fn detect_alsa_control() -> Option<String> {
    use alsa::mixer::{Mixer, Selem};
    for candidate in ["Master", "Digital", "PCM", "Speaker"] {
        if validate_alsa_control(candidate) {
            return Some(candidate.to_string());
        }
    }
    // Fall back to first available playback control
    let mixer = Mixer::new("default", false).ok()?;
    mixer.iter().find_map(|elem| {
        let selem = Selem::new(elem)?;
        if selem.has_playback_volume() {
            Some(selem.get_id().get_name().ok()?.to_string())
        } else {
            None
        }
    })
}

/// Start audio output. Waits for the Stream to have audio, then starts cpal.
/// Automatically restarts the pipeline if the stream format changes.
pub async fn play_audio(
    rx: mpsc::Receiver<AudioFrame>,
    stream: Arc<Mutex<Stream>>,
    time_provider: Arc<Mutex<TimeProvider>>,
    eq: SharedEq,
    speaker_eq: SharedEq,
    mixer: Arc<Mixer>,
    fade: Arc<FadeState>,
) {
    // Drain audio_rx in background
    tokio::spawn(async move {
        let mut rx = rx;
        while rx.recv().await.is_some() {}
    });

    // Shared flag for format-change signaling from cpal thread
    let format_changed = Arc::new(AtomicBool::new(false));

    loop {
        format_changed.store(false, Ordering::Relaxed);

        // Wait for the Stream to have a valid format
        let format = loop {
            let f = stream
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .format();
            if f.rate() > 0 && f.channels() > 0 {
                break f;
            }
            tokio::time::sleep(FORMAT_POLL_INTERVAL).await;
        };

        tracing::info!(
            rate = format.rate(),
            bits = format.bits(),
            channels = format.channels(),
            "Audio format detected"
        );

        let stream_clone = Arc::clone(&stream);
        let tp_clone = Arc::clone(&time_provider);
        let eq_clone = eq.clone();
        let speaker_eq_clone = speaker_eq.clone();
        let mixer_clone = Arc::clone(&mixer);
        let fade_clone = Arc::clone(&fade);
        let fc_clone = Arc::clone(&format_changed);

        // Spawn cpal thread — don't join, just let it run
        std::thread::spawn(move || {
            if let Err(e) = run_cpal(
                stream_clone,
                tp_clone,
                format,
                eq_clone,
                speaker_eq_clone,
                mixer_clone,
                fade_clone,
                fc_clone,
            ) {
                tracing::error!(error = %e, "Audio output failed");
            }
        });

        // Poll async until format changes
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            if format_changed.load(Ordering::Relaxed) {
                tracing::info!("Audio format changed, restarting pipeline");
                break;
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
#[allow(clippy::too_many_lines)] // Audio output setup with callback — splitting would obscure control flow
fn run_cpal(
    stream: Arc<Mutex<Stream>>,
    time_provider: Arc<Mutex<TimeProvider>>,
    format: snapcast_proto::SampleFormat,
    eq: SharedEq,
    speaker_eq: SharedEq,
    mixer: Arc<Mixer>,
    fade: Arc<FadeState>,
    format_changed: Arc<AtomicBool>,
) -> anyhow::Result<()> {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or_else(|| anyhow::anyhow!("no output device"))?;

    tracing::info!(device = %device.description().map(|d| format!("{d}")).unwrap_or_default(), "Using audio device");

    // Probe device capabilities — try native format, fallback to default
    let target_rate = format.rate();
    let target_channels = format.channels();
    let config = match device.supported_output_configs() {
        Ok(supported) => {
            let native = supported.into_iter().find(|cfg| {
                cfg.channels() == target_channels
                    && cfg.min_sample_rate() <= target_rate
                    && cfg.max_sample_rate() >= target_rate
            });
            if let Some(cfg) = native {
                cfg.with_sample_rate(target_rate).into()
            } else {
                tracing::warn!(
                    target_rate,
                    target_channels,
                    "Stream format not natively supported, using device default"
                );
                let default: cpal::StreamConfig = device.default_output_config()?.into();
                if default.sample_rate != target_rate {
                    anyhow::bail!(
                        "Device default rate ({} Hz) differs from stream ({target_rate} Hz). \
                         Configure the server to match or enable the 'resampler' feature.",
                        default.sample_rate
                    );
                }
                default
            }
        }
        Err(e) => {
            tracing::warn!(error = %e, "Cannot probe device, using stream format directly");
            cpal::StreamConfig {
                channels: target_channels,
                sample_rate: target_rate,
                buffer_size: cpal::BufferSize::Default,
            }
        }
    };

    let channels = config.channels as usize;

    let cpal_stream = device.build_output_stream(
        &config,
        move |data: &mut [f32], info: &cpal::OutputCallbackInfo| {
            let num_frames = data.len() / channels;

            let buffer_dac_usec = info
                .timestamp()
                .playback
                .duration_since(&info.timestamp().callback)
                .map_or(0, |d| d.as_micros() as i64)
                + {
                    #[allow(clippy::cast_possible_wrap)]
                    let v = num_frames as i64;
                    v * 1_000_000
                } / i64::from(format.rate());

            let server_now = {
                let tp = time_provider
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                now_usec() + tp.diff_to_server_usec()
            };

            let mut s = stream
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let current_format = s.format();

            // Format change detection — output silence if stream format changed
            if current_format.rate() > 0
                && current_format.channels() > 0
                && (current_format.rate() != format.rate()
                    || current_format.channels() != format.channels())
            {
                format_changed.store(true, Ordering::Relaxed);
                data.fill(0.0);
                return;
            }

            let current_frame_size = current_format.frame_size() as usize;
            let current_sample_size = current_format.sample_size() as usize;

            if current_frame_size == 0 {
                data.fill(0.0);
                return;
            }

            let mut pcm_buf = vec![0u8; num_frames * current_frame_size];
            s.get_player_chunk_or_silence(
                server_now,
                buffer_dac_usec,
                &mut pcm_buf,
                num_frames as u32,
            );
            drop(s);

            match current_sample_size {
                2 => {
                    for (i, chunk) in pcm_buf.chunks_exact(2).enumerate() {
                        if i < data.len() {
                            data[i] = f32::from(i16::from_le_bytes([chunk[0], chunk[1]]))
                                / f32::from(i16::MAX);
                        }
                    }
                }
                4 => {
                    for (i, chunk) in pcm_buf.chunks_exact(4).enumerate() {
                        if i < data.len() {
                            data[i] = f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
                        }
                    }
                }
                _ => data.fill(0.0),
            }

            // Apply EQ after PCM decode
            if let Ok(mut eq) = eq.try_lock() {
                eq.process(data);
            }

            // Apply speaker correction after music EQ
            if let Ok(mut spk) = speaker_eq.try_lock() {
                spk.process(data);
            }

            // Apply crossfade (fade-out/fade-in on zone switch)
            fade.process(data, channels);

            // Apply software volume (no-op for hardware/none mixer)
            let gain = mixer.software_gain();
            if gain < 1.0 {
                for sample in data.iter_mut() {
                    *sample *= gain;
                }
            }
        },
        |err| tracing::error!(error = %err, "Audio stream error"),
        None,
    )?;

    cpal_stream.play()?;
    tracing::info!("Audio output stream started");

    // Park indefinitely — the cpal callback sets format_changed,
    // and the async side will spawn a new thread when it detects it.
    std::thread::park();
    Ok(())
}

/// Play a 440 Hz sine wave for 2 seconds to verify audio output, then exit.
pub fn play_test_tone(device_name: &str) -> anyhow::Result<()> {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

    const FREQ: f32 = 440.0;
    const DURATION_SECS: f32 = 2.0;
    const AMPLITUDE: f32 = 0.3;

    let host = cpal::default_host();
    let device = if device_name == "default" {
        host.default_output_device()
    } else {
        host.output_devices()?.find(|d| {
            d.description()
                .is_ok_and(|n| format!("{n}").contains(device_name))
        })
    }
    .ok_or_else(|| anyhow::anyhow!("no output device '{device_name}'"))?;

    let config = device.default_output_config()?;
    let sample_rate = config.sample_rate() as f32;
    let channels = config.channels() as usize;
    let total_samples = (DURATION_SECS * sample_rate) as usize;
    let mut sample_idx = 0usize;

    println!(
        "Playing 440 Hz test tone for 2s on: {}",
        device
            .description()
            .map(|d| format!("{d}"))
            .unwrap_or_default()
    );

    let stream_config = cpal::StreamConfig {
        channels: config.channels() as u16,
        sample_rate: config.sample_rate(),
        buffer_size: cpal::BufferSize::Default,
    };

    let done = Arc::new(AtomicBool::new(false));
    let done_cb = done.clone();

    let stream = device.build_output_stream(
        &stream_config,
        move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
            for frame in data.chunks_mut(channels) {
                if sample_idx >= total_samples {
                    frame.fill(0.0);
                    done_cb.store(true, Ordering::Relaxed);
                    continue;
                }
                let t = sample_idx as f32 / sample_rate;
                let sample = AMPLITUDE * (2.0 * std::f32::consts::PI * FREQ * t).sin();
                frame.fill(sample);
                sample_idx += 1;
            }
        },
        |err| eprintln!("Audio error: {err}"),
        None,
    )?;

    stream.play()?;

    while !done.load(Ordering::Relaxed) {
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    // Let the buffer drain
    std::thread::sleep(std::time::Duration::from_millis(100));

    Ok(())
}
