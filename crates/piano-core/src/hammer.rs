//! A nonlinear felt hammer, shaping the excitation instead of a flat burst.
//!
//! Real piano hammers are a lossy nonlinear spring in Hertzian contact with
//! the string: force grows as compression raised to a power `p ≈ 2-3`
//! (A. Chaigne & A. Askenfelt, "Numerical simulations of piano strings, I:
//! A physical model for a struck string using finite difference methods",
//! JASA 95 (1994), §II; the same power-law contact force underlies Hall's
//! earlier hammer papers too). Two consequences follow directly from that
//! nonlinearity and are what this module reproduces:
//!
//! - **Contact gets shorter at higher velocity.** More compression makes the
//!   effective spring stiffer, so it rebounds the hammer faster.
//! - **Contact gets harder at higher velocity.** A shorter pulse spreads its
//!   energy over a wider band — more high-frequency content, not just more
//!   amplitude.
//!
//! # The simplification this makes, stated plainly
//!
//! A full simultaneous hammer/string solve is an implicit problem — the
//! string's own motion feeds back into the contact force — and belongs, if
//! ever built, inside the per-sample loop with a hard-capped fixed-point
//! iteration; `PERF-007` in `docs/PERFORMANCE.md` describes exactly that and
//! why it must stay bounded. This module instead solves only the hammer's
//! side of the contact (the string is treated as immobile during the
//! ~1-4 ms contact window, i.e. no back-reaction), producing a
//! velocity-shaped force envelope that scales the existing excitation noise
//! rather than replacing it. That keeps the excitation broadband — every
//! partial still gets excited, which is what makes a delay-line loop ring at
//! all — while making its *shape*, and therefore its brightness, a function
//! of how hard the key was struck, which scaling a flat-amplitude burst can
//! never achieve regardless of the scale factor.
//!
//! The contact simulation itself is a bounded explicit (semi-implicit Euler)
//! integration, capped at [`MAX_CONTACT_SAMPLES`] steps — a compile-time
//! bound on the loop, never a `while !converged`, per
//! `docs/REALTIME-AUDIO-RULES.md`.

use crate::math;

/// Upper bound on how long a hammer stays in contact with the string.
///
/// ~10.7 ms at 48 kHz — generous against the 1-4 ms Chaigne & Askenfelt
/// report — and small enough that a stack array of this size costs nothing.
/// This is also the hard cap that keeps [`simulate_contact`]'s integration a
/// bounded loop rather than an unbounded one.
pub const MAX_CONTACT_SAMPLES: usize = 512;

/// Hammer felt's Hertzian contact exponent, `F = K·x^p`.
///
/// 2.5 sits in the middle of the 2-3 range Chaigne & Askenfelt measure
/// across the piano's compass (softer/older felt trends toward 2, harder/new
/// felt toward 3).
const CONTACT_EXPONENT: f32 = 2.5;

/// Hertzian contact stiffness, in the model's own normalised units.
///
/// Chosen, together with [`HAMMER_MASS`] and the strike-velocity range
/// below, so the simulated contact duration lands in the 1-4 ms range the
/// literature reports at mid-range velocities — checked empirically against
/// [`simulate_contact`]'s own tests, not derived from a closed form (an
/// exact closed form exists via the Beta function for a lossless power-law
/// spring, but calibrating against the actual simulated output is more
/// honest than trusting an order-of-magnitude hand derivation of it).
const CONTACT_STIFFNESS: f32 = 1.7e9;

/// Hammer mass, normalised to 1. Only the ratio to [`CONTACT_STIFFNESS`]
/// matters for the contact-duration/peak-force behaviour this module
/// produces; this model is not calibrated to a specific real instrument's
/// absolute physical units.
const HAMMER_MASS: f32 = 1.0;

/// Softest strike this model represents, in metres/second.
///
/// Askenfelt & Jansson's hammer-velocity measurements on real pianists put
/// the playable range at roughly 0.5-6 m/s from *pp* to *fff*. `velocity =
/// 0` in the public `[0, 1]` API still gets this nonzero hammer speed
/// because a real ppp keystroke still moves the hammer, it just does so
/// slowly.
const MIN_STRIKE_MPS: f32 = 0.5;

/// Hardest strike this model represents, in metres/second. See
/// [`MIN_STRIKE_MPS`].
const MAX_STRIKE_MPS: f32 = 6.0;

/// Sane lower bound on a usable audio sample rate, in hertz.
///
/// Below this the integration step `1/sample_rate_hz` is large enough that
/// the explicit Euler step can overshoot into a huge or non-finite
/// compression within a handful of iterations — a real risk for
/// pathological inputs (a caller passing a near-zero-but-positive rate,
/// which is finite and `> 0` and so would otherwise slip past the simpler
/// "is this finite and positive" check). No real audio device ever asks
/// for a rate this low, so falling back to [`FALLBACK_SAMPLE_RATE_HZ`] is
/// the total, always-safe choice.
const MIN_SAMPLE_RATE_HZ: f32 = 1_000.0;

/// Sane upper bound on a usable audio sample rate, in hertz. See
/// [`MIN_SAMPLE_RATE_HZ`].
const MAX_SAMPLE_RATE_HZ: f32 = 10_000_000.0;

/// The sample rate [`simulate_contact`] falls back to for any input outside
/// `[MIN_SAMPLE_RATE_HZ, MAX_SAMPLE_RATE_HZ]`.
const FALLBACK_SAMPLE_RATE_HZ: f32 = 48_000.0;

/// Simulates one hammer-felt contact and returns a peak-normalised force
/// envelope plus how many of its samples are active.
///
/// `velocity` is the same `[0, 1]` strike velocity
/// [`PluckedString::pluck`](crate::string::PluckedString::pluck) already
/// takes; `sample_rate_hz` sets the integration step. Both are total: a
/// `sample_rate_hz` outside `[MIN_SAMPLE_RATE_HZ, MAX_SAMPLE_RATE_HZ]` (which
/// covers every real audio device and rejects non-finite values along with
/// the pathologically tiny-but-positive ones that would otherwise blow the
/// integration step up) falls back to a fixed 48 kHz, and
/// `velocity` is clamped into `[0, 1]` the same way `pluck` clamps it. The
/// returned count is always
/// `<= MAX_CONTACT_SAMPLES`; entries at or past it are `0.0` and mean
/// "contact has ended, the hammer has left the string".
#[must_use]
pub fn simulate_contact(velocity: f32, sample_rate_hz: f32) -> ([f32; MAX_CONTACT_SAMPLES], usize) {
    let velocity = math::clamp_or_low(velocity, 0.0, 1.0);
    let sample_rate_hz = if sample_rate_hz.is_finite()
        && (MIN_SAMPLE_RATE_HZ..=MAX_SAMPLE_RATE_HZ).contains(&sample_rate_hz)
    {
        sample_rate_hz
    } else {
        FALLBACK_SAMPLE_RATE_HZ
    };
    let strike_mps = MIN_STRIKE_MPS + velocity * (MAX_STRIKE_MPS - MIN_STRIKE_MPS);
    let dt = 1.0 / sample_rate_hz;

    let mut force = [0.0f32; MAX_CONTACT_SAMPLES];
    let mut compression = 0.0f32;
    let mut hammer_velocity = strike_mps;
    let mut active = 0usize;
    let mut peak = 0.0f32;

    for sample in &mut force {
        if compression <= 0.0 && active > 0 {
            break;
        }
        let restoring = CONTACT_STIFFNESS * math::powf(compression, CONTACT_EXPONENT) / HAMMER_MASS;
        hammer_velocity -= restoring * dt;
        compression = (compression + hammer_velocity * dt).max(0.0);
        let applied_force = CONTACT_STIFFNESS * math::powf(compression, CONTACT_EXPONENT);
        *sample = applied_force;
        peak = peak.max(applied_force);
        active += 1;
    }

    if peak > f32::EPSILON {
        for sample in force.iter_mut().take(active) {
            *sample /= peak;
        }
    }
    (force, active)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use proptest::prelude::*;

    use super::*;

    #[test]
    fn harder_strikes_produce_shorter_contact() {
        let (_, soft_samples) = simulate_contact(0.1, 48_000.0);
        let (_, hard_samples) = simulate_contact(0.9, 48_000.0);
        assert!(
            hard_samples < soft_samples,
            "hard {hard_samples} soft {soft_samples}"
        );
    }

    #[test]
    fn contact_duration_is_physically_plausible() {
        // Chaigne & Askenfelt (1994) report contact durations of roughly
        // 1-4 ms across normal playing velocities. Checked with a generous
        // margin, since this model is not calibrated to a specific
        // instrument.
        let sample_rate = 48_000.0;
        for velocity in [0.0, 0.5, 1.0] {
            let (_, samples) = simulate_contact(velocity, sample_rate);
            let milliseconds = samples as f32 / sample_rate * 1_000.0;
            assert!(
                (0.1..20.0).contains(&milliseconds),
                "velocity {velocity}: {milliseconds} ms is not plausible"
            );
        }
    }

    #[test]
    fn the_force_envelope_peaks_at_one() {
        let (force, active) = simulate_contact(0.7, 48_000.0);
        let peak = force.iter().take(active).copied().fold(0.0f32, f32::max);
        assert!((peak - 1.0).abs() < 1e-6, "peak {peak}");
    }

    #[test]
    fn samples_past_contact_are_silent() {
        let (force, active) = simulate_contact(0.5, 48_000.0);
        assert!(force.iter().skip(active).all(|sample| *sample == 0.0));
    }

    proptest! {
        /// The contact simulation must never panic, loop unboundedly or
        /// produce a non-finite value, for every reachable velocity and
        /// sample rate — including NaN, +-infinity and zero.
        #[test]
        fn simulate_contact_is_total(
            velocity in proptest::num::f32::ANY,
            sample_rate in proptest::num::f32::ANY,
        ) {
            let (force, active) = simulate_contact(velocity, sample_rate);
            prop_assert!(active <= MAX_CONTACT_SAMPLES);
            prop_assert!(force.iter().all(|sample| sample.is_finite()));
            prop_assert!(force.iter().all(|sample| (0.0..=1.0).contains(sample)));
        }
    }
}
