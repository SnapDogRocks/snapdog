// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! Persistent audio resamplers for streaming PCM data.
//!
//! [`F32Resampler`] / [`F32Resampling`] — F32 samples in/out, used for all audio paths.
//!
//! Wraps rubato's `Async` sinc resampler (fixed input size) with internal
//! buffers to handle arbitrary input chunk sizes. Filter state is maintained
//! across calls for artifact-free output.

use rubato::audioadapter_buffers::direct::SequentialSliceOfVecs;
use rubato::{
    Async, FixedAsync, Resampler, SincInterpolationParameters, SincInterpolationType,
    WindowFunction,
};

/// Sinc filter length — 256 taps provides excellent stopband attenuation (>100 dB)
/// while keeping latency acceptable for real-time streaming.
const SINC_LEN: usize = 256;

/// Oversampling factor for the sinc interpolation lookup table.
/// 256× oversampling gives sub-sample accuracy without audible interpolation artifacts.
const OVERSAMPLING_FACTOR: usize = 256;

/// Anti-aliasing filter cutoff as fraction of Nyquist.
/// 0.95 preserves content up to 95% of Nyquist while preventing aliasing at the transition band.
const F_CUTOFF: f32 = 0.95;

/// Rubato parameters shared by both resamplers.
const fn sinc_params() -> SincInterpolationParameters {
    SincInterpolationParameters {
        sinc_len: SINC_LEN,
        f_cutoff: Some(F_CUTOFF),
        interpolation: SincInterpolationType::Linear,
        oversampling_factor: OVERSAMPLING_FACTOR,
        window: WindowFunction::BlackmanHarris2,
    }
}

const CHUNK_SIZE: usize = 1024;

// ── F32 resampler ─────────────────────────────────────────────

/// Streaming resampler that operates on F32 interleaved samples.
///
/// Resamples in f32→f64→f32 precision. Used for all audio paths.
pub struct F32Resampler {
    resampler: Async<f64>,
    channels: usize,
    buffer: Vec<Vec<f64>>,
    chunk_size: usize,
}

impl F32Resampler {
    /// Create a new F32 resampler. Returns `None` if `source_rate` == `target_rate`.
    pub fn new(source_rate: u32, target_rate: u32, channels: u16) -> Option<Self> {
        if source_rate == target_rate {
            return None;
        }

        let ch = channels as usize;
        let params = sinc_params();
        let resampler = Async::<f64>::new_sinc(
            f64::from(target_rate) / f64::from(source_rate),
            2.0,
            &params,
            CHUNK_SIZE,
            ch,
            FixedAsync::Input,
        )
        .map_err(|e| tracing::error!(error = %e, "Failed to create F32 resampler"))
        .ok()?;

        tracing::info!(
            source_rate,
            target_rate,
            channels = ch,
            "F32 resampler created"
        );

        Some(Self {
            resampler,
            channels: ch,
            buffer: vec![Vec::new(); ch],
            chunk_size: CHUNK_SIZE,
        })
    }

    /// Feed F32 interleaved samples, get back resampled F32 interleaved samples.
    pub fn process(&mut self, samples: &[f32]) -> Vec<f32> {
        let frames = samples.len() / self.channels;
        for frame in 0..frames {
            for ch in 0..self.channels {
                self.buffer[ch].push(f64::from(samples[frame * self.channels + ch]));
            }
        }

        let mut output = Vec::new();
        while self.buffer[0].len() >= self.chunk_size {
            let chunk: Vec<Vec<f64>> = self
                .buffer
                .iter_mut()
                .map(|ch_buf| ch_buf.drain(..self.chunk_size).collect())
                .collect();

            // Planar (sequential) f64 input: one Vec per channel, each `chunk_size` frames.
            let adapter = match SequentialSliceOfVecs::new(&chunk, self.channels, self.chunk_size) {
                Ok(a) => a,
                Err(e) => {
                    tracing::warn!(error = %e, "F32 resample input adapter error, dropping chunk");
                    continue;
                }
            };
            match self.resampler.process(&adapter, None) {
                Ok(resampled) => {
                    // Interleaved f64 output ([f0c0, f0c1, f1c0, …]); cast to interleaved f32.
                    output.extend(resampled.take_data().into_iter().map(|s| s as f32));
                }
                Err(e) => tracing::warn!(error = %e, "F32 resample error, dropping chunk"),
            }
        }

        output
    }
}

/// Passthrough or resample F32.
/// Passthrough or active resampling for F32 audio.
pub enum F32Resampling {
    /// Source and target rates match — no processing needed.
    Passthrough,
    /// Active resampling via rubato.
    Active(F32Resampler),
}

impl F32Resampling {
    /// Create a new resampler, or passthrough if rates match.
    pub fn new(source_rate: u32, target_rate: u32, channels: u16) -> Self {
        F32Resampler::new(source_rate, target_rate, channels)
            .map_or(Self::Passthrough, Self::Active)
    }

    /// Returns resampled F32 data, or `None` when buffering (not enough input yet).
    /// Passthrough returns the original data.
    pub fn process(&mut self, samples: &[f32]) -> Option<Vec<f32>> {
        match self {
            Self::Passthrough => Some(samples.to_vec()),
            Self::Active(r) => {
                let out = r.process(samples);
                if out.is_empty() { None } else { Some(out) }
            }
        }
    }

    /// Like `process`, but takes ownership to avoid cloning on passthrough.
    pub fn process_or_passthrough(&mut self, samples: Vec<f32>) -> Vec<f32> {
        match self {
            Self::Passthrough => samples,
            Self::Active(r) => r.process(&samples),
        }
    }
}

// ── Conversion ────────────────────────────────────────────────

/// Convert F32 interleaved samples to PCM bytes at the given bit depth.
///
/// Supports 16-bit (S16LE), 24-bit (S24LE), and 32-bit (S32LE) output.
/// This is the final output stage before writing to snapserver.
#[inline]
pub fn f32_to_pcm(samples: &[f32], bit_depth: u16) -> Vec<u8> {
    let bytes_per_sample = (bit_depth / 8) as usize;
    let mut out = Vec::with_capacity(samples.len() * bytes_per_sample);
    match bit_depth {
        16 => {
            for &s in samples {
                let v = (s * 32767.0).clamp(-32768.0, 32767.0) as i16;
                out.extend_from_slice(&v.to_le_bytes());
            }
        }
        24 => {
            for &s in samples {
                let v = (s * 8_388_607.0).clamp(-8_388_608.0, 8_388_607.0) as i32;
                let b = v.to_le_bytes();
                out.extend_from_slice(&b[..3]); // lower 3 bytes of i32
            }
        }
        32 => {
            for &s in samples {
                let v = (s * 2_147_483_647.0).clamp(-2_147_483_648.0, 2_147_483_647.0) as i32;
                out.extend_from_slice(&v.to_le_bytes());
            }
        }
        _ => {
            tracing::error!(bit_depth, "Unsupported bit depth, falling back to 16-bit");
            return f32_to_pcm(samples, 16);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn f32_passthrough_when_rates_match() {
        assert!(matches!(
            F32Resampling::new(48000, 48000, 2),
            F32Resampling::Passthrough
        ));
    }

    #[test]
    fn f32_resamples_after_enough_data() {
        let mut r = F32Resampling::new(44100, 48000, 2);
        let mut total = Vec::new();
        for _ in 0..8 {
            let samples: Vec<f32> = (0..512)
                .map(|i| (i as f32 * 440.0 * 2.0 * std::f32::consts::PI / 44100.0).sin() * 0.5)
                .collect();
            if let Some(out) = r.process(&samples) {
                total.extend_from_slice(&out);
            }
        }
        assert!(!total.is_empty());
    }

    #[test]
    fn passthrough_is_exact_identity() {
        let mut r = F32Resampling::new(48000, 48000, 2);
        let buf: Vec<f32> = (0..1000).map(|i| (i as f32 * 0.013).sin() * 0.5).collect();
        assert_eq!(
            r.process(&buf),
            Some(buf.clone()),
            "passthrough is lossless"
        );
        assert_eq!(r.process_or_passthrough(buf.clone()), buf);
    }

    #[test]
    fn buffering_returns_none_until_chunk_full() {
        let mut r = F32Resampling::new(44100, 48000, 2);
        // 512 frames (1024 interleaved samples) < CHUNK_SIZE (1024 frames) → buffering.
        assert_eq!(r.process(&vec![0.1f32; 1024]), None);
    }

    #[test]
    fn downsample_48k_to_24k_roughly_halves_frames() {
        let mut r = F32Resampling::new(48000, 24000, 2);
        let chunks = 32;
        let in_frames = chunks * 1024;
        let mut out = Vec::new();
        for _ in 0..chunks {
            let chunk: Vec<f32> = (0..1024 * 2)
                .map(|i| (i as f32 * 0.02).sin() * 0.5)
                .collect();
            if let Some(o) = r.process(&chunk) {
                out.extend_from_slice(&o);
            }
        }
        let out_frames = out.len() / 2;
        assert!(
            out_frames < in_frames,
            "downsampling reduces the frame count"
        );
        assert!(
            out_frames >= in_frames * 40 / 100 && out_frames <= in_frames * 55 / 100,
            "out {out_frames} ≈ half of {in_frames} (sinc warm-up + trailing buffer)"
        );
        assert!(
            out.iter().all(|s| s.is_finite()),
            "all output samples finite"
        );
    }

    #[test]
    fn f32_to_pcm_16bit() {
        let bytes = f32_to_pcm(&[0.0, 1.0, -1.0, 0.5], 16);
        assert_eq!(bytes.len(), 8);
        assert_eq!(i16::from_le_bytes([bytes[0], bytes[1]]), 0);
        assert_eq!(i16::from_le_bytes([bytes[2], bytes[3]]), 32767);
        assert_eq!(i16::from_le_bytes([bytes[4], bytes[5]]), -32767);
        assert_eq!(i16::from_le_bytes([bytes[6], bytes[7]]), 16383);
    }

    #[test]
    fn f32_to_pcm_24bit() {
        let bytes = f32_to_pcm(&[0.0, 1.0, -1.0], 24);
        assert_eq!(bytes.len(), 9); // 3 bytes per sample
        // Silence
        assert_eq!(&bytes[0..3], &[0, 0, 0]);
        // Max positive: 0x7FFFFF = 8388607
        assert_eq!(&bytes[3..6], &[0xFF, 0xFF, 0x7F]);
    }

    #[test]
    fn f32_to_pcm_32bit() {
        let bytes = f32_to_pcm(&[0.0, 1.0], 32);
        assert_eq!(bytes.len(), 8);
        assert_eq!(
            i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
            0
        );
        assert_eq!(
            i32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]),
            2_147_483_647
        );
    }
}
