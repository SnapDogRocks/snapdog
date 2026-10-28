//! Audio output — cpal callback reads from Stream directly.

use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use snapcast_client::AudioFrame;
use snapcast_client::connection::now_usec;
use snapcast_client::stream::Stream;
use snapcast_client::time_provider::TimeProvider;
use tokio::sync::mpsc;

use crate::eq::ZoneEq;

pub mod osc_desk;

/// Maximum MIDI CC value (7-bit).
const MIDI_CC_MAX: u8 = 127;
/// Polling interval while waiting for audio format from the stream.
const FORMAT_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(100);
/// Amplitude below which audio is considered silence (for fade-in detection).
const SILENCE_THRESHOLD: f32 = 1e-6;

/// Shared EQ processor, updated from the event loop, read from the audio thread.
pub type SharedEq = Arc<Mutex<ZoneEq>>;

/// Passes a volume and mute state changed at the mixer itself on to the server.
/// Called from the mixer's own thread.
pub type VolumeReport = Box<dyn Fn(u8, bool) + Send>;

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
    /// Channel faders on a networked mixing desk over OSC (Behringer X32/M32).
    Desk(osc_desk::DeskMixer),
    /// No volume control.
    None,
}

impl Mixer {
    /// Build the mixer from `--mixer`. Mixers that can read their own state back use
    /// `report` as the reverse channel; the others ignore it.
    pub fn from_cli(raw: &str, volume: Arc<VolumeState>, report: Option<VolumeReport>) -> Self {
        let (mode, param) = raw.split_once(':').unwrap_or((raw, ""));
        match mode {
            "software" | "" => Self::Software(volume),
            #[cfg(target_os = "linux")]
            "hardware" => Self::alsa(param, volume),
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
            "x32" => match start_x32(param, report) {
                Ok(desk) => Self::Desk(desk),
                Err(e) => {
                    tracing::error!(error = %e, "X32 mixer init failed, falling back to software");
                    Self::Software(volume)
                }
            },
            "none" => Self::None,
            // Accepted by the command line (compatibility), but there is no script
            // mixer; the startup check has already said so.
            "script" => Self::Software(volume),
            other => {
                tracing::warn!(mode = other, "Unknown mixer mode, using software");
                Self::Software(volume)
            }
        }
    }

    /// ALSA control `param`, or an auto-detected one when empty (Linux only).
    #[cfg(target_os = "linux")]
    fn alsa(param: &str, volume: Arc<VolumeState>) -> Self {
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
                let value = midi_cc_value(percent, muted);
                if let Ok(mut conn) = conn.lock() {
                    // CC message: 0xB0 | channel, cc, value
                    if let Err(e) = conn.send(&[0xB0 | channel, *cc, value]) {
                        tracing::warn!(error = %e, "MIDI send failed");
                    }
                }
            }
            Self::Desk(desk) => desk.set(percent, muted),
            Self::None => {}
        }
    }

    /// Get the software gain to apply in the audio callback.
    /// Returns 1.0 for hardware/midi/desk/none (volume is handled elsewhere).
    pub fn software_gain(&self) -> f32 {
        match self {
            Self::Software(vol) => vol.gain(),
            _ => 1.0,
        }
    }
}

/// Start the desk mixer for `x32:<host>:<channels>[:<port>]`.
fn start_x32(param: &str, report: Option<VolumeReport>) -> anyhow::Result<osc_desk::DeskMixer> {
    let target = osc_desk::parse_x32_param(param)?;
    let sync = if report.is_some() { "both" } else { "out" };
    let desk = osc_desk::DeskMixer::start(osc_desk::X32, &target, report)?;
    tracing::info!(
        addr = %target.addr,
        channels = ?target.channels,
        sync,
        "X32 mixer started"
    );
    Ok(desk)
}

/// Volume as a MIDI CC value: 0–100 % over 0–127, 0 when muted.
fn midi_cc_value(percent: u8, muted: bool) -> u8 {
    if muted {
        return 0;
    }
    (u16::from(percent) * u16::from(MIDI_CC_MAX) / 100).min(u16::from(MIDI_CC_MAX)) as u8
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

/// Which output device `--soundcard` names.
#[derive(Debug, PartialEq, Eq)]
enum DeviceSelector<'a> {
    /// The system default.
    Default,
    /// Position in the list printed by `--list`.
    Index(usize),
    /// Any device whose name or id contains this text, ignoring case.
    Name(&'a str),
}

/// An output device as `--list` shows it and `--soundcard` matches it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct OutputDevice {
    /// Human-readable description.
    name: String,
    /// Stable identifier of the audio host (`alsa:hw:CARD=...`), empty when unknown.
    id: String,
}

impl<'a> DeviceSelector<'a> {
    fn parse(raw: &'a str) -> Self {
        let raw = raw.trim();
        if raw.is_empty() || raw.eq_ignore_ascii_case("default") {
            Self::Default
        } else if let Ok(index) = raw.parse() {
            Self::Index(index)
        } else {
            Self::Name(raw)
        }
    }

    /// Position of the selected device in `devices`, or `None` for the system default.
    fn pick(&self, devices: &[OutputDevice]) -> Result<Option<usize>, String> {
        match self {
            Self::Default => Ok(None),
            Self::Index(index) if *index < devices.len() => Ok(Some(*index)),
            Self::Index(index) => Err(format!(
                "there is no output device with index {index} ({} found)",
                devices.len()
            )),
            Self::Name(part) => {
                let needle = part.to_lowercase();
                let exact: Vec<usize> = devices
                    .iter()
                    .enumerate()
                    .filter(|(_, d)| d.is_called(&needle))
                    .map(|(i, _)| i)
                    .collect();
                // A name that is a device's whole name or id wins over longer ones that
                // merely contain it: `hw:CARD=X,DEV=0` is part of `plughw:CARD=X,DEV=0`.
                if let [one] = exact.as_slice() {
                    return Ok(Some(*one));
                }
                let matches: Vec<usize> = if exact.is_empty() {
                    devices
                        .iter()
                        .enumerate()
                        .filter(|(_, d)| {
                            d.name.to_lowercase().contains(&needle)
                                || d.id.to_lowercase().contains(&needle)
                        })
                        .map(|(i, _)| i)
                        .collect()
                } else {
                    exact
                };
                match matches.as_slice() {
                    [one] => Ok(Some(*one)),
                    [] => Err(format!(
                        "no output device name or id contains '{part}'{}",
                        if looks_like_alsa_name(part) {
                            " (Snapclient's ALSA names such as hw:1 are not understood: use the index \
                             or a part of the name or id from --list)"
                        } else {
                            ""
                        }
                    )),
                    several => Err(format!(
                        "'{part}' matches {} output devices, be more specific or use the index: {}",
                        several.len(),
                        several
                            .iter()
                            .map(|i| format!("{i}: {}", devices[*i].label()))
                            .collect::<Vec<_>>()
                            .join("; ")
                    )),
                }
            }
        }
    }
}

/// `hw:1`, `plughw:CARD=x,DEV=0`: how Snapclient's `--soundcard` names an ALSA device.
fn looks_like_alsa_name(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    [
        "hw:",
        "plughw:",
        "sysdefault:",
        "front:",
        "surround",
        "iec958:",
    ]
    .iter()
    .any(|prefix| lower.starts_with(prefix))
}

fn output_devices(host: &cpal::Host) -> anyhow::Result<(Vec<cpal::Device>, Vec<OutputDevice>)> {
    use cpal::traits::{DeviceTrait, HostTrait};
    let devices: Vec<cpal::Device> = host.output_devices()?.collect();
    let described = devices
        .iter()
        .map(|d| OutputDevice {
            name: d
                .description()
                .map_or_else(|_| "(unnamed device)".to_string(), |n| format!("{n}")),
            id: d.id().map(|id| id.to_string()).unwrap_or_default(),
        })
        .collect();
    Ok((devices, described))
}

/// `0: name [id]; 1: name [id]` for an error message, or `none`.
fn device_list(devices: &[OutputDevice]) -> String {
    if devices.is_empty() {
        return "none".to_string();
    }
    devices
        .iter()
        .enumerate()
        .map(|(i, d)| format!("{i}: {}", d.label()))
        .collect::<Vec<_>>()
        .join("; ")
}

impl OutputDevice {
    /// Whether `lowercase` (already lower-cased) is this device's whole name or whole id,
    /// the id also without its audio-host prefix (`alsa:`).
    fn is_called(&self, lowercase: &str) -> bool {
        let id = self.id.to_lowercase();
        let bare = id.split_once(':').map_or("", |(_, rest)| rest);
        self.name.to_lowercase() == lowercase
            || (!id.is_empty() && (id == lowercase || bare == lowercase))
    }

    /// `name [id]`, or just `name` when there is no id.
    fn label(&self) -> String {
        if self.id.is_empty() {
            self.name.clone()
        } else {
            format!("{} [{}]", self.name, self.id)
        }
    }
}

/// The output device chosen with `--soundcard`.
pub fn find_output_device(host: &cpal::Host, soundcard: &str) -> anyhow::Result<cpal::Device> {
    use cpal::traits::HostTrait;
    let selector = DeviceSelector::parse(soundcard);
    if selector == DeviceSelector::Default {
        return host
            .default_output_device()
            .ok_or_else(|| anyhow::anyhow!("no output device"));
    }
    let (mut devices, described) = output_devices(host).map_err(|e| {
        anyhow::anyhow!("--soundcard {soundcard}: cannot list the output devices: {e}")
    })?;
    match selector.pick(&described) {
        Ok(Some(index)) => Ok(devices.swap_remove(index)),
        Ok(None) => unreachable!("handled above"),
        Err(why) => anyhow::bail!(
            "--soundcard {soundcard}: {why}. Available devices: {}",
            device_list(&described)
        ),
    }
}

/// Fail at startup if `--soundcard` names a device that does not exist.
pub fn check_output_device(soundcard: &str) -> anyhow::Result<()> {
    if DeviceSelector::parse(soundcard) == DeviceSelector::Default {
        return Ok(());
    }
    find_output_device(&cpal::default_host(), soundcard).map(|_| ())
}

/// Print the output devices, one `index: name [id]` per line (`--list`). The device the
/// system uses by default is marked once, even when several devices share its name.
pub fn list_output_devices() -> anyhow::Result<()> {
    use cpal::traits::{DeviceTrait, HostTrait};
    let host = cpal::default_host();
    let (_, devices) = output_devices(&host)?;
    let default_id = host
        .default_output_device()
        .and_then(|d| d.id().ok())
        .map(|id| id.to_string());
    if devices.is_empty() {
        println!("No audio output devices found");
    }
    let default_index = default_id
        .as_deref()
        .and_then(|id| devices.iter().position(|d| d.id == id));
    for (index, device) in devices.iter().enumerate() {
        let marker = if default_index == Some(index) {
            " (default)"
        } else {
            ""
        };
        println!("{index}: {}{marker}", device.label());
    }
    Ok(())
}

/// Start audio output. Waits for the Stream to have audio, then starts cpal.
/// Automatically restarts the pipeline if the stream format changes.
#[allow(clippy::too_many_arguments)]
pub async fn play_audio(
    soundcard: Arc<str>,
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
        let soundcard_clone = Arc::clone(&soundcard);

        // Spawn cpal thread — don't join, just let it run
        std::thread::spawn(move || {
            if let Err(e) = run_cpal(
                &soundcard_clone,
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
#[allow(clippy::too_many_lines)]
// Audio output setup with callback — splitting would obscure control flow
fn run_cpal(
    soundcard: &str,
    stream: Arc<Mutex<Stream>>,
    time_provider: Arc<Mutex<TimeProvider>>,
    format: snapcast_proto::SampleFormat,
    eq: SharedEq,
    speaker_eq: SharedEq,
    mixer: Arc<Mixer>,
    fade: Arc<FadeState>,
    format_changed: Arc<AtomicBool>,
) -> anyhow::Result<()> {
    use cpal::traits::{DeviceTrait, StreamTrait};

    let host = cpal::default_host();
    let device = find_output_device(&host, soundcard)?;

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
        config,
        move |data: &mut [f32], info: &cpal::OutputCallbackInfo| {
            let num_frames = data.len() / channels;

            let buffer_dac_usec = i64::try_from(
                info.timestamp()
                    .playback
                    .duration_since(info.timestamp().callback)
                    .as_micros(),
            )
            .unwrap_or(i64::MAX)
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
                    for (i, chunk) in pcm_buf.as_chunks::<2>().0.iter().enumerate() {
                        if i < data.len() {
                            data[i] = f32::from(i16::from_le_bytes([chunk[0], chunk[1]]))
                                / f32::from(i16::MAX);
                        }
                    }
                }
                4 => {
                    for (i, chunk) in pcm_buf.as_chunks::<4>().0.iter().enumerate() {
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
    use cpal::traits::{DeviceTrait, StreamTrait};

    const FREQ: f32 = 440.0;
    const DURATION_SECS: f32 = 2.0;
    const AMPLITUDE: f32 = 0.3;

    let host = cpal::default_host();
    let device = find_output_device(&host, device_name)?;

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
        stream_config,
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

#[cfg(test)]
mod tests {
    // ── --soundcard selection ─────────────────────────────────

    fn devices(list: &[(&str, &str)]) -> Vec<OutputDevice> {
        list.iter()
            .map(|(name, id)| OutputDevice {
                name: (*name).to_string(),
                id: (*id).to_string(),
            })
            .collect()
    }

    #[test]
    fn soundcard_selector_parses_default_index_and_name() {
        assert_eq!(DeviceSelector::parse("default"), DeviceSelector::Default);
        assert_eq!(DeviceSelector::parse("DEFAULT"), DeviceSelector::Default);
        assert_eq!(DeviceSelector::parse(""), DeviceSelector::Default);
        assert_eq!(DeviceSelector::parse("2"), DeviceSelector::Index(2));
        assert_eq!(DeviceSelector::parse("hw:1"), DeviceSelector::Name("hw:1"));
        assert_eq!(DeviceSelector::parse(" USB "), DeviceSelector::Name("USB"));
    }

    #[test]
    fn soundcard_selector_picks_by_index_and_unique_name() {
        let list = devices(&[
            ("Built-in Output", "coreaudio:1"),
            ("USB Audio DAC", "coreaudio:2"),
            ("HDMI", "coreaudio:3"),
        ]);
        assert_eq!(DeviceSelector::Default.pick(&list), Ok(None));
        assert_eq!(DeviceSelector::Index(1).pick(&list), Ok(Some(1)));
        assert_eq!(DeviceSelector::Name("DAC").pick(&list), Ok(Some(1)));
    }

    #[test]
    fn soundcard_name_matching_ignores_case() {
        let list = devices(&[("Built-in Output", ""), ("USB Audio DAC", "")]);
        for text in ["usb audio dac", "USB AUDIO DAC", "uSb", "dac"] {
            assert_eq!(
                DeviceSelector::Name(text).pick(&list),
                Ok(Some(1)),
                "{text}"
            );
        }
    }

    #[test]
    fn soundcard_can_be_chosen_by_the_id_that_list_prints() {
        let list = devices(&[
            ("Card A", "alsa:hw:CARD=A,DEV=0"),
            ("Card B", "alsa:hw:CARD=B,DEV=0"),
        ]);
        assert_eq!(DeviceSelector::Name("hw:CARD=B").pick(&list), Ok(Some(1)));
        assert_eq!(DeviceSelector::Name("hw:card=a").pick(&list), Ok(Some(0)));
    }

    #[test]
    fn a_whole_id_beats_the_longer_ids_that_contain_it() {
        let list = devices(&[
            ("Card X", "alsa:plughw:CARD=X,DEV=0"),
            ("Card X", "alsa:hw:CARD=X,DEV=0"),
            ("Card X", "alsa:hw:CARD=X,DEV=1"),
        ]);
        // `hw:CARD=X,DEV=0` is a substring of the plughw id as well, but equals one id
        // once the host prefix is left off.
        assert_eq!(
            DeviceSelector::Name("hw:CARD=X,DEV=0").pick(&list),
            Ok(Some(1))
        );
        assert_eq!(
            DeviceSelector::Name("alsa:hw:CARD=X,DEV=0").pick(&list),
            Ok(Some(1))
        );
        assert_eq!(
            DeviceSelector::Name("ALSA:PLUGHW:card=x,dev=0").pick(&list),
            Ok(Some(0))
        );
        // A part that fits several is refused, and the error lists them with their ids.
        let error = DeviceSelector::Name("CARD=X").pick(&list).unwrap_err();
        assert!(error.contains("matches 3 output devices"), "{error}");
        assert!(
            error.contains("0: Card X [alsa:plughw:CARD=X,DEV=0]"),
            "{error}"
        );
        assert!(
            error.contains("2: Card X [alsa:hw:CARD=X,DEV=1]"),
            "{error}"
        );
        let error = DeviceSelector::Name("hw:CARD=X,DEV=")
            .pick(&list)
            .unwrap_err();
        assert!(error.contains("matches 3 output devices"), "{error}");
    }

    #[test]
    fn two_devices_with_the_same_whole_name_are_still_ambiguous() {
        let list = devices(&[("Speakers", "a:1"), ("Speakers", "a:2")]);
        let error = DeviceSelector::Name("speakers").pick(&list).unwrap_err();
        assert!(error.contains("matches 2 output devices"), "{error}");
        assert!(
            error.contains("0: Speakers [a:1]; 1: Speakers [a:2]"),
            "{error}"
        );
    }

    #[test]
    fn soundcard_selector_refuses_what_it_cannot_find_or_decide() {
        let list = devices(&[("USB Audio DAC", "x:1"), ("USB Audio Line", "x:2")]);
        let missing = DeviceSelector::Index(5).pick(&list).unwrap_err();
        assert!(
            missing.contains("no output device with index 5"),
            "{missing}"
        );
        let none = DeviceSelector::Name("HDMI").pick(&list).unwrap_err();
        assert!(
            none.contains("no output device name or id contains 'HDMI'"),
            "{none}"
        );
        assert!(
            !none.contains("Snapclient"),
            "no ALSA hint for a plain name: {none}"
        );
        let ambiguous = DeviceSelector::Name("USB").pick(&list).unwrap_err();
        assert!(
            ambiguous.contains("matches 2 output devices"),
            "{ambiguous}"
        );
    }

    #[test]
    fn snapclient_alsa_names_get_a_hint_instead_of_a_silent_failure() {
        let list = devices(&[("Built-in Output", "coreaudio:1")]);
        for name in ["hw:1", "plughw:0,0", "sysdefault:CARD=x", "HW:2"] {
            let error = DeviceSelector::Name(name).pick(&list).unwrap_err();
            assert!(error.contains("not understood"), "{name}: {error}");
        }
        assert!(looks_like_alsa_name("hw:1"));
        assert!(!looks_like_alsa_name("USB Audio"));
    }

    #[test]
    fn device_list_shows_index_name_and_id() {
        assert_eq!(device_list(&[]), "none");
        let list = devices(&[("Card A", "alsa:hw:A"), ("Plain", "")]);
        assert_eq!(device_list(&list), "0: Card A [alsa:hw:A]; 1: Plain");
    }

    /// `--soundcard` has to reach the place that opens the device, not only the startup
    /// check: with a name that matches nothing, opening must fail and say why. If the
    /// choice were dropped, this would open the system default device instead and run on.
    #[test]
    fn the_audio_thread_opens_the_chosen_soundcard_and_not_the_default() {
        let format = snapcast_proto::SampleFormat::new(48000, 16, 2);
        let (done, result) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let outcome = run_cpal(
                "no-such-device-0xdead",
                Arc::new(Mutex::new(Stream::new(format))),
                Arc::new(Mutex::new(TimeProvider::new())),
                format,
                Arc::new(Mutex::new(ZoneEq::new(48000, 2))),
                Arc::new(Mutex::new(ZoneEq::new(48000, 2))),
                Arc::new(Mixer::None),
                Arc::new(FadeState::new()),
                Arc::new(AtomicBool::new(false)),
            );
            let _ = done.send(outcome.map_err(|e| e.to_string()));
        });
        let outcome = result
            .recv_timeout(std::time::Duration::from_secs(20))
            .expect("opening an unknown soundcard returns instead of playing on another device");
        let error = outcome.expect_err("an unknown soundcard must not open");
        assert!(
            error.contains("--soundcard no-such-device-0xdead"),
            "{error}"
        );
    }

    use super::osc_desk::{Desk, X32};
    use super::*;

    /// `--mixer midi` and `--mixer x32` drive the same X32 fader; a volume must land in the
    /// same place either way. The X32 in Generic CC mode sets the fader to CC / 128
    /// (measured: CC 96 = 0.75 = 0 dB). MIDI has 128 steps and rounds down, OSC has 1024,
    /// so they may differ by up to two MIDI steps, never more.
    #[test]
    fn midi_and_x32_put_the_fader_in_the_same_place() {
        for percent in 0..=100 {
            let midi = f32::from(midi_cc_value(percent, false)) / 128.0;
            let osc = X32.fader_value(percent);
            assert!(
                (osc - midi).abs() <= 2.0 / 128.0,
                "{percent} %: OSC {osc}, MIDI {midi}"
            );
        }
        assert_eq!(midi_cc_value(100, true), 0);
    }
}
