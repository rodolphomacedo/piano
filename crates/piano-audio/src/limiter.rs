//! The output mix bus's headroom safety net.
//!
//! [`Engine::process_chunk`](crate::engine::Engine) sums every ringing voice
//! plus the soundboard additively, with nothing before this module capping
//! the result. See [`soft_limit`]'s own doc comment for why that is a real
//! gap, not a hypothetical one.

use piano_core::math;

/// Magnitude below which [`soft_limit`] leaves a sample untouched.
///
/// Chosen for headroom, not modelled on anything physical: a single freshly
/// struck voice never gets close to it (M4's hammer-excitation fix already
/// keeps one string's own peak well under `1.0`), so ordinary play is
/// bit-identical to having no limiter at all. It only starts doing anything
/// once enough simultaneously ringing voices — a chord, the sustain pedal's
/// sympathetic resonance, `PERF-008`'s bridge bus — sum past it.
pub(crate) const OUTPUT_LIMITER_THRESHOLD: f32 = 0.9;

/// Softly caps `sample`'s magnitude to `1.0`, transparent below `threshold`.
///
/// `docs/REALTIME-AUDIO-RULES.md` states this project's output is "bounded
/// by construction", but nothing enforced that once more than one voice
/// could be ringing at once: summing every voice plus the soundboard has no
/// ceiling of its own, so a full chord or a sustain-pedal buildup could sum
/// past `±1.0` and hit whatever hard-clipping or wraparound the host's
/// `f32 -> PCM` conversion does — audibly harsh, reported alongside the
/// too-fast decay this same change fixes. Below `threshold` this is the
/// identity function, so a single voice's own render is unaffected; only
/// the excess is compressed, smoothly, into the remaining headroom up to
/// `1.0` — a standard soft-knee limiter, not a physical model, so it needs
/// no literature citation the way `piano-core`'s DSP does.
///
/// Total for every input: a non-finite `sample` maps to silence rather than
/// propagating a `NaN` or `±∞` into the audio device.
#[inline]
pub(crate) fn soft_limit(sample: f32, threshold: f32) -> f32 {
    if !sample.is_finite() {
        return 0.0;
    }
    let threshold = math::clamp_or_low(threshold, 0.0, 0.999);
    let magnitude = math::abs(sample);
    if magnitude <= threshold {
        return sample;
    }
    let headroom = 1.0 - threshold;
    let excess = magnitude - threshold;
    let saturated = threshold + headroom * math::tanh(excess / headroom);
    if sample < 0.0 { -saturated } else { saturated }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::float_cmp)]

    use super::*;

    #[test]
    fn leaves_a_signal_below_threshold_untouched() {
        assert_eq!(soft_limit(0.5, OUTPUT_LIMITER_THRESHOLD), 0.5);
        assert_eq!(soft_limit(-0.5, OUTPUT_LIMITER_THRESHOLD), -0.5);
    }

    #[test]
    fn compresses_a_signal_above_threshold_but_keeps_its_sign() {
        let limited = soft_limit(1.05, OUTPUT_LIMITER_THRESHOLD);
        assert!(limited > OUTPUT_LIMITER_THRESHOLD && limited < 1.0);
        let limited_negative = soft_limit(-1.05, OUTPUT_LIMITER_THRESHOLD);
        assert!((limited_negative + limited).abs() < 1e-6);
    }

    #[test]
    fn never_exceeds_full_scale_no_matter_how_loud_the_input() {
        assert!(soft_limit(1_000.0, OUTPUT_LIMITER_THRESHOLD) <= 1.0);
        assert!(soft_limit(f32::MAX, OUTPUT_LIMITER_THRESHOLD) <= 1.0);
    }

    #[test]
    fn non_finite_input_maps_to_silence() {
        assert_eq!(soft_limit(f32::NAN, OUTPUT_LIMITER_THRESHOLD), 0.0);
        assert_eq!(soft_limit(f32::INFINITY, OUTPUT_LIMITER_THRESHOLD), 0.0);
        assert_eq!(soft_limit(f32::NEG_INFINITY, OUTPUT_LIMITER_THRESHOLD), 0.0);
    }

    /// Not a full property test (this crate has no `proptest` dependency
    /// today, unlike `piano-core`) — a fixed sweep across the pathological
    /// values `f32` offers, covering the same totality contract: never
    /// panics, never returns non-finite, never exceeds unit magnitude.
    #[test]
    fn soft_limit_is_total_across_pathological_inputs() {
        let samples = [
            0.0,
            -0.0,
            1.0,
            -1.0,
            f32::MIN,
            f32::MAX,
            f32::EPSILON,
            f32::NAN,
            f32::INFINITY,
            f32::NEG_INFINITY,
        ];
        let thresholds = [0.0, 0.9, 1.0, -1.0, 5.0, f32::NAN, f32::INFINITY];
        for &sample in &samples {
            for &threshold in &thresholds {
                let limited = soft_limit(sample, threshold);
                assert!(
                    limited.is_finite(),
                    "sample {sample} threshold {threshold} gave non-finite {limited}"
                );
                assert!(
                    limited.abs() <= 1.0,
                    "sample {sample} threshold {threshold} gave {limited} past unit magnitude"
                );
            }
        }
    }
}
