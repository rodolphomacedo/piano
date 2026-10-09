//! Per-key hammer felt — each hammer its own, not one smooth curve (#58).
//!
//! [`hammer_for_frequency`] gives every key the hammer its register calls
//! for: lighter and harder toward the treble, on one smooth curve. A real
//! set of hammers is not that smooth. Felt is pressed from wool, hammers are
//! cut from a single felt strip whose density varies along its length, and
//! every hammer wears and compacts at its own rate with play; a technician
//! voicing a piano spends hours needling and hardening individual hammers
//! precisely because neighbouring keys come out of the factory and out of
//! years of use sounding unequal in attack. H. A. Conklin, "Design and tone
//! in the mechanoacoustic piano. Part I. Piano hammers and tonal effects",
//! JASA 99(6), 1996, describes both the felt-pressing variation and the
//! voicing that exists to even it out; A. Stulov, "Hysteretic model of the
//! grand piano hammer felt", JASA 97(4), 1995, is the felt law the
//! stiffness below feeds.
//!
//! What neither gives is a single figure for "how unequal", which depends
//! on the instrument and on how recently it was voiced.
//! [`DEFAULT_HAMMER_UNEVENNESS`] is therefore this project's reasoned
//! choice, not a measurement, and it is a file setting
//! (`registers.hammer_unevenness`) so it can be turned down to a freshly
//! voiced concert instrument or up to a neglected upright.
//!
//! The variation is deterministic: key `k` always gets the same felt, so a
//! render is reproducible and a player learns the instrument's character
//! instead of hearing it reshuffle every session.

use piano_core::excitation::hammer_for_frequency;
use piano_core::hammer::HammerConfig;
use piano_core::math;

/// How far a hammer's felt strays from its register's curve by default:
/// stiffness and felt brightness each land up to 25% either side.
///
/// Set by measurement, not by literature. Rendering C4–C5 and comparing
/// each key's attack spectral centroid with its smooth-curve twin
/// (`tests/hammer_voicing.rs`): at `0.15` most keys moved under 4%, buried
/// under the ~12% key-to-key spread strike-point combing already gives the
/// smooth curve; at `0.3` they moved by 0–20%, each by its own amount; at
/// `0.5` by up to 42%, a badly worn set. `0.25` sits where every key gets
/// its own attack without any one sounding like a different instrument.
/// The mean shift leans darker, because the brightest strikes already sit
/// near `piano_core::hammer`'s excitation ceiling and can only move down.
pub const DEFAULT_HAMMER_UNEVENNESS: f32 = 0.25;

/// Largest unevenness accepted. At `0.5` a hammer's stiffness can halve or
/// grow by half against its neighbour's — a badly worn set; beyond it a
/// factor of `1 − u` would approach zero stiffness.
pub const MAX_HAMMER_UNEVENNESS: f32 = 0.5;

/// Key `key_index`'s hammer: its register's curve, with stiffness and felt
/// brightness both nudged by the same per-key offset scaled by
/// `unevenness`. Harder felt is both stiffer and brighter, so the two move
/// together. Total: `NaN` or out-of-range `unevenness` clamps into
/// `[0, MAX_HAMMER_UNEVENNESS]`; `0` reproduces the smooth curve exactly.
#[must_use]
pub fn hammer_for_key(key_index: usize, frequency_hz: f32, unevenness: f32) -> HammerConfig {
    let smooth = hammer_for_frequency(frequency_hz);
    let spread = math::clamp_or_low(unevenness, 0.0, MAX_HAMMER_UNEVENNESS);
    let factor = 1.0 + spread * felt_offset(key_index);
    HammerConfig {
        stiffness: smooth.stiffness * factor,
        felt_bandwidth: smooth.felt_bandwidth * factor,
        ..smooth
    }
}

/// A fixed pseudo-random offset in `[-1, 1]` for `key_index` — the
/// "lowbias32" integer hash (C. Wellons, 2018), chosen because consecutive
/// inputs land far apart, so adjacent keys are not correlated.
fn felt_offset(key_index: usize) -> f32 {
    let mut hash = (key_index as u32).wrapping_add(0x9E37_79B9);
    hash ^= hash >> 16;
    hash = hash.wrapping_mul(0x7FEB_352D);
    hash ^= hash >> 15;
    hash = hash.wrapping_mul(0x846C_A68B);
    hash ^= hash >> 16;
    (hash as f32 / u32::MAX as f32) * 2.0 - 1.0
}

#[cfg(test)]
mod tests {
    #![allow(clippy::float_cmp)]

    use super::*;
    use proptest::prelude::*;

    #[test]
    fn zero_unevenness_is_the_smooth_register_curve() {
        for key_index in 0..88 {
            assert_eq!(
                hammer_for_key(key_index, 440.0, 0.0),
                hammer_for_frequency(440.0)
            );
        }
    }

    #[test]
    fn neighbouring_keys_get_different_felt() {
        let stiffness =
            |key_index| hammer_for_key(key_index, 440.0, DEFAULT_HAMMER_UNEVENNESS).stiffness;
        let differing = (0..87)
            .filter(|&key| (stiffness(key) / stiffness(key + 1) - 1.0).abs() > 0.02)
            .count();
        assert!(
            differing > 60,
            "only {differing} of 87 neighbour pairs differ by >2%"
        );
    }

    #[test]
    fn the_offsets_are_spread_across_their_whole_range_and_centred() {
        let offsets: Vec<f32> = (0..88).map(felt_offset).collect();
        let mean = offsets.iter().sum::<f32>() / 88.0;
        assert!(mean.abs() < 0.2, "mean offset {mean}");
        assert!(offsets.iter().any(|o| *o > 0.7) && offsets.iter().any(|o| *o < -0.7));
    }

    #[test]
    fn the_same_key_always_gets_the_same_hammer() {
        assert_eq!(
            hammer_for_key(40, 220.0, 0.3),
            hammer_for_key(40, 220.0, 0.3)
        );
    }

    proptest! {
        #[test]
        fn hammer_for_key_is_total(
            key_index in proptest::num::usize::ANY,
            frequency_hz in proptest::num::f32::ANY,
            unevenness in proptest::num::f32::ANY,
        ) {
            let hammer = hammer_for_key(key_index, frequency_hz, unevenness);
            prop_assert!(hammer.stiffness.is_finite() && hammer.stiffness > 0.0);
            prop_assert!(hammer.felt_bandwidth.is_finite() && hammer.felt_bandwidth > 0.0);
        }
    }
}
