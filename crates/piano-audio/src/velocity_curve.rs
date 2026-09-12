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
/// Measured, not guessed — `engine_tests.rs`'s
/// `a_linear_velocity_map_is_far_from_even_in_decibels` records what A4's
/// early-window RMS does across the velocity range with no curve at all
/// (`exponent = 1.0`):
///
/// | velocity | 0.10 | 0.25 | 0.40 | 0.55 | 0.70 | 0.85 | 1.00 |
/// |---|---|---|---|---|---|---|---|
/// | dB | −34.0 | −22.8 | −17.4 | −16.0 | −15.8 | −14.2 | −11.7 |
///
/// Two-thirds of the available range (0.10 → 0.40) already covers 16.6 of
/// the total 22.3 dB span; the top half (0.55 → 1.00) covers under 4.3 dB —
/// exactly the reported "soft playing collapses into a narrow sliver near
/// silence, and everything past mezzo-forte sounds the same". This is a
/// felt hammer's own doing (`piano_core::hammer`'s stiffening Hertzian
/// contact law is convex in compression), not a bug in the strike, so the
/// compensating curve has to be convex too — a *concave* (`exponent < 1`)
/// curve would only make the collapse worse.
///
/// A sweep of candidate exponents against that same table (least-squares
/// fit of the resulting dB curve to a straight line) put `1.8` at the
/// lowest residual (`2.16` dB) against `1.0`'s own `2.92`, `2.0`'s `2.23`
/// and `2.5`'s `2.61` — the flattest achievable spread of loudness per unit
/// of input velocity a single exponent gets, per the "Work" section of
/// issue #79 ("an exponent, or a small breakpoint table").
pub(crate) const DEFAULT_VELOCITY_CURVE_EXPONENT: f32 = 1.8;

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
            let warped = warp_velocity(velocity, DEFAULT_VELOCITY_CURVE_EXPONENT);
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
