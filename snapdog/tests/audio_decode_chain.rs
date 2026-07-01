// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! IT-T60 / IT-NG-07 (partial): a **bit-exact** decode-chain golden for the
//! same-rate path.
//!
//! When the source rate already equals the 48 kHz target, the resampler is a
//! passthrough (no rubato / no sinc), so the whole chain — symphonia FLAC decode →
//! resample (passthrough) → identity EQ — is bit-exact and cross-platform stable:
//! 16-bit FLAC is lossless, `i16 → f32` is an exact power-of-two divide,
//! passthrough returns its input untouched, and a fresh (disabled, band-less) EQ
//! early-returns. The rate-converting arm (sinc, `f64`) is *not* bit-exact and
//! stays on the tolerance/fingerprint track (`IT-NG-07`).
//!
//! Fixture: `tests/fixtures/decode_chain_48k.flac` (48 kHz / 16-bit / stereo /
//! 256 frames), produced by `decode_chain_48k.gen.py`. Golden:
//! `decode_chain_48k.json`. Regenerate the fixture with the script, then the
//! golden with `UPDATE_GOLDEN=1`.

mod common;

use snapdog::audio::eq::ZoneEq;
use snapdog::audio::resample::F32Resampling;
use snapdog::audio::{self, PcmMessage};

#[tokio::test]
async fn flac_48k_decode_chain_is_bit_exact() {
    let flac = common::fixtures_dir().join("decode_chain_48k.flac");
    assert!(flac.exists(), "missing fixture {}", flac.display());

    // symphonia decode is sync + uses blocking_send → run off the async worker.
    let (tx, mut rx) = audio::pcm_channel(256);
    let path = flac.clone();
    let decode = tokio::task::spawn_blocking(move || {
        audio::decode_cached_file(&path, "audio/flac", None, &tx)
    });

    let mut samples: Vec<f32> = Vec::new();
    let mut rate = 0u32;
    let mut channels = 0u16;
    while let Some(msg) = rx.recv().await {
        match msg {
            PcmMessage::Format {
                sample_rate,
                channels: ch,
            } => {
                rate = sample_rate;
                channels = ch;
            }
            PcmMessage::Audio(chunk) => samples.extend_from_slice(&chunk),
            PcmMessage::Error { message, details } => {
                panic!("decode error: {message} ({details:?})")
            }
            _ => {}
        }
    }
    decode.await.expect("join decode task").expect("decode ok");

    // The fixture is 48 kHz / stereo / 256 frames.
    assert_eq!(rate, 48_000, "fixture sample rate");
    assert_eq!(channels, 2, "fixture channels");
    assert_eq!(samples.len(), 256 * 2, "interleaved sample count");
    assert!(
        samples.iter().all(|s| s.is_finite() && s.abs() <= 1.0),
        "decoded samples are valid PCM in [-1, 1]"
    );
    assert!(samples.iter().any(|&s| s != 0.0), "signal is non-silent");

    // Stage 1 — resampling at a matching rate is a bit-exact passthrough (no rubato).
    let decoded = samples.clone();
    let mut resampler = F32Resampling::new(rate, 48_000, channels);
    let resampled = resampler.process_or_passthrough(samples);
    assert_eq!(
        resampled, decoded,
        "48k→48k resample must be an exact passthrough"
    );

    // Stage 2 — a fresh ZoneEq (disabled, no bands) is a bit-exact no-op.
    let mut eq = ZoneEq::new(48_000, channels);
    let mut chained = resampled.clone();
    eq.process(&mut chained);
    assert_eq!(chained, resampled, "identity EQ must not alter samples");

    // Bit-exact golden of the full chain output (UPDATE_GOLDEN=1 regenerates).
    common::assert_json_golden("decode_chain_48k", &chained);
}
