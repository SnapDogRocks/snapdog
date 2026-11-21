// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! Channel mapping between a source and the zone output.
//!
//! The zone streams with the configured channel count. A source with another count is
//! mapped before resampling and EQ, so that everything downstream sees whole output frames.

/// Gain for a centre or surround channel folded into a front channel (−3 dB).
const FOLD_GAIN: f32 = std::f32::consts::FRAC_1_SQRT_2;

/// Map interleaved samples with `from` channels to `to` channels.
///
/// - Same count: unchanged.
/// - To mono: the average of all channels.
/// - Mono to more: the signal on the two front channels, any further channel silent.
/// - More than two to stereo: a downmix in the common layout (front left, front right,
///   centre, LFE, then surround pairs). The centre goes to both sides at −3 dB, each surround
///   channel to its side at −3 dB, the LFE is left out. Each side is normalised so that it
///   cannot clip.
/// - Other combinations: the first channels are kept, missing ones stay silent.
///
/// A trailing partial frame is dropped.
#[must_use]
pub fn map_channels(samples: Vec<f32>, from: u16, to: u16) -> Vec<f32> {
    let (from, to) = (usize::from(from), usize::from(to));
    if from == to || from == 0 || to == 0 {
        return samples;
    }
    let frames = samples.len() / from;
    let mut out = Vec::with_capacity(frames * to);
    for frame in samples.chunks_exact(from) {
        if to == 1 {
            #[allow(clippy::cast_precision_loss)]
            out.push(frame.iter().sum::<f32>() / from as f32);
        } else if from == 1 {
            out.extend_from_slice(&[frame[0], frame[0]]);
            out.extend(std::iter::repeat_n(0.0, to - 2));
        } else if from > 2 && to == 2 {
            out.extend_from_slice(&downmix_to_stereo(frame));
        } else {
            let keep = from.min(to);
            out.extend_from_slice(&frame[..keep]);
            out.extend(std::iter::repeat_n(0.0, to - keep));
        }
    }
    out
}

/// Fold one frame of three or more channels into a left and a right sample.
fn downmix_to_stereo(frame: &[f32]) -> [f32; 2] {
    let (mut left, mut right) = (frame[0], frame[1]);
    let (mut left_gain, mut right_gain) = (1.0, 1.0);
    if let Some(&centre) = frame.get(2) {
        left = centre.mul_add(FOLD_GAIN, left);
        right = centre.mul_add(FOLD_GAIN, right);
        left_gain += FOLD_GAIN;
        right_gain += FOLD_GAIN;
    }
    // Index 3 is the LFE and is left out. Surround channels follow in left/right pairs.
    for (i, &sample) in frame.iter().enumerate().skip(4) {
        if i % 2 == 0 {
            left = sample.mul_add(FOLD_GAIN, left);
            left_gain += FOLD_GAIN;
        } else {
            right = sample.mul_add(FOLD_GAIN, right);
            right_gain += FOLD_GAIN;
        }
    }
    [left / left_gain, right / right_gain]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: &[f32], b: &[f32]) -> bool {
        a.len() == b.len() && a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-6)
    }

    #[test]
    fn mono_to_stereo_duplicates_each_sample_and_keeps_the_frame_count() {
        let out = map_channels(vec![0.1, 0.2, 0.3], 1, 2);
        assert!(close(&out, &[0.1, 0.1, 0.2, 0.2, 0.3, 0.3]));
        assert_eq!(out.len() / 2, 3, "three frames in, three frames out");
    }

    #[test]
    fn stereo_to_mono_averages_both_sides() {
        let out = map_channels(vec![0.2, 0.4, -0.5, 0.5], 2, 1);
        assert!(close(&out, &[0.3, 0.0]));
    }

    #[test]
    #[allow(clippy::suboptimal_flops)] // the expected values are written as the formula reads
    fn five_one_to_stereo_folds_centre_and_surround_and_drops_the_lfe() {
        // FL FR C LFE SL SR
        let frame = [0.4, 0.2, 0.5, 1.0, 0.3, 0.1];
        let out = map_channels(frame.repeat(2), 6, 2);
        assert_eq!(out.len(), 4, "two frames of stereo");
        let g = FOLD_GAIN;
        let left = (0.4 + 0.5 * g + 0.3 * g) / (1.0 + 2.0 * g);
        let right = (0.2 + 0.5 * g + 0.1 * g) / (1.0 + 2.0 * g);
        assert!(close(&out[..2], &[left, right]));
    }

    #[test]
    fn a_full_scale_downmix_does_not_clip() {
        let out = map_channels(vec![1.0; 6], 6, 2);
        assert!(out.iter().all(|s| s.abs() <= 1.0 + 1e-6));
    }

    #[test]
    fn same_count_is_unchanged_and_a_partial_frame_is_dropped() {
        assert_eq!(map_channels(vec![1.0, 2.0, 3.0], 2, 2), vec![1.0, 2.0, 3.0]);
        assert_eq!(map_channels(vec![1.0, 2.0, 3.0], 2, 1).len(), 1);
    }

    #[test]
    fn stereo_to_more_channels_leaves_the_extra_ones_silent() {
        assert!(close(
            &map_channels(vec![0.1, 0.2], 2, 4),
            &[0.1, 0.2, 0.0, 0.0]
        ));
    }
}
