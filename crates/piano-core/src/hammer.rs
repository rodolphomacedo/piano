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

/// Widest [`HammerConfig::contact_exponent`] `sanitize_hammer` allows.
///
/// Bounded, not just required positive, because [`simulate_contact`]'s
/// integration must stay provably finite for *any* live-set value (this
/// project's hot-path totality rule) — an unbounded exponent combined with
/// compression above 1 can overflow `powf` to infinity. 0.5-8.0 comfortably
/// covers the 2-3 real felt range Chaigne & Askenfelt report plus enough
/// slack either side for a deliberately exaggerated, non-physical voicing.
const MIN_CONTACT_EXPONENT: f32 = 0.5;
/// See [`MIN_CONTACT_EXPONENT`].
const MAX_CONTACT_EXPONENT: f32 = 8.0;

/// Widest [`HammerConfig::stiffness`] `sanitize_hammer` allows — two orders
/// of magnitude either side of [`DEFAULT_HAMMER`]'s value, chosen together
/// with [`MAX_COMPRESSION`], [`MIN_MASS`] and [`MAX_CONTACT_EXPONENT`] so
/// `simulate_contact`'s per-step force can never exceed `f32`'s range (see
/// the derivation in that function's doc comment).
const MIN_STIFFNESS: f32 = 1.7e7;
/// See [`MIN_STIFFNESS`].
const MAX_STIFFNESS: f32 = 1.7e11;

/// Widest [`HammerConfig::mass`] `sanitize_hammer` allows.
///
/// Bounded away from zero because mass is a divisor in `simulate_contact`'s
/// restoring-force step — an unbounded-small mass blows the force up the
/// same way an unbounded-large exponent does.
const MIN_MASS: f32 = 0.01;
/// See [`MIN_MASS`].
const MAX_MASS: f32 = 100.0;

/// Hard cap on the contact simulation's compression state, in the model's
/// own normalised units.
///
/// Applied every integration step regardless of [`HammerConfig`], on top of
/// the field-level bounds above: those bounds alone keep any *single* step's
/// force finite, but only re-clamping compression itself, every step, stops
/// an unstable (numerically stiff) combination of extreme stiffness, mass
/// and sample rate from growing compression — and so the force derived from
/// it — without bound across [`MAX_CONTACT_SAMPLES`] steps. Ten sits far
/// above where a real strike's compression ever settles at the default
/// [`HammerConfig`] (checked empirically, the same way [`CONTACT_STIFFNESS`]
/// below was), so this cap is inert for realistic voicings.
const MAX_COMPRESSION: f32 = 10.0;

/// Tunable felt-contact physics for one string's hammer.
///
/// Everything here feeds [`simulate_contact`]; see that function and the
/// module docs for what each field does physically. `sanitize_hammer`
/// bounds every field before use, so `simulate_contact` stays total for any
/// `HammerConfig` a caller constructs directly — its fields are `pub` and
/// not validated at construction, the same pattern `StringConfig`'s
/// `damping`/`sustain`/`inharmonicity` already use.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HammerConfig {
    /// Hertzian contact exponent, `F = K·x^p`. See [`CONTACT_EXPONENT`]'s
    /// doc comment for the physical range this represents.
    pub contact_exponent: f32,
    /// Hertzian contact stiffness, in the model's own normalised units.
    pub stiffness: f32,
    /// Hammer mass, normalised to 1 at the default.
    pub mass: f32,
}

/// [`HammerConfig`] matching this module's original, single shared
/// (pre-M15) constants — every string used exactly these values before the
/// live parameter studio made hammer physics per-string.
pub const DEFAULT_HAMMER: HammerConfig = HammerConfig {
    contact_exponent: CONTACT_EXPONENT,
    stiffness: CONTACT_STIFFNESS,
    mass: HAMMER_MASS,
};

/// Hammer felt's Hertzian contact exponent, `F = K·x^p`, at [`DEFAULT_HAMMER`].
///
/// 2.5 sits in the middle of the 2-3 range Chaigne & Askenfelt measure
/// across the piano's compass (softer/older felt trends toward 2, harder/new
/// felt toward 3).
const CONTACT_EXPONENT: f32 = 2.5;

/// Hertzian contact stiffness at [`DEFAULT_HAMMER`], in the model's own
/// normalised units.
///
/// Chosen, together with [`HAMMER_MASS`] and the strike-velocity range
/// below, so the simulated contact duration lands in the 1-4 ms range the
/// literature reports at mid-range velocities — checked empirically against
/// [`simulate_contact`]'s own tests, not derived from a closed form (an
/// exact closed form exists via the Beta function for a lossless power-law
/// spring, but calibrating against the actual simulated output is more
/// honest than trusting an order-of-magnitude hand derivation of it).
const CONTACT_STIFFNESS: f32 = 1.7e9;

/// Hammer mass at [`DEFAULT_HAMMER`], normalised to 1. Only the ratio to
/// [`CONTACT_STIFFNESS`] matters for the contact-duration/peak-force
/// behaviour this module produces; this model is not calibrated to a
/// specific real instrument's absolute physical units.
const HAMMER_MASS: f32 = 1.0;

/// Clamps `hammer`'s fields into the bounds `simulate_contact` needs to stay
/// total — see [`MIN_CONTACT_EXPONENT`], [`MIN_STIFFNESS`] and [`MIN_MASS`].
/// `NaN` maps to each bound's low end, the same convention
/// [`crate::math::clamp_or_low`] uses everywhere else in this crate.
fn sanitize_hammer(hammer: HammerConfig) -> HammerConfig {
    HammerConfig {
        contact_exponent: math::clamp_or_low(
            hammer.contact_exponent,
            MIN_CONTACT_EXPONENT,
            MAX_CONTACT_EXPONENT,
        ),
        stiffness: math::clamp_or_low(hammer.stiffness, MIN_STIFFNESS, MAX_STIFFNESS),
        mass: math::clamp_or_low(hammer.mass, MIN_MASS, MAX_MASS),
    }
}

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
/// takes; `sample_rate_hz` sets the integration step; `hammer` is this
/// string's own felt-contact physics (`sanitize_hammer` bounds it first, so
/// any `HammerConfig` a caller constructs is safe to pass here). All three
/// are total: a `sample_rate_hz` outside
/// `[MIN_SAMPLE_RATE_HZ, MAX_SAMPLE_RATE_HZ]` (which covers every real audio
/// device and rejects non-finite values along with the pathologically
/// tiny-but-positive ones that would otherwise blow the integration step up)
/// falls back to a fixed 48 kHz, `velocity` is clamped into `[0, 1]` the same
/// way `pluck` clamps it, and `hammer`'s fields are bounded by
/// `sanitize_hammer`. The per-step compression state is additionally capped
/// at [`MAX_COMPRESSION`] on every iteration: the field-level bounds alone
/// keep any *single* step's force finite, but only re-clamping compression
/// itself stops an unstable (numerically stiff) combination of extreme
/// `hammer` values and sample rate from growing without bound across
/// [`MAX_CONTACT_SAMPLES`] steps — together these two guarantees are what
/// keeps every returned sample finite and in `[0, 1]` for any input,
/// verified by `simulate_contact_is_total` rather than argued for. The
/// returned count is always `<= MAX_CONTACT_SAMPLES`; entries at or past it
/// are `0.0` and mean "contact has ended, the hammer has left the string".
#[must_use]
pub fn simulate_contact(
    velocity: f32,
    sample_rate_hz: f32,
    hammer: HammerConfig,
) -> ([f32; MAX_CONTACT_SAMPLES], usize) {
    let velocity = math::clamp_or_low(velocity, 0.0, 1.0);
    let sample_rate_hz = if sample_rate_hz.is_finite()
        && (MIN_SAMPLE_RATE_HZ..=MAX_SAMPLE_RATE_HZ).contains(&sample_rate_hz)
    {
        sample_rate_hz
    } else {
        FALLBACK_SAMPLE_RATE_HZ
    };
    let hammer = sanitize_hammer(hammer);
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
        let restoring =
            hammer.stiffness * math::powf(compression, hammer.contact_exponent) / hammer.mass;
        hammer_velocity -= restoring * dt;
        compression = math::clamp_or_low(compression + hammer_velocity * dt, 0.0, MAX_COMPRESSION);
        let applied_force = hammer.stiffness * math::powf(compression, hammer.contact_exponent);
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
        let (_, soft_samples) = simulate_contact(0.1, 48_000.0, DEFAULT_HAMMER);
        let (_, hard_samples) = simulate_contact(0.9, 48_000.0, DEFAULT_HAMMER);
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
            let (_, samples) = simulate_contact(velocity, sample_rate, DEFAULT_HAMMER);
            let milliseconds = samples as f32 / sample_rate * 1_000.0;
            assert!(
                (0.1..20.0).contains(&milliseconds),
                "velocity {velocity}: {milliseconds} ms is not plausible"
            );
        }
    }

    #[test]
    fn the_force_envelope_peaks_at_one() {
        let (force, active) = simulate_contact(0.7, 48_000.0, DEFAULT_HAMMER);
        let peak = force.iter().take(active).copied().fold(0.0f32, f32::max);
        assert!((peak - 1.0).abs() < 1e-6, "peak {peak}");
    }

    #[test]
    fn samples_past_contact_are_silent() {
        let (force, active) = simulate_contact(0.5, 48_000.0, DEFAULT_HAMMER);
        assert!(force.iter().skip(active).all(|sample| *sample == 0.0));
    }

    #[test]
    fn a_stiffer_hammer_produces_a_shorter_contact() {
        let soft = HammerConfig {
            stiffness: MIN_STIFFNESS,
            ..DEFAULT_HAMMER
        };
        let stiff = HammerConfig {
            stiffness: MAX_STIFFNESS,
            ..DEFAULT_HAMMER
        };
        let (_, soft_samples) = simulate_contact(0.7, 48_000.0, soft);
        let (_, stiff_samples) = simulate_contact(0.7, 48_000.0, stiff);
        assert!(
            stiff_samples < soft_samples,
            "stiff {stiff_samples} soft {soft_samples}"
        );
    }

    proptest! {
        /// The contact simulation must never panic, loop unboundedly or
        /// produce a non-finite value, for every reachable velocity, sample
        /// rate and hammer configuration — including NaN, +-infinity and
        /// zero in every field.
        #[test]
        fn simulate_contact_is_total(
            velocity in proptest::num::f32::ANY,
            sample_rate in proptest::num::f32::ANY,
            contact_exponent in proptest::num::f32::ANY,
            stiffness in proptest::num::f32::ANY,
            mass in proptest::num::f32::ANY,
        ) {
            let hammer = HammerConfig { contact_exponent, stiffness, mass };
            let (force, active) = simulate_contact(velocity, sample_rate, hammer);
            prop_assert!(active <= MAX_CONTACT_SAMPLES);
            prop_assert!(force.iter().all(|sample| sample.is_finite()));
            prop_assert!(force.iter().all(|sample| (0.0..=1.0).contains(sample)));
        }
    }
}
