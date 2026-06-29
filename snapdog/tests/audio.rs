// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! Audio pipeline contract (IT-T60): golden F32→PCM conversion at the Snapcast
//! output boundary. Deterministic, pure-function — no fixtures needed.

use snapdog::audio::resample::f32_to_pcm;

#[test]
fn pcm16_silence_is_all_zeros() {
    assert_eq!(f32_to_pcm(&[0.0, 0.0], 16), vec![0u8, 0, 0, 0]);
}

#[test]
fn pcm16_full_scale_endpoints() {
    assert_eq!(f32_to_pcm(&[1.0], 16), 32767i16.to_le_bytes().to_vec());
    assert_eq!(f32_to_pcm(&[-1.0], 16), (-32767i16).to_le_bytes().to_vec());
}

#[test]
fn pcm16_clamps_out_of_range() {
    assert_eq!(f32_to_pcm(&[2.0], 16), 32767i16.to_le_bytes().to_vec());
    assert_eq!(f32_to_pcm(&[-2.0], 16), (-32768i16).to_le_bytes().to_vec());
}

#[test]
fn pcm_byte_lengths_per_bit_depth() {
    assert_eq!(
        f32_to_pcm(&[0.0; 4], 16).len(),
        8,
        "16-bit = 2 bytes/sample"
    );
    assert_eq!(
        f32_to_pcm(&[0.0; 4], 24).len(),
        12,
        "24-bit = 3 bytes/sample"
    );
    assert_eq!(
        f32_to_pcm(&[0.0; 4], 32).len(),
        16,
        "32-bit = 4 bytes/sample"
    );
}

#[test]
fn pcm24_silence_is_three_zero_bytes() {
    assert_eq!(f32_to_pcm(&[0.0], 24), vec![0u8, 0, 0]);
}

#[test]
fn unsupported_bit_depth_falls_back_to_16() {
    // Falls back to 16-bit (2 bytes/sample) rather than panicking.
    assert_eq!(f32_to_pcm(&[0.0; 3], 8).len(), 6);
}
