//! Maps a strike's incoming `[0, 1]` velocity (MIDI, computer keyboard or a
//! studio slider — see `docs/PARAMETER-STUDIO.md`) onto the velocity
//! [`piano_core::UnisonGroup::pluck`] actually receives (issue #79).
//!
//! `Engine::note_on` fed `pluck` its `velocity` argument unchanged: a linear
//! map. [`warp_velocity`] is what closes that gap.

use piano_core::math;

/// Default exponent [`warp_velocity`] applies: `pluck_velocity =
/// velocity ^ this`.
///
/// `1.0`, the identity: a real action moves the hammer roughly in
/// proportion to the key, and the strike now turns hammer velocity into
/// sound the way a hammer does — low-frequency drive in proportion to its
/// momentum, brightness on top from the shorter contact (see
/// `piano_core::string::low_frequency_weight`). Measured at A4 with this
/// exponent:
///
/// | velocity | 0.10 | 0.25 | 0.40 | 0.55 | 0.70 | 0.85 | 1.00 |
/// |---|---|---|---|---|---|---|---|
/// | dB | −39.0 | −28.1 | −21.2 | −16.3 | −12.4 | −9.2 | −6.9 |
///
/// A 32 dB span with every step louder, inside the 30-40 dB a piano's
/// fundamental covers from *pianissimo* to *fortissimo*. Issue #79's `1.8`
/// compensated for the excitation it replaced, which normalised every
/// strike's slope to unit peak and so gave a harder, shorter strike *less*
/// low-frequency drive: A4 put 16.6 of its 22.3 dB in the bottom third of
/// the range, and A6 stopped getting louder at all past *mezzo-forte*. Kept
/// adjustable for a player whose controller's velocity response differs.
pub const DEFAULT_VELOCITY_CURVE_EXPONENT: f32 = 1.0;

/// Floor `Engine::set_velocity_curve_exponent` clamps into. At or below `0`
/// `velocity.powf(exponent)` degenerates (a constant `1.0`, or a division
/// by zero for a negative exponent at `velocity = 0`); `0.25` — a
/// pronounced *concave* curve, for a caller who wants soft touches boosted
/// rather than spread out — is the most such a curve is ever useful before
/// it stops resembling a piano at all.
pub(crate) const MIN_VELOCITY_CURVE_EXPONENT: f32 = 0.25;

/// Ceiling `Engine::set_velocity_curve_exponent` clamps into. Beyond `6.0`
/// nearly the entire velocity range plucks at a near-silent strike velocity
/// — generous headroom for an unusually stiff action without handing over
/// "every key sounds like it was barely touched".
pub(crate) const MAX_VELOCITY_CURVE_EXPONENT: f32 = 6.0;

/// Warps `velocity` through `exponent`: `velocity.clamp(0, 1) ^
/// exponent.clamp(MIN, MAX)`.
///
/// Total for every input. `velocity` is clamped into `[0, 1]` first — a
/// `NaN` or an out-of-range value lands on a bound rather than reaching
/// `powf` — and `exponent`'s clamp keeps it strictly positive, so the base
/// is always non-negative and the exponent always finite and positive:
/// `powf` under those conditions cannot itself return a `NaN`.
#[inline]
pub(crate) fn warp_velocity(velocity: f32, exponent: f32) -> f32 {
    let velocity = math::clamp_or_low(velocity, 0.0, 1.0);
    let exponent = math::clamp_or_low(
        exponent,
        MIN_VELOCITY_CURVE_EXPONENT,
        MAX_VELOCITY_CURVE_EXPONENT,
    );
    math::clamp_or_low(math::powf(velocity, exponent), 0.0, 1.0)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::float_cmp)]

    use super::*;

    #[test]
    fn a_linear_map_is_the_identity() {
        for velocity in [0.0_f32, 0.25, 0.5, 0.75, 1.0] {
            assert_eq!(warp_velocity(velocity, 1.0), velocity);
        }
    }

    #[test]
    fn the_endpoints_are_fixed_for_any_exponent() {
        for exponent in [
            MIN_VELOCITY_CURVE_EXPONENT,
            1.0,
            DEFAULT_VELOCITY_CURVE_EXPONENT,
            MAX_VELOCITY_CURVE_EXPONENT,
        ] {
            assert_eq!(warp_velocity(0.0, exponent), 0.0);
            assert_eq!(warp_velocity(1.0, exponent), 1.0);
        }
    }

    #[test]
    fn an_exponent_above_one_pulls_every_interior_velocity_down() {
        for velocity in [0.1_f32, 0.25, 0.5, 0.75, 0.9] {
            let warped = warp_velocity(velocity, 1.8);
            assert!(
                warped < velocity,
                "velocity {velocity} warped to {warped}, expected below the linear map"
            );
        }
    }

    #[test]
    fn warping_stays_monotonically_increasing() {
        let mut previous = warp_velocity(0.0, DEFAULT_VELOCITY_CURVE_EXPONENT);
        for tenth in 1..=10 {
            let velocity = tenth as f32 / 10.0;
            let warped = warp_velocity(velocity, DEFAULT_VELOCITY_CURVE_EXPONENT);
            assert!(
                warped > previous,
                "velocity {velocity} did not warp to more than the previous step"
            );
            previous = warped;
        }
    }

    #[test]
    fn out_of_range_exponents_fall_back_to_a_bound_rather_than_degenerating() {
        assert_eq!(
            warp_velocity(0.5, 0.0),
            warp_velocity(0.5, MIN_VELOCITY_CURVE_EXPONENT)
        );
        assert_eq!(
            warp_velocity(0.5, -3.0),
            warp_velocity(0.5, MIN_VELOCITY_CURVE_EXPONENT)
        );
        assert_eq!(
            warp_velocity(0.5, f32::NAN),
            warp_velocity(0.5, MIN_VELOCITY_CURVE_EXPONENT)
        );
    }

    /// Not a full property test (this crate has no `proptest` dependency
    /// today, unlike `piano-core`) — a fixed sweep across the pathological
    /// values `f32` offers, covering the same totality contract `limiter`'s
    /// own `soft_limit_is_total_across_pathological_inputs` does: never
    /// panics, never returns non-finite, never leaves `[0, 1]`.
    #[test]
    fn warp_velocity_is_total_across_pathological_inputs() {
        let velocities = [
            0.0,
            -0.0,
            1.0,
            -1.0,
            2.0,
            f32::MIN,
            f32::MAX,
            f32::EPSILON,
            f32::NAN,
            f32::INFINITY,
            f32::NEG_INFINITY,
        ];
        let exponents = [0.0, -1.0, 1.0, 1.8, 6.0, 100.0, f32::NAN, f32::INFINITY];
        for &velocity in &velocities {
            for &exponent in &exponents {
                let warped = warp_velocity(velocity, exponent);
                assert!(
                    warped.is_finite(),
                    "velocity {velocity} exponent {exponent} gave non-finite {warped}"
                );
                assert!(
                    (0.0..=1.0).contains(&warped),
                    "velocity {velocity} exponent {exponent} gave {warped} outside [0, 1]"
                );
            }
        }
    }
}
