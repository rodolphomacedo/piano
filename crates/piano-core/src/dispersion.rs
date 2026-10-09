//! An allpass dispersion cascade — why upper piano partials sit sharp.
//!
//! A real string is stiff, not an ideal flexible string, so its overtones sit
//! above an exact harmonic series: `f_n ≈ n·f_1·sqrt(1 + B·n²)` (H. Fletcher,
//! "Normal Vibration Frequencies of a Stiff Piano String", JASA 36 (1964)).
//! A digital waveguide reproduces that by cascading first-order allpass
//! sections inside the loop (D. Jaffe & J. O. Smith, "Extensions of the
//! Karplus-Strong Plucked-String Algorithm", 1983; S. Van Duyne & J. O.
//! Smith, "A Simplified Approach to Modeling Dispersion Caused by Stiffness
//! in Strings and Plates", ICMC 1994): each section has unity magnitude at
//! every frequency but a phase delay that falls with frequency, so the loop
//! is shorter for high partials and they sit sharp.
//!
//! # How the cascade is fitted
//!
//! Every section shares one coefficient (Van Duyne & Smith's identical-
//! section structure), so the cascade has two unknowns: the section count
//! `M` and the coefficient `a`. Both are *fitted* per string against
//! Fletcher's curve, not derived from register: for each `M` the delay
//! budget allows, a bounded grid-then-golden-section search picks the `a`
//! whose loop puts partials `2..=K` closest (in relative frequency) to
//! `n·f0·sqrt((1 + B·n²)/(1 + B))` once the loop is tuned so partial 1 lands
//! on `f0`. `M` grows only until every partial up to the eighth sits within
//! two cents, since each section costs a multiply per sample.
//!
//! The previous design tied `a = -200·B` (clamped at `-0.8`) and `M` to
//! register. Measured, it realised 1-13% of the requested `B` below C6 and
//! none at all above it, where `M` reached zero (issue #96).
//!
//! The search is bounded — fixed grid and refinement counts, at most
//! [`MAX_SECTIONS`] candidates — and allocation-free, so the live
//! [`DispersionCascade::set_inharmonicity`] stays legal on the audio thread
//! (it refits the coefficient only, keeping `M`, which is the cheap part).

use core::f32::consts::TAU;

use crate::math;

/// Highest number of allpass sections any string ever uses.
pub const MAX_SECTIONS: usize = 8;

/// [`StringConfig`](crate::string::StringConfig) uses this when the caller
/// does not choose an inharmonicity coefficient. Representative of a
/// mid-register plain-wire piano string (Fletcher & Rossing, *The Physics of
/// Musical Instruments*, 2nd ed., §12.4).
pub const DEFAULT_INHARMONICITY: f32 = 0.000_4;

/// Highest inharmonicity coefficient accepted — a ceiling above anything a
/// full-size piano's top treble string reaches, not a typical value.
pub const MAX_INHARMONICITY: f32 = 0.05;

/// Highest coefficient magnitude a section may take, short of the unit
/// circle so the cascade is stable by construction.
///
/// A bass string needs its delay to fall between partials only tens of
/// hertz apart, which a first-order section only does with its pole close
/// to the unit circle. The previous cap of `0.8` came from a unison collapse
/// at A5 that was driven by that note being given roughly 20 times its
/// physical `B` (issue #96); with the fitted design, mid and treble
/// coefficients sit far from this bound.
const MAX_COEFFICIENT: f32 = 0.97;

/// Most partials the fit weighs. Beyond the sixteenth, a piano partial is
/// both quiet and short-lived.
const MAX_FITTED_PARTIALS: usize = 16;

/// Highest frequency, in radians per sample, a fitted partial may sit at —
/// about 10.8 kHz at 48 kHz. The fit is not asked to place partials the
/// loss filter has already all but removed.
const HIGHEST_FITTED_OMEGA: f32 = 0.45 * core::f32::consts::PI;

/// Share of the loop period the cascade may claim as delay at DC. The rest
/// is left to the delay line, which must stay at least a few samples long.
const MAX_DELAY_SHARE: f32 = 0.5;

/// Coarse grid points per coefficient search, before refinement.
const GRID_POINTS: usize = 16;

/// Golden-section refinement steps after the grid. Twelve steps shrink the
/// bracket to `0.618^12 ≈ 0.003` of its starting width.
const REFINE_STEPS: usize = 12;

/// `1/φ`, the golden-section bracket ratio.
const INVERSE_PHI: f32 = 0.618_034;

/// The partials whose placement decides whether a note sounds in tune with
/// itself; above the eighth, partials are quiet and short-lived.
const AUDIBLE_PARTIALS: usize = 8;

/// Once every audible partial sits within this many cents of Fletcher's
/// curve, more sections are not worth their per-sample cost. Two cents is
/// well under the mistuning a listener detects in a single partial.
const AUDIBLE_TOLERANCE_CENTS: f32 = 2.0;

/// `1200 / ln 2`: cents per unit of small relative frequency error.
const CENTS_PER_UNIT_RATIO: f32 = 1731.234;

/// One first-order allpass section, `H(z) = (a + z⁻¹) / (1 + a·z⁻¹)`.
///
/// Implemented as the canonical single-multiply structure (J. O. Smith III,
/// *Physical Audio Signal Processing*, "Elementary Allpass Filters"): one
/// state variable rather than the two a direct-form realisation would need.
#[derive(Debug, Clone, Copy, Default)]
struct Section {
    state: f32,
}

impl Section {
    #[inline]
    fn process(&mut self, input: f32, coefficient: f32) -> f32 {
        let next_state = math::flush_denormal(input - coefficient * self.state);
        let output = coefficient * next_state + self.state;
        self.state = next_state;
        output
    }
}

/// A fitted `(section count, shared coefficient)` pair.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Design {
    sections: usize,
    coefficient: f32,
}

impl Design {
    const NONE: Self = Self {
        sections: 0,
        coefficient: 0.0,
    };
}

/// What the fit is aiming at: one string's fundamental and stiffness.
#[derive(Debug, Clone, Copy)]
struct FitTarget {
    omega0: f32,
    inharmonicity: f32,
    partials: usize,
}

impl FitTarget {
    fn new(omega0: f32, inharmonicity: f32) -> Self {
        let mut target = Self {
            omega0,
            inharmonicity,
            partials: 0,
        };
        target.partials = (1..=MAX_FITTED_PARTIALS)
            .take_while(|&n| target.omega_of(n) < HIGHEST_FITTED_OMEGA)
            .count();
        target
    }

    /// Where partial `n` should sit, with partial 1 tuned to `omega0`.
    fn omega_of(&self, n: usize) -> f32 {
        let n = n as f32;
        let b = self.inharmonicity;
        n * self.omega0 * math::sqrt((1.0 + b * n * n) / (1.0 + b))
    }

    /// Relative frequency error of partial `n` under `design`, with the loop
    /// tuned so partial 1 lands exactly on target. Positive is flat.
    fn relative_error(&self, design: Design, n: usize) -> f32 {
        let m = design.sections as f32;
        let cascade_delay = |omega: f32| m * math::allpass_phase_delay(design.coefficient, omega);
        let rest_of_loop = TAU / self.omega0 - cascade_delay(self.omega0);
        let omega = self.omega_of(n);
        let wanted = TAU * n as f32 / omega;
        (wanted - rest_of_loop - cascade_delay(omega)) / wanted
    }

    /// Squared relative errors over partials `2..=K`, weighted by `1/n²` —
    /// roughly how a struck string's partial energy falls with `n` — so the
    /// fit spends its freedom on the partials that carry the sound.
    fn error(&self, design: Design) -> f32 {
        (2..=self.partials)
            .map(|n| {
                let relative = self.relative_error(design, n);
                relative * relative / (n * n) as f32
            })
            .sum()
    }

    /// Worst error, in cents, over partials `2..=AUDIBLE_PARTIALS`.
    fn worst_audible_cents(&self, design: Design) -> f32 {
        (2..=self.partials.min(AUDIBLE_PARTIALS))
            .map(|n| math::abs(self.relative_error(design, n)) * CENTS_PER_UNIT_RATIO)
            .fold(0.0, f32::max)
    }
}

/// Fits the cascade for a string whose loop is `period` samples long.
fn fit(period: f32, inharmonicity: f32) -> Design {
    let Some(target) = fit_target(period, inharmonicity) else {
        return Design::NONE;
    };
    let budget = period * MAX_DELAY_SHARE;
    let mut best: Option<(Design, f32)> = None;
    for sections in 1..=MAX_SECTIONS {
        let Some(candidate) = fit_coefficient(&target, sections, budget) else {
            break;
        };
        if best.is_none_or(|(_, error)| candidate.1 < error) {
            best = Some(candidate);
        }
        if target.worst_audible_cents(candidate.0) <= AUDIBLE_TOLERANCE_CENTS {
            break;
        }
    }
    best.map_or(Design::NONE, |(design, _)| design)
}

/// `None` when there is nothing to fit: no stiffness, an unusable period,
/// or fewer than two partials below [`HIGHEST_FITTED_OMEGA`].
fn fit_target(period: f32, inharmonicity: f32) -> Option<FitTarget> {
    let b = math::clamp_or_low(inharmonicity, 0.0, MAX_INHARMONICITY);
    if !(period.is_finite() && period > 2.0 && b > 0.0) {
        return None;
    }
    let target = FitTarget::new(TAU / period, b);
    (target.partials >= 2).then_some(target)
}

/// Best coefficient for exactly `sections` sections, with its error, or
/// `None` when even a coefficient of zero (one sample per section) would
/// exceed `budget`.
fn fit_coefficient(target: &FitTarget, sections: usize, budget: f32) -> Option<(Design, f32)> {
    let steepest = steepest_coefficient(sections, budget)?;
    let error_at = |coefficient: f32| {
        target.error(Design {
            sections,
            coefficient,
        })
    };
    let coefficient = minimise(error_at, steepest, 0.0);
    let design = Design {
        sections,
        coefficient,
    };
    Some((design, target.error(design)))
}

/// Most negative coefficient whose DC delay, over `sections` sections, fits
/// in `budget` samples: `(1 - a)/(1 + a) = budget / sections`.
fn steepest_coefficient(sections: usize, budget: f32) -> Option<f32> {
    let per_section = budget / sections as f32;
    if per_section < 1.0 {
        return None;
    }
    let coefficient = (1.0 - per_section) / (1.0 + per_section);
    Some(coefficient.max(-MAX_COEFFICIENT))
}

/// Minimises `error` over `[low, high]`: a coarse grid, then golden-section
/// refinement around the best grid point. Bounded by construction.
fn minimise(error: impl Fn(f32) -> f32, low: f32, high: f32) -> f32 {
    let step = (high - low) / (GRID_POINTS - 1) as f32;
    let best_index = (0..GRID_POINTS)
        .map(|index| (index, error(low + step * index as f32)))
        .fold((0, f32::INFINITY), |best, candidate| {
            if candidate.1 < best.1 {
                candidate
            } else {
                best
            }
        })
        .0;
    let centre = low + step * best_index as f32;
    golden_section(&error, (centre - step).max(low), (centre + step).min(high))
}

fn golden_section(error: &impl Fn(f32) -> f32, mut low: f32, mut high: f32) -> f32 {
    for _ in 0..REFINE_STEPS {
        let left = high - INVERSE_PHI * (high - low);
        let right = low + INVERSE_PHI * (high - low);
        if error(left) < error(right) {
            high = right;
        } else {
            low = left;
        }
    }
    f32::midpoint(low, high)
}

/// A cascade of identical first-order allpass sections, fitted per string.
#[derive(Debug, Clone, Copy)]
pub struct DispersionCascade {
    sections: [Section; MAX_SECTIONS],
    design: Design,
    period: f32,
}

impl DispersionCascade {
    /// Builds a cascade fitted to a string whose loop is `period` samples
    /// long (sample rate over fundamental) with inharmonicity
    /// `inharmonicity`. Total for every `f32`: anything non-finite, a period
    /// too short to hold two fitted partials, or `B <= 0` gives an empty
    /// cascade — a pure pass-through with zero delay.
    #[must_use]
    pub fn new(period: f32, inharmonicity: f32) -> Self {
        Self {
            sections: [Section::default(); MAX_SECTIONS],
            design: fit(period, inharmonicity),
            period,
        }
    }

    /// Updates the inharmonicity coefficient in place, live. Keeps the
    /// section count and refits only the coefficient — cheap enough for the
    /// audio thread — unless the cascade is currently empty, in which case
    /// it runs the full fit and clears the sections it brings into use.
    pub fn set_inharmonicity(&mut self, inharmonicity: f32) {
        let previous = self.design.sections;
        self.design = match fit_target(self.period, inharmonicity) {
            None => Design::NONE,
            Some(_) if previous == 0 => fit(self.period, inharmonicity),
            Some(target) => fit_coefficient(&target, previous, self.period * MAX_DELAY_SHARE)
                .map_or(Design::NONE, |(design, _)| design),
        };
        for section in self.sections.iter_mut().skip(previous) {
            *section = Section::default();
        }
    }

    /// Runs the signal through every active section, in order.
    #[inline]
    #[must_use]
    pub fn process(&mut self, input: f32) -> f32 {
        let coefficient = self.design.coefficient;
        self.sections
            .iter_mut()
            .take(self.design.sections)
            .fold(input, |sample, section| {
                section.process(sample, coefficient)
            })
    }

    /// Total phase delay the active sections add at DC.
    #[inline]
    #[must_use]
    pub fn phase_delay_at_dc(&self) -> f32 {
        self.phase_delay_at(0.0)
    }

    /// Total phase delay the active sections add at `omega` radians per
    /// sample — what the loop is tuned by at the fundamental.
    #[inline]
    #[must_use]
    pub fn phase_delay_at(&self, omega: f32) -> f32 {
        self.design.sections as f32 * math::allpass_phase_delay(self.design.coefficient, omega)
    }

    /// Clears every section's state, for a fresh strike.
    pub fn reset(&mut self) {
        self.sections = [Section::default(); MAX_SECTIONS];
    }

    /// How many sections are active, for tests and diagnostics.
    #[inline]
    #[must_use]
    pub fn active_sections(&self) -> usize {
        self.design.sections
    }

    /// The fitted shared coefficient, for tests and diagnostics.
    #[inline]
    #[must_use]
    pub fn coefficient(&self) -> f32 {
        self.design.coefficient
    }
}

#[cfg(test)]
#[path = "dispersion_tests.rs"]
mod tests;
