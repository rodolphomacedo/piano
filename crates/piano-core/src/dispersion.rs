//! An allpass dispersion cascade — why upper piano partials sit sharp.
//!
//! A real string is stiff, not an ideal flexible string, so its overtones sit
//! above an exact harmonic series: `f_n ≈ n·f_1·sqrt(1 + B·n²)` (H. Fletcher,
//! "Normal Vibration Frequencies of a Stiff Piano String", JASA 36 (1964)).
//! A digital waveguide reproduces that by cascading first-order allpass
//! sections inside the loop (D. Jaffe & J. O. Smith, "Extensions of the
//! Karplus-Strong Plucked-String Algorithm", 1983): each section has unity
//! magnitude at every frequency but adds a frequency-dependent phase delay,
//! and stacking enough of them approximates the stretched dispersion curve
//! well enough to be audible as "piano" rather than "guitar".
//!
//! # The simplification this makes, stated plainly
//!
//! Fitting a cascade exactly to Fletcher's curve is a numerical optimisation
//! problem in its own right (Jaffe & Smith's own solution to it is
//! iterative, not closed-form). This implementation instead ties one
//! section's coefficient magnitude to `B` by a simple monotonic scaling and
//! the number of sections to register (following the measured table in
//! `docs/PHYSICS.md`: roughly 8 sections needed at A0, 2 at A4, 0-1 at C8),
//! and the scaling constant was calibrated by checking the measured partial
//! sharpening on a rendered note (see the M4 milestone's spectral test),
//! not asserted from the formula alone.

use crate::math;

/// Highest number of allpass sections any string ever uses.
///
/// `docs/PHYSICS.md` measures roughly 8 sections needed at A0 and 0-1 at C8;
/// 8 is therefore an upper bound that costs nothing in the treble, where the
/// active section count computes down to 0 or 1.
pub const MAX_SECTIONS: usize = 8;

/// [`StringConfig`](crate::string::StringConfig) uses this when the caller
/// does not choose an inharmonicity coefficient. Representative of a
/// mid-register piano string — Fletcher & Rossing report typical values
/// from roughly 0.0001 in the bass to 0.05 at the top of the treble.
pub const DEFAULT_INHARMONICITY: f32 = 0.000_4;

/// Highest inharmonicity coefficient accepted, at the top of the range
/// Fletcher & Rossing report for a full-size piano.
pub const MAX_INHARMONICITY: f32 = 0.05;

/// Highest magnitude allowed for a section's coefficient, short of the unit
/// circle, so the cascade is stable by construction for every `B` — and low
/// enough that one section's own phase response cannot destabilise a
/// unison group. See the note below for why "stable" alone (the original
/// reason for this margin) was not the same as "safe".
///
/// # Why `0.9` was not safe, even though it never diverged
///
/// A single allpass section is magnitude-preserving at every frequency for
/// any `|coefficient| < 1` — that is the whole point of the structure, and
/// it is why this cascade cannot itself lose energy. But its *phase*
/// response grows arbitrarily steep as the coefficient approaches the unit
/// circle: [`Section::phase_delay_at_dc`] is `19` samples at a coefficient
/// of `-0.9`, against `4` samples at `-0.5`. Steep phase response means
/// high sensitivity to small frequency changes — and `crate::unison`
/// detunes a note's own unison strings from each other by a few cents for
/// exactly the beating a real piano has.
///
/// [`section_count`] computes exactly `1` active section for roughly
/// C#5-B5 (554-988 Hz) — the docs already call this "0-1" a soft boundary
/// — and a struck note there with high enough inharmonicity clamped its
/// one section's coefficient to `-0.9`. That one section's steep phase
/// response, applied to three strings a few cents apart, was enough to
/// spin their post-dispersion signals out of phase with each other within
/// a handful of round trips. `crate::unison::UnisonGroup`'s local blend
/// assumes "same note, coupled with no latency, so the difference term is
/// genuinely small" (`crate::string::PluckedString::write_mixed_feedback`'s
/// own doc comment) — once the three strings' dispersed signals stopped
/// being genuinely close, blending them fed each string a partly
/// self-cancelling signal every round trip, an unbounded, compounding loss
/// nothing about `sustain` predicts or accounts for.
///
/// Measured, at A5 (the worst point in that band — the highest
/// inharmonicity within the single-section region): a trichord's RMS fell
/// from `0.14` to numerically silent within six 20 ms windows (about
/// 120 ms), while the same note plucked as a monochord (no detuning to
/// diverge against) rang normally. At `0.8`, swept across the whole
/// affected band (G5 through B5), every trichord decays smoothly instead
/// — see `piano-audio`'s `report_a5_unison_group_decay_in_isolation` and
/// `crate::unison::tests::a_high_inharmonicity_trichord_in_the_single_
/// dispersion_section_band_does_not_collapse`. `0.8` was chosen as the
/// smallest reduction from `0.9` that closed the gap with margin across
/// that whole band, not the smallest one that merely fixed A5 itself.
const MAX_COEFFICIENT: f32 = 0.8;

/// Scales `B` into a per-section allpass coefficient.
///
/// Negative, because the string must appear to have *less* group delay at
/// high frequency than at DC for its upper partials to sit sharp rather than
/// flat — see the module docs. Tuned so [`DEFAULT_INHARMONICITY`] on a
/// mid-register note produces a clearly measurable sharpening curve well
/// short of [`MAX_COEFFICIENT`].
const COEFFICIENT_GAIN: f32 = 200.0;

/// One first-order allpass section, `H(z) = (a + z⁻¹) / (1 + a·z⁻¹)`.
///
/// Implemented as the canonical single-multiply structure (J. O. Smith III,
/// *Physical Audio Signal Processing*, "Elementary Allpass Filters"): one
/// state variable rather than the two a direct-form realisation would need.
#[derive(Debug, Clone, Copy, Default)]
struct Section {
    coefficient: f32,
    state: f32,
}

impl Section {
    fn new(coefficient: f32) -> Self {
        Self {
            coefficient: math::clamp_or_low(coefficient, -MAX_COEFFICIENT, MAX_COEFFICIENT),
            state: 0.0,
        }
    }

    #[inline]
    fn process(&mut self, input: f32) -> f32 {
        let next_state = math::flush_denormal(input - self.coefficient * self.state);
        let output = self.coefficient * next_state + self.state;
        self.state = next_state;
        output
    }

    #[inline]
    fn phase_delay_at_dc(self) -> f32 {
        (1.0 - self.coefficient) / (1.0 + self.coefficient)
    }

    fn reset(&mut self) {
        self.state = 0.0;
    }
}

/// A cascade of identical allpass sections, the number scaled by register.
#[derive(Debug, Clone, Copy)]
pub struct DispersionCascade {
    sections: [Section; MAX_SECTIONS],
    active: usize,
}

impl DispersionCascade {
    /// Builds a cascade for a string at `frequency_hz` with inharmonicity
    /// `inharmonicity`.
    #[must_use]
    pub fn new(frequency_hz: f32, inharmonicity: f32) -> Self {
        let coefficient = coefficient_from_inharmonicity(inharmonicity);
        Self {
            sections: [Section::new(coefficient); MAX_SECTIONS],
            active: section_count(frequency_hz),
        }
    }

    /// Updates the inharmonicity coefficient in place, live — the same
    /// pattern as [`crate::string::PluckedString::set_damping`]. Section
    /// count stays fixed: it depends only on register, which does not
    /// change for an already-built string.
    pub fn set_inharmonicity(&mut self, inharmonicity: f32) {
        let coefficient = coefficient_from_inharmonicity(inharmonicity);
        for section in &mut self.sections {
            section.coefficient = coefficient;
        }
    }

    /// Runs the signal through every active section, in order.
    #[inline]
    #[must_use]
    pub fn process(&mut self, input: f32) -> f32 {
        let mut sample = input;
        for section in self.sections.iter_mut().take(self.active) {
            sample = section.process(sample);
        }
        sample
    }

    /// Total phase delay the active sections add at DC — part of what tunes
    /// the loop, same reasoning as
    /// [`crate::filter::LoopFilter::phase_delay_at_dc`].
    #[inline]
    #[must_use]
    pub fn phase_delay_at_dc(&self) -> f32 {
        self.sections
            .iter()
            .copied()
            .take(self.active)
            .map(Section::phase_delay_at_dc)
            .sum()
    }

    /// Clears every section's state, for a fresh strike.
    pub fn reset(&mut self) {
        for section in &mut self.sections {
            section.reset();
        }
    }

    /// How many sections are active, for tests and diagnostics.
    #[inline]
    #[must_use]
    pub fn active_sections(&self) -> usize {
        self.active
    }
}

/// Total for every `f32`, including `NaN` and infinities: both are clamped
/// by [`math::clamp_or_low`] before they can reach the coefficient.
fn coefficient_from_inharmonicity(inharmonicity: f32) -> f32 {
    let b = math::clamp_or_low(inharmonicity, 0.0, MAX_INHARMONICITY);
    math::clamp_or_low(-COEFFICIENT_GAIN * b, -MAX_COEFFICIENT, MAX_COEFFICIENT)
}

/// Total for every `f32`: a non-finite or non-positive frequency falls back
/// to A0 rather than feeding a `NaN` or a negative-domain log into the
/// section-count formula.
fn section_count(frequency_hz: f32) -> usize {
    let frequency = if frequency_hz.is_finite() && frequency_hz > 0.0 {
        frequency_hz
    } else {
        27.5
    };
    let octaves_above_a0 = math::ln(frequency / 27.5) / core::f32::consts::LN_2;
    let sections = 8.0 - 1.5 * octaves_above_a0;
    math::round(math::clamp_or_low(sections, 0.0, MAX_SECTIONS as f32)) as usize
}

#[cfg(test)]
mod tests {
    #![allow(clippy::float_cmp, clippy::unwrap_used, clippy::expect_used)]

    use proptest::prelude::*;

    use super::*;

    #[test]
    fn register_scaling_matches_the_documented_table() {
        // docs/PHYSICS.md: ~8 sections at A0, ~2 at A4, ~0-1 at C8.
        assert_eq!(section_count(27.5), 8);
        assert_eq!(section_count(440.0), 2);
        assert!(section_count(4186.0) <= 1);
    }

    #[test]
    fn more_inharmonicity_means_a_larger_coefficient_magnitude() {
        let small = coefficient_from_inharmonicity(0.0001).abs();
        let large = coefficient_from_inharmonicity(0.01).abs();
        assert!(large > small, "small {small} large {large}");
    }

    #[test]
    fn zero_inharmonicity_is_a_pure_delay_of_the_section_count() {
        // At coefficient 0 each section's transfer function is exactly
        // H(z) = z^-1 (see `Section::process`'s doc comment), so B=0
        // degrades gracefully to "no dispersion, just `active` extra
        // samples of delay" rather than a no-op — those extra samples are
        // exactly what `phase_delay_at_dc` reports and what
        // `PluckedString` compensates for when tuning the loop.
        let mut cascade = DispersionCascade::new(440.0, 0.0);
        let active = cascade.active_sections();
        let inputs = [0.3, -0.6, 0.9, -0.2, 0.5, -0.5, 0.1, -0.1];
        let outputs: Vec<f32> = inputs.iter().map(|&x| cascade.process(x)).collect();
        for (expected, actual) in inputs.iter().zip(outputs.iter().skip(active)) {
            assert!(
                (actual - expected).abs() < 1e-6,
                "output {actual} vs delayed input {expected}"
            );
        }
    }

    #[test]
    fn set_inharmonicity_changes_the_coefficient_without_resetting_state() {
        let mut cascade = DispersionCascade::new(440.0, 0.0);
        let _ = cascade.process(1.0);
        let before_phase_delay = cascade.phase_delay_at_dc();
        cascade.set_inharmonicity(0.01);
        assert_ne!(cascade.phase_delay_at_dc(), before_phase_delay);
    }

    #[test]
    fn reset_clears_every_section() {
        let mut cascade = DispersionCascade::new(27.5, 0.01);
        let _ = cascade.process(1.0);
        cascade.reset();
        assert_eq!(cascade.process(0.0), 0.0);
    }

    proptest! {
        /// The cascade must stay bounded and finite for every reachable
        /// inharmonicity and frequency, including NaN, +-infinity and zero.
        #[test]
        fn cascade_never_diverges(
            inharmonicity in proptest::num::f32::ANY,
            frequency in proptest::num::f32::ANY,
            input in -1.0f32..1.0,
        ) {
            let mut cascade = DispersionCascade::new(frequency, inharmonicity);
            let mut output = 0.0;
            for _ in 0..1_000 {
                output = cascade.process(input);
            }
            prop_assert!(output.is_finite());
            prop_assert!(output.abs() <= 1.0 + 1e-3, "output {output} exceeded input bound");
            prop_assert!(cascade.active_sections() <= MAX_SECTIONS);
        }
    }
}
