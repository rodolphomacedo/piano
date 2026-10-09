//! The spectral shaping a hammer strike passes through before it enters the
//! string: the felt's own velocity-dependent lowpass, then the lowpass the
//! hammer's *mass* imposes in the treble, where it stays on the string for
//! longer than one period.
//!
//! # Why the treble needs a second filter
//!
//! A. Askenfelt & E. Jansson, "From touch to string vibrations" (JASA 88(1),
//! 1990, and parts II-III, 1991), measure hammer-string contact durations of
//! roughly 4 ms in the low bass falling to well under 1 ms at the top of the
//! compass. A bass string's period is many times longer than that, so the
//! hammer has left before the first reflection returns. From about C6 upward
//! the contact outlasts the period: the hammer is still pressed against the
//! string while several round trips pass under it, and its mass loads every
//! one of them. A mass on a string reflects high frequencies and passes low
//! ones, so the upper partials of a treble note are barely excited — which is
//! why the top two octaves of a real piano are dominated by their
//! fundamental, and why a treble note excited with the same broadband burst
//! as the middle sounds glassy and shrill (before this filter, E6's second
//! partial measured 6 dB *above* its fundamental at the attack).
//!
//! This model does not simulate that loading — it would need the full
//! finite-difference hammer-string interaction (issue #55). It reproduces its
//! *spectral consequence* with a lowpass whose corner, in partials, falls as
//! the contact-to-period ratio `r = τ·f0` grows: `corner = f0·P·r⁻³`.
//! [`LOADING_CORNER_PARTIALS`] and [`LOADING_EXPONENT`] are calibrated by
//! measurement, the same way `hammer::EXCITATION_BANDWIDTH_FACTOR` was: the
//! corner sits far above audibility in the bass, near 3.8 kHz at A4 (barely
//! touching it), near 1.8 kHz at C6 and on the fundamental itself from
//! about E6 up ([`MIN_CORNER_PARTIALS`]). The
//! filter is normalised to unit gain at the fundamental, so it changes the
//! *balance* of a note's partials, not how loud the note is.

use crate::filter::OnePoleLowpass;
use crate::hammer::{DEFAULT_HAMMER, HammerConfig};
use crate::math;

/// Identical one-pole sections in the felt's velocity-dependent lowpass.
/// Two, for 12 dB/octave: one is measurably too gentle (see
/// `PluckedString::write_excitation`); three and beyond start eating the
/// attack's audible transient along with the click.
pub(crate) const FELT_POLES: usize = 2;

/// Identical one-pole sections in the mass-loading lowpass. Three put the
/// second partial of a key cornered at its own fundamental 12 dB under it
/// — measured, A7's went from -6 dB to -10 dB and C8's below -30 dB — where
/// two leave it 8 dB under.
const LOADING_POLES: usize = 3;

/// Contact duration at A0, in seconds — Askenfelt & Jansson's low-bass value.
const CONTACT_SECONDS_AT_A0: f32 = 4.0e-3;

/// Contact duration at C8, in seconds — the same papers' top-treble value.
const CONTACT_SECONDS_AT_C8: f32 = 0.6e-3;

const A0_HZ: f32 = 27.5;
const C8_HZ: f32 = 4_186.0;

/// The loading corner, in partials, where contact lasts exactly one period.
const LOADING_CORNER_PARTIALS: f32 = 2.0;

/// How steeply the corner falls with the contact-to-period ratio.
const LOADING_EXPONENT: f32 = 3.0;

/// Highest corner used, as a fraction of the sample rate. Above it a
/// one-pole section stops behaving like the analogue lowpass it maps from;
/// at this corner the filter is transparent across every audible partial
/// anyway, once normalised at the fundamental.
const MAX_CORNER_FRACTION: f32 = 0.4;

/// Lowest corner used, in partials. Normalising a lowpass back to unit gain
/// at the fundamental amplifies everything *below* the fundamental by the
/// same factor — the burst's near-DC content included — so the corner may
/// not fall below the fundamental itself, where that gain is at most
/// `√2` per section.
const MIN_CORNER_PARTIALS: f32 = 1.0;

/// [`DEFAULT_HAMMER`]'s own contact duration at [`REFERENCE_VELOCITY`], in
/// seconds — measured from `hammer::simulate_contact` (198 samples at
/// 48 kHz) and pinned by `the_reference_contact_matches_the_simulation`.
const REFERENCE_CONTACT_SECONDS: f32 = 4.12e-3;

/// The strike velocity [`hammer_for_frequency`] calibrates contact at: a
/// *mezzo-forte*, where Askenfelt & Jansson's durations were reported.
pub const REFERENCE_VELOCITY: f32 = 0.6;

/// Hammer mass at C8 relative to A0. Treble hammers weigh roughly 4-5 g
/// against 10-11 g in the bass (H. A. Conklin, "Design and tone in the
/// mechanoacoustic piano. Part I", JASA 99(6), 1996).
const MASS_RATIO_AT_C8: f32 = 0.4;

/// Felt and mass-loading lowpasses in series, plus the gain that returns the
/// fundamental to the level the felt alone would give it.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ExcitationShaper {
    felt: [OnePoleLowpass; FELT_POLES],
    loading: [OnePoleLowpass; LOADING_POLES],
    loading_gain: f32,
}

impl ExcitationShaper {
    /// A shaper for one strike: the felt corner at `felt_cutoff_hz`, the
    /// loading corner from `fundamental_hz`.
    pub(crate) fn new(felt_cutoff_hz: f32, fundamental_hz: f32, sample_rate_hz: f32) -> Self {
        let corner = mass_loading_corner_hz(fundamental_hz, sample_rate_hz);
        let section = OnePoleLowpass::from_cutoff(corner, sample_rate_hz);
        let per_section = section.magnitude_at(fundamental_hz, sample_rate_hz);
        Self {
            felt: [OnePoleLowpass::from_cutoff(felt_cutoff_hz, sample_rate_hz); FELT_POLES],
            loading: [section; LOADING_POLES],
            loading_gain: 1.0
                / [per_section; LOADING_POLES]
                    .iter()
                    .product::<f32>()
                    .max(f32::MIN_POSITIVE),
        }
    }

    /// Shapes one excitation sample.
    #[inline]
    pub(crate) fn process(&mut self, sample: f32) -> f32 {
        let felt = self
            .felt
            .iter_mut()
            .fold(sample, |value, stage| stage.process(value));
        self.loading
            .iter_mut()
            .fold(felt * self.loading_gain, |value, stage| {
                stage.process(value)
            })
    }
}

/// Hammer-string contact duration for a string at `fundamental_hz`,
/// interpolated geometrically between [`CONTACT_SECONDS_AT_A0`] and
/// [`CONTACT_SECONDS_AT_C8`] — a straight line on the log-log plot the
/// measurements are reported on.
#[must_use]
pub fn contact_seconds(fundamental_hz: f32) -> f32 {
    CONTACT_SECONDS_AT_A0
        * math::powf(
            CONTACT_SECONDS_AT_C8 / CONTACT_SECONDS_AT_A0,
            keyboard_position(fundamental_hz),
        )
}

/// The hammer for a string at `fundamental_hz`: lighter toward the treble
/// ([`MASS_RATIO_AT_C8`]) and with felt stiff enough that, at
/// [`REFERENCE_VELOCITY`], its contact lasts [`contact_seconds`].
///
/// One hammer for the whole keyboard — the bass hammer, 4 ms of contact —
/// left a top-octave string driven for six or seven of its own periods, which
/// measured as a 4 ms click 13-20 dB louder than the note it started. For
/// a power-law felt `F = K·xᵖ` the contact lasts `τ ∝ (m/K)^(1/(p+1))`, so
/// the stiffness that gives duration `τ` at mass `m` is
/// `K = K₀·(m/m₀)·(τ₀/τ)^(p+1)`.
#[must_use]
pub fn hammer_for_frequency(fundamental_hz: f32) -> HammerConfig {
    let mass =
        DEFAULT_HAMMER.mass * math::powf(MASS_RATIO_AT_C8, keyboard_position(fundamental_hz));
    let shortening = REFERENCE_CONTACT_SECONDS / contact_seconds(fundamental_hz);
    HammerConfig {
        mass,
        stiffness: DEFAULT_HAMMER.stiffness
            * (mass / DEFAULT_HAMMER.mass)
            * math::powf(shortening, DEFAULT_HAMMER.contact_exponent + 1.0),
        ..DEFAULT_HAMMER
    }
}

/// Where `fundamental_hz` sits between A0 (`0`) and C8 (`1`), in log
/// frequency; clamped, and `0` for `NaN`.
fn keyboard_position(fundamental_hz: f32) -> f32 {
    math::ln(math::clamp_or_low(fundamental_hz, A0_HZ, C8_HZ) / A0_HZ) / math::ln(C8_HZ / A0_HZ)
}

/// The mass-loading lowpass corner, in hertz, for a string at
/// `fundamental_hz` — see the module docs. Total: a `NaN` or non-positive
/// frequency clamps to A0, whose corner is the transparent ceiling.
#[must_use]
pub fn mass_loading_corner_hz(fundamental_hz: f32, sample_rate_hz: f32) -> f32 {
    let fundamental = math::clamp_or_low(fundamental_hz, A0_HZ, C8_HZ);
    let ratio = contact_seconds(fundamental) * fundamental;
    let corner = fundamental * LOADING_CORNER_PARTIALS / math::powf(ratio, LOADING_EXPONENT);
    let ceiling = math::clamp_or_low(sample_rate_hz, 1.0, f32::MAX) * MAX_CORNER_FRACTION;
    math::clamp_or_low(corner, fundamental * MIN_CORNER_PARTIALS, ceiling)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    const RATE: f32 = 48_000.0;

    #[test]
    fn the_bass_is_left_alone_and_the_top_treble_is_lowpassed_below_its_second_partial() {
        assert!(mass_loading_corner_hz(55.0, RATE) >= RATE * MAX_CORNER_FRACTION * 0.99);
        assert!(mass_loading_corner_hz(440.0, RATE) > 3_500.0);
        assert!(mass_loading_corner_hz(2_093.0, RATE) < 1.2 * 2_093.0);
    }

    #[test]
    fn the_reference_contact_matches_the_simulation() {
        let (_, samples) =
            crate::hammer::simulate_contact(REFERENCE_VELOCITY, RATE, DEFAULT_HAMMER);
        let measured = samples as f32 / RATE;
        assert!(
            (measured / REFERENCE_CONTACT_SECONDS - 1.0).abs() < 0.03,
            "{measured}"
        );
    }

    #[test]
    fn each_register_s_hammer_lands_on_its_contact_duration() {
        for fundamental in [27.5, 110.0, 440.0, 1_760.0, 4_186.0] {
            let hammer = hammer_for_frequency(fundamental);
            let (_, samples) = crate::hammer::simulate_contact(REFERENCE_VELOCITY, RATE, hammer);
            let ratio = samples as f32 / RATE / contact_seconds(fundamental);
            assert!((ratio - 1.0).abs() < 0.1, "{fundamental} Hz: {ratio}");
        }
    }

    #[test]
    fn contact_duration_hits_both_anchors() {
        assert!((contact_seconds(A0_HZ) - CONTACT_SECONDS_AT_A0).abs() < 1e-6);
        assert!((contact_seconds(C8_HZ) - CONTACT_SECONDS_AT_C8).abs() < 1e-6);
    }

    #[test]
    fn a_steady_tone_at_the_fundamental_keeps_the_felt_only_level() {
        let fundamental = 1_319.0;
        let mut shaper = ExcitationShaper::new(20_000.0, fundamental, RATE);
        let omega = core::f32::consts::TAU * fundamental / RATE;
        let peak = (0..20_000)
            .map(|n| shaper.process(math::sin(omega * n as f32)))
            .skip(10_000)
            .fold(0.0f32, |high, value| high.max(value.abs()));
        let felt_alone =
            OnePoleLowpass::from_cutoff(20_000.0, RATE).magnitude_at(fundamental, RATE);
        assert!(
            (peak - felt_alone * felt_alone).abs() < 0.05,
            "peak {peak}, felt-only {}",
            felt_alone * felt_alone
        );
    }

    proptest! {
        #[test]
        fn the_shaper_is_total(
            fundamental in proptest::num::f32::ANY,
            cutoff in proptest::num::f32::ANY,
            input in -1.0e3f32..1.0e3,
        ) {
            let mut shaper = ExcitationShaper::new(cutoff, fundamental, RATE);
            for _ in 0..64 {
                prop_assert!(shaper.process(input).is_finite());
            }
            let hammer = hammer_for_frequency(fundamental);
            prop_assert!(hammer.mass.is_finite() && hammer.stiffness.is_finite());
        }
    }
}
