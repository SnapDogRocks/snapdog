// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! Per-zone parametric EQ using biquad filters.
//!
//! Each band is a second-order IIR filter (biquad) applied independently
//! per channel. Coefficients can be updated glitch-free between samples.

/// Custom message type ID for EQ config over Snapcast custom-protocol.
pub use snapdog_common::MSG_TYPE_EQ_CONFIG as TYPE_EQ_CONFIG;

use biquad::{Biquad, Coefficients, DirectForm2Transposed, Hertz, Q_BUTTERWORTH_F32, ToHertz};
use serde::{Deserialize, Serialize};

// ── Config types (re-exported from snapdog-common) ────────────

pub use snapdog_common::{EqBand, EqConfig, FilterType};

// ── Persistence ───────────────────────────────────────────────

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Serialized form for eq.json.
#[derive(Debug, Default, Serialize, Deserialize)]
struct EqStoreData {
    #[serde(default)]
    zones: HashMap<usize, EqConfig>,
    #[serde(default)]
    clients: HashMap<usize, EqConfig>,
    #[serde(default)]
    speaker_corrections: HashMap<usize, EqConfig>,
}

/// Per-zone and per-client EQ store. Loads from / saves to eq.json.
pub struct EqStore {
    data: EqStoreData,
    path: Option<PathBuf>,
}

impl EqStore {
    /// Load from file, or create empty.
    pub fn load(path: &Path) -> Self {
        let data: EqStoreData = if path.exists() {
            match std::fs::read_to_string(path) {
                Ok(s) => serde_json::from_str(&s).unwrap_or_else(|e| {
                    tracing::warn!(path = %path.display(), error = %e, "Failed to parse EQ config, using defaults");
                    EqStoreData::default()
                }),
                Err(e) => {
                    tracing::warn!(path = %path.display(), error = %e, "Failed to read EQ config, using defaults");
                    EqStoreData::default()
                }
            }
        } else {
            EqStoreData::default()
        };
        if !data.zones.is_empty() || !data.clients.is_empty() {
            tracing::info!(
                zones = data.zones.len(),
                clients = data.clients.len(),
                "EQ config loaded"
            );
        }
        Self {
            data,
            path: Some(path.to_owned()),
        }
    }

    /// Get EQ config for a zone.
    #[must_use]
    pub fn get(&self, zone: usize) -> EqConfig {
        self.data.zones.get(&zone).cloned().unwrap_or_default()
    }

    /// Set EQ config for a zone and persist.
    pub fn set(&mut self, zone: usize, config: EqConfig) {
        self.data.zones.insert(zone, config);
        self.save();
    }

    /// Get EQ config for a client.
    #[must_use]
    pub fn get_client(&self, client: usize) -> EqConfig {
        self.data.clients.get(&client).cloned().unwrap_or_default()
    }

    /// Set EQ config for a client and persist.
    pub fn set_client(&mut self, client: usize, config: EqConfig) {
        self.data.clients.insert(client, config);
        self.save();
    }

    /// Get speaker correction config for a client.
    #[must_use]
    pub fn get_speaker_correction(&self, client: usize) -> EqConfig {
        self.data
            .speaker_corrections
            .get(&client)
            .cloned()
            .unwrap_or_default()
    }

    /// Set speaker correction config for a client and persist.
    pub fn set_speaker_correction(&mut self, client: usize, config: EqConfig) {
        self.data.speaker_corrections.insert(client, config);
        self.save();
    }

    fn save(&self) {
        if let Some(ref path) = self.path {
            if let Ok(json) = serde_json::to_string_pretty(&self.data) {
                if let Err(e) = std::fs::write(path, json) {
                    tracing::warn!(error = %e, "Failed to save eq.json");
                }
            }
        }
    }
}

// ── Presets ───────────────────────────────────────────────────

/// Return the EQ bands for a named preset, or `None` if unknown.
#[must_use]
#[allow(clippy::too_many_lines)] // Data table of EQ presets — not decomposable
pub fn preset(name: &str) -> Option<Vec<EqBand>> {
    Some(match name {
        "flat" => vec![],
        "bass_boost" => vec![EqBand {
            freq: 100.0,
            gain: 6.0,
            q: 0.7,
            filter_type: FilterType::LowShelf,
        }],
        "treble_boost" => vec![EqBand {
            freq: 8000.0,
            gain: 4.0,
            q: 0.7,
            filter_type: FilterType::HighShelf,
        }],
        "vocal" => vec![
            EqBand {
                freq: 200.0,
                gain: -2.0,
                q: 1.0,
                filter_type: FilterType::Peaking,
            },
            EqBand {
                freq: 2500.0,
                gain: 3.0,
                q: 1.0,
                filter_type: FilterType::Peaking,
            },
        ],
        "loudness" => vec![
            EqBand {
                freq: 60.0,
                gain: 4.0,
                q: 0.7,
                filter_type: FilterType::LowShelf,
            },
            EqBand {
                freq: 1000.0,
                gain: 1.0,
                q: 0.5,
                filter_type: FilterType::Peaking,
            },
            EqBand {
                freq: 10000.0,
                gain: 3.0,
                q: 0.7,
                filter_type: FilterType::HighShelf,
            },
        ],
        "rock" => vec![
            EqBand {
                freq: 80.0,
                gain: 3.0,
                q: 0.8,
                filter_type: FilterType::Peaking,
            },
            EqBand {
                freq: 400.0,
                gain: -1.5,
                q: 1.0,
                filter_type: FilterType::Peaking,
            },
            EqBand {
                freq: 3000.0,
                gain: 2.0,
                q: 1.2,
                filter_type: FilterType::Peaking,
            },
            EqBand {
                freq: 10000.0,
                gain: 1.5,
                q: 0.7,
                filter_type: FilterType::HighShelf,
            },
        ],
        "jazz" => vec![
            EqBand {
                freq: 100.0,
                gain: 2.0,
                q: 0.7,
                filter_type: FilterType::LowShelf,
            },
            EqBand {
                freq: 1000.0,
                gain: -1.0,
                q: 0.8,
                filter_type: FilterType::Peaking,
            },
            EqBand {
                freq: 4000.0,
                gain: 1.5,
                q: 1.0,
                filter_type: FilterType::Peaking,
            },
        ],
        "classical" => vec![
            EqBand {
                freq: 60.0,
                gain: 1.5,
                q: 0.7,
                filter_type: FilterType::LowShelf,
            },
            EqBand {
                freq: 500.0,
                gain: -0.5,
                q: 0.8,
                filter_type: FilterType::Peaking,
            },
            EqBand {
                freq: 8000.0,
                gain: 1.0,
                q: 0.7,
                filter_type: FilterType::Peaking,
            },
            EqBand {
                freq: 14000.0,
                gain: 1.5,
                q: 0.7,
                filter_type: FilterType::HighShelf,
            },
        ],
        "electronic" => vec![
            EqBand {
                freq: 50.0,
                gain: 4.0,
                q: 0.8,
                filter_type: FilterType::Peaking,
            },
            EqBand {
                freq: 300.0,
                gain: -2.0,
                q: 1.0,
                filter_type: FilterType::Peaking,
            },
            EqBand {
                freq: 5000.0,
                gain: 2.5,
                q: 1.2,
                filter_type: FilterType::Peaking,
            },
        ],
        "late_night" => vec![
            EqBand {
                freq: 50.0,
                gain: -3.0,
                q: 0.7,
                filter_type: FilterType::LowShelf,
            },
            EqBand {
                freq: 2000.0,
                gain: 2.0,
                q: 1.0,
                filter_type: FilterType::Peaking,
            },
            EqBand {
                freq: 10000.0,
                gain: -2.0,
                q: 0.7,
                filter_type: FilterType::HighShelf,
            },
        ],
        _ => return None,
    })
}

/// List all available preset names.
#[must_use]
pub const fn preset_names() -> &'static [&'static str] {
    &[
        "flat",
        "bass_boost",
        "treble_boost",
        "vocal",
        "rock",
        "jazz",
        "classical",
        "electronic",
        "loudness",
        "late_night",
    ]
}

// ── DSP engine ────────────────────────────────────────────────

/// Per-zone EQ processor. Owns biquad filter instances for each band and channel.
pub struct ZoneEq {
    bands: Vec<BandPair>,
    enabled: bool,
    sample_rate: f32,
    channels: usize,
}

/// One biquad per channel for a single band.
struct BandPair {
    filters: Vec<DirectForm2Transposed<f32>>,
}

impl ZoneEq {
    /// Create a new EQ processor.
    #[must_use]
    pub const fn new(sample_rate: u32, channels: u16) -> Self {
        Self {
            bands: vec![],
            enabled: false,
            sample_rate: sample_rate as f32,
            channels: channels as usize,
        }
    }

    /// Update the EQ configuration. Rebuilds all filter coefficients.
    pub fn set_config(&mut self, config: &EqConfig) {
        self.enabled = config.enabled;
        self.bands = config
            .bands
            .iter()
            .filter_map(|b| self.make_band(b))
            .collect();
    }

    /// Process interleaved f32 samples in-place.
    pub fn process(&mut self, samples: &mut [f32]) {
        if !self.enabled || self.bands.is_empty() {
            return;
        }
        let ch = self.channels;
        let frames = samples.len() / ch;
        for frame in 0..frames {
            for band in &mut self.bands {
                for c in 0..ch {
                    let idx = frame * ch + c;
                    samples[idx] = band.filters[c].run(samples[idx]);
                }
            }
        }
    }

    fn make_band(&self, band: &EqBand) -> Option<BandPair> {
        let fs: Hertz<f32> = self.sample_rate.hz();
        let f0: Hertz<f32> = band.freq.hz();
        let q = if band.q > 0.0 {
            band.q
        } else {
            Q_BUTTERWORTH_F32
        };

        let filter_type = match band.filter_type {
            FilterType::LowShelf => biquad::Type::LowShelf(band.gain),
            FilterType::HighShelf => biquad::Type::HighShelf(band.gain),
            FilterType::Peaking => biquad::Type::PeakingEQ(band.gain),
            FilterType::LowPass => biquad::Type::LowPass,
            FilterType::HighPass => biquad::Type::HighPass,
        };

        let coeffs = Coefficients::<f32>::from_params(filter_type, fs, f0, q).ok()?;
        let filters = (0..self.channels)
            .map(|_| DirectForm2Transposed::<f32>::new(coeffs))
            .collect();
        Some(BandPair { filters })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passthrough_when_disabled() {
        let mut eq = ZoneEq::new(48000, 2);
        let mut samples = vec![0.5f32, -0.5, 0.3, -0.3];
        let original = samples.clone();
        eq.process(&mut samples);
        assert_eq!(samples, original);
    }

    #[test]
    fn passthrough_when_no_bands() {
        let mut eq = ZoneEq::new(48000, 2);
        eq.set_config(&EqConfig {
            enabled: true,
            bands: vec![],
            preset: None,
        });
        let mut samples = vec![0.5f32, -0.5, 0.3, -0.3];
        let original = samples.clone();
        eq.process(&mut samples);
        assert_eq!(samples, original);
    }

    #[test]
    fn modifies_signal_when_enabled() {
        let mut eq = ZoneEq::new(48000, 2);
        eq.set_config(&EqConfig {
            enabled: true,
            bands: vec![EqBand {
                freq: 1000.0,
                gain: 6.0,
                q: 1.0,
                filter_type: FilterType::Peaking,
            }],
            preset: None,
        });
        let mut samples: Vec<f32> = (0..1024).map(|i| (i as f32 * 0.1).sin()).collect();
        let original = samples.clone();
        eq.process(&mut samples);
        assert_ne!(samples, original);
    }

    #[test]
    fn preset_flat_returns_empty() {
        assert_eq!(preset("flat").unwrap().len(), 0);
    }

    #[test]
    fn preset_unknown_returns_none() {
        assert!(preset("nonexistent").is_none());
    }
}
