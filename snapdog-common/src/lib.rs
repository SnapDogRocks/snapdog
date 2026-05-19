// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! Shared types and constants for `SnapDog` server and client.

// Pedantic lints allowed crate-wide: arithmetic casts are intentional in audio math,
// float comparisons are acceptable for gain/volume values, and must_use on every
// public helper adds noise without safety benefit.
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::float_cmp)]
#![forbid(unsafe_code)]
#![warn(missing_docs)]

use serde::{Deserialize, Serialize};

/// Client name used by `SnapDog` clients to identify themselves to the server.
pub const CLIENT_NAME: &str = "SnapDog";

/// Snapcast custom message type ID for EQ configuration.
pub const MSG_TYPE_EQ_CONFIG: u16 = 10;

/// Snapcast custom message type ID for speaker correction EQ.
pub const MSG_TYPE_SPEAKER_EQ: u16 = 11;

/// Snapcast custom message type ID for audio fade-out trigger.
/// Payload: fade duration in milliseconds as u16 little-endian.
pub const MSG_TYPE_FADE_OUT: u16 = 12;

/// Default crossfade duration in milliseconds.
pub const DEFAULT_FADE_MS: u16 = 300;

/// Default audio sample rate in Hz.
pub const DEFAULT_SAMPLE_RATE: u32 = 48000;

/// Maximum number of EQ bands per zone/client.
pub const MAX_EQ_BANDS: usize = 10;

// ── EQ types ──────────────────────────────────────────────────

/// Filter type for an EQ band.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FilterType {
    /// Boosts or cuts frequencies below the cutoff.
    LowShelf,
    /// Boosts or cuts frequencies above the cutoff.
    HighShelf,
    /// Boosts or cuts a narrow band around the center frequency.
    Peaking,
    /// Passes frequencies below the cutoff, attenuates above.
    LowPass,
    /// Passes frequencies above the cutoff, attenuates below.
    HighPass,
}

/// Single EQ band configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EqBand {
    /// Center frequency in Hz.
    pub freq: f32,
    /// Gain in dB (positive = boost, negative = cut). Ignored for low/high pass.
    pub gain: f32,
    /// Q factor controlling bandwidth. Higher values = narrower band.
    pub q: f32,
    /// Filter type (low shelf, high shelf, peaking, low pass, high pass).
    #[serde(rename = "type")]
    pub filter_type: FilterType,
}

/// Full EQ configuration for a zone.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EqConfig {
    /// Whether the EQ is active. When `false`, audio passes through unmodified.
    pub enabled: bool,
    /// Ordered list of biquad filter bands applied in series.
    pub bands: Vec<EqBand>,
    /// Name of the preset this config was loaded from, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preset: Option<String>,
}

impl Default for EqConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            bands: vec![],
            preset: Some("flat".into()),
        }
    }
}

// ── Volume ────────────────────────────────────────────────────

/// Calculate linear fade gain for a given position.
/// Returns 1.0→0.0 for fade-out, 0.0→1.0 for fade-in.
#[must_use]
#[inline]
pub fn fade_gain(remaining: u32, total: u32, fading_out: bool) -> f32 {
    if total == 0 {
        return 1.0;
    }
    let pos = remaining as f32 / total as f32;
    if fading_out { pos } else { 1.0 - pos }
}

/// Perceptual (quadratic) volume curve: maps linear 0–100 to 0.0–1.0.
/// Input: linear percentage (0–100). Output: gain factor (0.0–1.0).
#[must_use]
pub fn perceptual_volume(linear: u8) -> f32 {
    let normalized = f32::from(linear) / 100.0;
    normalized * normalized
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn volume_curve_boundaries() {
        assert_eq!(perceptual_volume(0), 0.0);
        assert_eq!(perceptual_volume(100), 1.0);
    }

    #[test]
    fn volume_curve_midpoint() {
        let mid = perceptual_volume(50);
        assert!((mid - 0.25).abs() < 0.001);
    }

    #[test]
    fn eq_config_default() {
        let config = EqConfig::default();
        assert!(!config.enabled);
        assert!(config.bands.is_empty());
        assert_eq!(config.preset, Some("flat".into()));
    }

    #[test]
    fn fade_gain_zero_total() {
        assert_eq!(fade_gain(0, 0, true), 1.0);
        assert_eq!(fade_gain(0, 0, false), 1.0);
    }

    #[test]
    fn fade_gain_out_full_to_zero() {
        assert_eq!(fade_gain(100, 100, true), 1.0);
        assert_eq!(fade_gain(50, 100, true), 0.5);
        assert_eq!(fade_gain(0, 100, true), 0.0);
    }

    #[test]
    fn fade_gain_in_zero_to_full() {
        assert_eq!(fade_gain(100, 100, false), 0.0);
        assert_eq!(fade_gain(50, 100, false), 0.5);
        assert_eq!(fade_gain(0, 100, false), 1.0);
    }
}
