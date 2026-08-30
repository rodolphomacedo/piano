//! Modal synthesis standing in for the piano's soundboard (`PERF-009`).
//!
//! # Why modal synthesis, not convolution — and a rule this project cannot
//! bend
//!
//! A measured soundboard impulse response is 50 000-100 000 taps at
//! 48 kHz; direct convolution is 2.4-4.8 G multiply-adds/second
//! (`PERF-009` in `docs/PERFORMANCE.md`). That entry lists three options —
//! uniformly-partitioned FFT convolution, non-uniform partitioning, and
//! modal synthesis — and this project can only ever build the third one.
//! The first two require possessing a measured impulse response, i.e. a
//! recording of a real soundboard, to convolve against. This project's own
//! `CLAUDE.md` states, without qualification, that there are no samples or
//! recordings anywhere in this repository and none may be added — a rule
//! that applies here regardless of how the convolution options are phrased
//! in the performance register. **No audio file, sampled impulse response,
//! or other captured-audio asset is read, generated-then-treated-as-real,
//! or stored anywhere in this module or this crate.**
//!
//! Modal synthesis sidesteps the problem entirely: a soundboard's resonant
//! behaviour is a bank of damped normal modes (frequency, decay time, gain)
//! — a bank of resonant filters excited by the same signal the real board
//! would be, rather than a recording of one particular board's response.
//! B. Bank et al. (already cited in `docs/PRIOR-ART.md` for this exact
//! choice: "soundboard modelling by resonator banks") describe this as the
//! cheap, fully parametric alternative to convolution, at the honestly
//! stated cost of being less faithful to any one specific real instrument —
//! precisely the trade `PERF-009` itself anticipated. This implementation
//! uses [`MODE_COUNT`] modes, fewer than a "faithful" soundboard model would
//! (real boards have dozens of resolvable low-order modes alone,
//! e.g. K. Wogram, "Acoustical Research on Pianos" (1980); N. Suzuki,
//! "Vibration and sound radiation of a piano soundboard" (JASA 80, 1986)),
//! which is explicitly the cheaper, more parametric option `PERF-009`
//! itself describes as acceptable.
//!
//! # Honesty about the specific numbers
//!
//! [`DEFAULT_MODES`]' frequencies, decay times and gains are representative,
//! literature-informed order-of-magnitude values (grand-piano soundboards'
//! lowest structural modes cluster roughly 80-400 Hz per Wogram and Suzuki
//! above, with higher, more heavily damped modes filling in above that) —
//! not measurements of one specific real instrument, and not fit to any
//! recording, honestly labelled the same way `piano_audio::voicing`'s
//! per-partial decay anchors already are for exactly the same reason:
//! this project has no per-instrument measurement to calibrate against and
//! says so rather than presenting a guess as data.
//!
//! # The resonator itself
//!
//! Each mode is a standard two-pole digital resonator (J. O. Smith III,
//! *Physical Audio Signal Processing*, "Two-Pole Resonators" — already this
//! project's foundational DSP reference per `docs/PRIOR-ART.md`):
//! `y[n] = 2r·cos(θ)·y[n-1] - r²·y[n-2] + g·x[n]`, poles at `r·e^{±jθ}`,
//! `θ = 2π·f/fs` and `r = exp(-1 / (τ·fs))` so the impulse response's
//! envelope decays to `1/e` in exactly `τ` seconds — the same
//! decay-time-to-coefficient derivation style `piano_audio::voicing`'s
//! `solve_loop_losses` already uses for the string loop, applied
//! here to a resonant filter instead of a delay-line loop gain.

use crate::{math, units::SampleRate};

/// How many resonant modes make up the bank.
///
/// Deliberately small — see the module docs on why fewer modes than
/// "faithful" is the expected, accepted cost of modal synthesis.
pub const MODE_COUNT: usize = 8;

/// Shortest decay time accepted for a mode.
///
/// A guard against a non-positive or `NaN` decay reaching `exp(-1/(τ·fs))`,
/// not against instability from *below*: a shorter decay gives a *smaller*
/// pole radius, which is further inside the unit circle, and
/// [`MAX_RADIUS`] is what holds the other end of the line. Low enough that
/// a heavily damped high mode — [`DEFAULT_MODE_Q`] at 5 kHz is under 2 ms —
/// is expressible rather than being silently clamped into ringing.
const MIN_DECAY_SECONDS: f32 = 0.000_1;

/// Quality factor every entry in [`DEFAULT_MODES`] is built at.
///
/// A ribbed spruce soundboard is a heavily damped structure: its modal loss
/// factor `η` runs to roughly `0.02`-`0.05` (J. Berthaut, M. N. Ichchou &
/// L. Jézéquel, "Piano soundboard: structural behavior…", *Applied
/// Acoustics* 64 (2003); consistent with the modal damping in Wogram and
/// Suzuki, cited in the module docs), giving `Q = 1/η` of about 20-50.
///
/// This table was previously written as hand-typed decay times which worked
/// out to `Q` between 302 and 1100 — a struck metal bar, not a wooden box.
/// Every note therefore rang with eight fixed, foreign, barely-damped
/// pitches under it, and where one of them landed near a partial (the
/// 420 Hz mode sits 81 cents from A4's fundamental) the two beat against
/// each other. Deriving the table from one stated `Q` rather than eight
/// independent numbers is what keeps that from silently returning.
const DEFAULT_MODE_Q: f32 = 30.0;

/// The decay time a mode at `frequency_hz` must have to reach `q`.
///
/// For a two-pole resonator whose envelope falls to `1/e` in `τ` seconds,
/// `Q = π·f·τ`, so `τ = Q/(π·f)` — the same decay-time-to-coefficient
/// derivation the module docs describe, run in the other direction.
const fn decay_seconds_for_q(frequency_hz: f32, q: f32) -> f32 {
    q / (core::f32::consts::PI * frequency_hz)
}

/// Highest pole radius allowed, short of the unit circle, so every
/// resonator is stable by construction — same margin pattern as
/// `filter::MAX_POLE` and `dispersion`'s `MAX_COEFFICIENT`.
const MAX_RADIUS: f32 = 0.999_9;

/// Lowest mode frequency [`Resonator::new`] accepts, so a live-set mode
/// (`Soundboard::set_mode`) cannot feed a non-positive frequency into the
/// resonator's phase calculation — the fixed [`DEFAULT_MODES`] table never gets
/// close to this bound, but a caller-supplied [`SoundboardMode`] now can.
const MIN_MODE_FREQUENCY_HZ: f32 = 1.0;

/// Highest mode frequency accepted, generously above the audible range
/// (a real listener stops hearing well below this), so a live-set mode
/// cannot alias the resonator's phase into a musically meaningless range.
const MAX_MODE_FREQUENCY_HZ: f32 = 20_000.0;

/// Highest relative gain accepted for one mode — several times the fixed
/// [`DEFAULT_MODES`] table's own loudest entry (`1.0`), enough headroom for live
/// tuning without a caller driving one resonator's contribution to the mix
/// arbitrarily loud.
const MAX_MODE_GAIN: f32 = 4.0;

/// One soundboard mode's physical parameters.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SoundboardMode {
    /// Mode frequency, in hertz.
    pub frequency_hz: f32,
    /// Time for this mode's contribution to decay to `1/e`, in seconds.
    pub decay_seconds: f32,
    /// Relative gain of this mode in the mixed output.
    pub gain: f32,
}

/// The bank's default modal table — what [`Soundboard::new`] starts every
/// board from, before any live [`Soundboard::set_mode`] call replaces an
/// entry. See the module docs' honesty note on where these numbers come
/// from and what they are not.
///
/// Public because a control surface offering live mode editing has to show
/// what the running board started at, and a [`Resonator`]'s parameters
/// cannot be read back out of its coefficients — the starting table is the
/// only truthful thing such a surface can display.
/// Every mode's decay time comes from [`DEFAULT_MODE_Q`] rather than being
/// written down separately — see that constant for why.
pub const DEFAULT_MODES: [SoundboardMode; MODE_COUNT] = [
    default_mode(80.0, 1.0),
    default_mode(130.0, 0.8),
    default_mode(190.0, 0.7),
    default_mode(280.0, 0.6),
    default_mode(420.0, 0.5),
    default_mode(650.0, 0.4),
    default_mode(950.0, 0.3),
    default_mode(1_400.0, 0.25),
];

/// One entry of [`DEFAULT_MODES`], damped at [`DEFAULT_MODE_Q`].
const fn default_mode(frequency_hz: f32, gain: f32) -> SoundboardMode {
    SoundboardMode {
        frequency_hz,
        decay_seconds: decay_seconds_for_q(frequency_hz, DEFAULT_MODE_Q),
        gain,
    }
}

/// One mode's two-pole resonator state. See the module docs for the
/// difference equation and its derivation.
#[derive(Debug, Clone, Copy, Default)]
struct Resonator {
    two_r_cos_theta: f32,
    neg_r_squared: f32,
    input_gain: f32,
    state1: f32,
    state2: f32,
}

/// The factor that makes a [`Resonator`]'s gain at its own resonant
/// frequency come out as exactly the `gain` its caller asked for.
///
/// `H(z) = g / ((1 - r·e^{jθ}z⁻¹)(1 - r·e^{-jθ}z⁻¹))`. Evaluated at
/// resonance (`z = e^{jθ}`) the first factor collapses to `1 - r` and the
/// second to `1 - r·e^{-j2θ}`, so the peak magnitude is
/// `g / ((1 - r)·|1 - r·e^{-j2θ}|)`. Both factors have to be divided out;
/// this returns their product, which is multiplied in rather than divided
/// by, so no input can produce a division at all.
///
/// This module used to divide out only `(1 - r)`, leaving the whole
/// `1/|1 - r·e^{-j2θ}|` term — approximately `1/(2·sin θ)` as `r → 1` —
/// standing in the signal path. Measured against the shipped mode table
/// that was a gain of `47.75` where `1.0` was asked for at 80 Hz, falling
/// to `0.69` where `0.25` was asked for at 1400 Hz: a bass-tilted boost of
/// up to 34 dB that made every note sound a fixed low thump plus seven
/// foreign pitches. `Resonator`'s own gain-normalisation test now measures
/// this rather than trusting the algebra above.
///
/// Total: `radius <= MAX_RADIUS < 1` and `theta` is finite (its frequency is
/// clamped before it gets here), so the result is finite and strictly
/// positive — it is never smaller than `(1 - r)²`.
#[inline]
#[must_use]
fn resonant_peak_reciprocal(radius: f32, theta: f32) -> f32 {
    let two_theta = 2.0 * theta;
    let real = 1.0 - radius * math::cos(two_theta);
    let imaginary = radius * math::sin(two_theta);
    (1.0 - radius) * math::sqrt(real * real + imaginary * imaginary)
}

impl Resonator {
    /// Total for every field of `mode`, including `NaN` and infinities:
    /// each is clamped into a finite, resonator-safe range before it can
    /// reach a division, a trig function or the recursion's own gain —
    /// necessary since [`Soundboard::set_mode`] now lets a caller supply
    /// `mode` live, not just the fixed, already-sane [`DEFAULT_MODES`] table.
    fn new(mode: SoundboardMode, sample_rate: f32) -> Self {
        let decay = math::clamp_or_low(mode.decay_seconds, MIN_DECAY_SECONDS, f32::MAX);
        let radius = math::clamp_or_low(math::exp(-1.0 / (decay * sample_rate)), 0.0, MAX_RADIUS);
        let frequency_hz = math::clamp_or_low(
            mode.frequency_hz,
            MIN_MODE_FREQUENCY_HZ,
            MAX_MODE_FREQUENCY_HZ,
        );
        let gain = math::clamp_or_low(mode.gain, 0.0, MAX_MODE_GAIN);
        let theta = core::f32::consts::TAU * frequency_hz / sample_rate;
        Self {
            two_r_cos_theta: 2.0 * radius * math::cos(theta),
            neg_r_squared: -(radius * radius),
            input_gain: gain * resonant_peak_reciprocal(radius, theta),
            state1: 0.0,
            state2: 0.0,
        }
    }

    #[inline]
    fn process(&mut self, input: f32) -> f32 {
        let output = self.two_r_cos_theta * self.state1
            + self.neg_r_squared * self.state2
            + self.input_gain * input;
        let output = math::flush_denormal(output);
        self.state2 = self.state1;
        self.state1 = output;
        output
    }
}

/// A bank of [`MODE_COUNT`] resonant modes, driven by one shared input and
/// summed into one radiated output.
///
/// Allocates nothing beyond its own fixed-size field (no heap allocation at
/// all, in fact — `Resonator` is a small `Copy` struct); constructed once,
/// in [`Soundboard::new`], from a sample rate fixed for the instrument's
/// lifetime. [`Soundboard::process`] is allocation-free, lock-free and
/// panic-free.
#[derive(Debug, Clone)]
pub struct Soundboard {
    resonators: [Resonator; MODE_COUNT],
    /// Kept so [`Soundboard::set_mode`] can rebuild one resonator later
    /// without the caller having to re-supply the sample rate.
    sample_rate: f32,
}

impl Soundboard {
    /// Builds a soundboard tuned for `sample_rate`, from the fixed
    /// [`DEFAULT_MODES`] table.
    #[must_use]
    pub fn new(sample_rate: SampleRate) -> Self {
        let hertz = sample_rate.hertz();
        let mut resonators = [Resonator::default(); MODE_COUNT];
        for (slot, mode) in resonators.iter_mut().zip(DEFAULT_MODES) {
            *slot = Resonator::new(mode, hertz);
        }
        Self {
            resonators,
            sample_rate: hertz,
        }
    }

    /// Rebuilds mode `index` in place, live, from `mode` — the same
    /// "replace the whole resonator" approach `dispersion::DispersionCascade`
    /// does not need (it only ever changes one coefficient) but a mode
    /// change does, since frequency, decay and gain all feed the
    /// resonator's fixed coefficients at construction time.
    ///
    /// Total for any `index`: out of `[0, MODE_COUNT)` is silently a no-op,
    /// the same "caller cannot crash the audio thread with a bad index"
    /// contract as everywhere else in this crate — there is no per-mode
    /// error to report on the audio thread, so silence, not a `Result`, is
    /// the total answer here.
    pub fn set_mode(&mut self, index: usize, mode: SoundboardMode) {
        if let Some(slot) = self.resonators.get_mut(index) {
            *slot = Resonator::new(mode, self.sample_rate);
        }
    }

    /// Drives every mode with `driving_input` (typically the engine's
    /// direct, pre-soundboard mix for this sample) and returns the summed
    /// radiated output — the post-mix stage `docs/ARCHITECTURE.md`
    /// documents, added into the direct signal rather than replacing it;
    /// see that document for why.
    #[inline]
    pub fn process(&mut self, driving_input: f32) -> f32 {
        // Every other filter in this crate is only ever fed a driving
        // signal that already passed through a string's own bounded loop,
        // but `Soundboard` sits after the mix of every voice, so a caller
        // driving it directly (as `any_input_sequence_is_total` does) can
        // hand it `NaN` or `+-infinity`. `clamp_or_low` maps `NaN` to the
        // low bound and this generous range absorbs infinities too,
        // keeping every resonator's own recursion finite by construction
        // rather than trusting every caller to pre-sanitise.
        let driving_input = math::clamp_or_low(driving_input, -1.0e6, 1.0e6);
        let mut output = 0.0f32;
        for resonator in &mut self.resonators {
            output += resonator.process(driving_input);
        }
        output
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::float_cmp, clippy::unwrap_used, clippy::expect_used)]

    use proptest::prelude::*;

    use super::*;

    fn soundboard() -> Soundboard {
        let rate = SampleRate::new(48_000.0).expect("48 kHz is valid");
        Soundboard::new(rate)
    }

    #[test]
    fn silence_in_is_silence_out() {
        let mut board = soundboard();
        for _ in 0..1_000 {
            assert_eq!(board.process(0.0), 0.0);
        }
    }

    #[test]
    fn an_impulse_produces_a_ringing_tail() {
        let mut board = soundboard();
        board.process(1.0);
        let mut peak_after_impulse = 0.0f32;
        for _ in 0..2_000 {
            peak_after_impulse = peak_after_impulse.max(math::abs(board.process(0.0)));
        }
        assert!(
            peak_after_impulse > 1e-3,
            "peak {peak_after_impulse} did not ring"
        );
    }

    #[test]
    fn the_ring_eventually_decays_to_silence() {
        let mut board = soundboard();
        board.process(1.0);
        for _ in 0..(48_000 * 5) {
            board.process(0.0);
        }
        let tail = (0..128)
            .map(|_| math::abs(board.process(0.0)))
            .fold(0.0f32, f32::max);
        assert!(tail < 1e-4, "tail {tail} has not decayed");
    }

    #[test]
    fn output_stays_bounded_for_a_full_second() {
        let mut board = soundboard();
        for index in 0..48_000 {
            let input = if index == 0 { 1.0 } else { 0.0 };
            let sample = board.process(input);
            assert!(sample.is_finite(), "sample {index} was not finite");
            assert!(sample.abs() < 8.0, "sample {index} = {sample} escaped");
        }
    }

    /// Drives one mode alone with a unit sine at `frequency_hz` and returns
    /// the steady-state output amplitude — the resonator's actual gain,
    /// measured rather than derived from its coefficients.
    fn measured_gain_at(mode: SoundboardMode, frequency_hz: f32) -> f32 {
        const RATE: f32 = 48_000.0;
        let mut board = soundboard();
        for index in 0..MODE_COUNT {
            board.set_mode(
                index,
                SoundboardMode {
                    gain: 0.0,
                    ..DEFAULT_MODES[0]
                },
            );
        }
        board.set_mode(0, mode);

        // Six time constants of settling puts the transient at least 300 dB
        // down, so what is measured afterwards is steady state alone.
        let settle = (RATE * mode.decay_seconds * 6.0) as usize + 1;
        let omega = core::f32::consts::TAU * frequency_hz / RATE;
        let mut peak = 0.0f32;
        for step in 0..settle + (RATE as usize / 4) {
            let output = board.process(math::sin(omega * step as f32));
            if step >= settle {
                peak = peak.max(math::abs(output));
            }
        }
        peak
    }

    /// The regression test for the defect behind the report "the mid-register
    /// strings sound bad; something metallic is knocking".
    ///
    /// `Resonator::new` promises a peak gain of `gain` at the mode's own
    /// frequency. Before `resonant_peak_reciprocal` existed it delivered
    /// `47.75` for a requested `1.0` at 80 Hz and `0.69` for a requested
    /// `0.25` at 1400 Hz — a frequency-dependent boost of up to 34 dB, which
    /// put a fixed low thump and seven foreign pitches under every note
    /// played. Measured, not derived: the algebra is exactly what was wrong
    /// before, so asserting against a rendered signal is the point.
    #[test]
    fn every_mode_resonates_at_the_gain_it_was_asked_for() {
        for mode in DEFAULT_MODES {
            let measured = measured_gain_at(mode, mode.frequency_hz);
            let ratio = measured / mode.gain;
            assert!(
                (0.7..1.4).contains(&ratio),
                "mode at {} Hz was asked for a gain of {} and resonates at {measured} \
                 ({:+.1} dB off)",
                mode.frequency_hz,
                mode.gain,
                20.0 * math::ln(ratio) / core::f32::consts::LN_10 * 2.0
            );
        }
    }

    /// The same promise, held across the whole range a caller can live-set —
    /// not only at the eight frequencies the default table happens to use.
    /// A normalisation that is right for the shipped modes and wrong
    /// elsewhere would still ambush anyone editing a mode in the studio.
    #[test]
    fn the_gain_promise_holds_across_the_whole_settable_frequency_range() {
        for frequency_hz in [20.0f32, 55.0, 110.0, 440.0, 1_000.0, 4_000.0, 9_000.0] {
            let mode = SoundboardMode {
                frequency_hz,
                decay_seconds: decay_seconds_for_q(frequency_hz, DEFAULT_MODE_Q),
                gain: 0.5,
            };
            let measured = measured_gain_at(mode, frequency_hz);
            let ratio = measured / mode.gain;
            assert!(
                (0.7..1.4).contains(&ratio),
                "a mode at {frequency_hz} Hz asked for gain 0.5 resonates at {measured}"
            );
        }
    }

    /// The other half of the same report: these are meant to be a wooden
    /// box's resonances, not a tuned bell's. The shipped table used to work
    /// out to `Q` between 302 and 1100; a ribbed spruce soundboard's modal
    /// `Q` is roughly 20-50 (see [`DEFAULT_MODE_Q`]). The band is
    /// deliberately wider than that on both sides — this guards against the
    /// table drifting back into bell territory, not against re-voicing it.
    #[test]
    fn no_default_mode_rings_like_a_bell() {
        for mode in DEFAULT_MODES {
            let q = core::f32::consts::PI * mode.frequency_hz * mode.decay_seconds;
            assert!(
                (10.0..80.0).contains(&q),
                "the mode at {} Hz has Q={q:.0}; a soundboard is 20-50, a bell is in \
                 the hundreds or thousands",
                mode.frequency_hz
            );
        }
    }

    /// `MIN_DECAY_SECONDS` must not silently turn a heavily damped high mode
    /// into a ringing one. Clamping a 5 kHz mode's 1.9 ms decay up to the old
    /// 10 ms floor would have quintupled its `Q` behind the caller's back.
    #[test]
    fn a_heavily_damped_high_mode_is_expressible_rather_than_clamped_into_ringing() {
        let frequency_hz = 5_000.0;
        let decay_seconds = decay_seconds_for_q(frequency_hz, DEFAULT_MODE_Q);
        assert!(
            decay_seconds > MIN_DECAY_SECONDS,
            "a {DEFAULT_MODE_Q}-Q mode at {frequency_hz} Hz needs {decay_seconds}s, \
             below the {MIN_DECAY_SECONDS}s floor"
        );
    }

    #[test]
    fn set_mode_on_an_out_of_range_index_is_a_silent_no_op() {
        let mut board = soundboard();
        board.set_mode(
            MODE_COUNT + 5,
            SoundboardMode {
                frequency_hz: 5_000.0,
                decay_seconds: 2.0,
                gain: 3.0,
            },
        );
        board.process(1.0);
        for _ in 0..1_000 {
            assert!(board.process(0.0).is_finite());
        }
    }

    #[test]
    fn set_mode_changes_which_frequency_rings() {
        let mut low = soundboard();
        let mut high = soundboard();
        high.set_mode(
            0,
            SoundboardMode {
                frequency_hz: 10_000.0,
                decay_seconds: 0.3,
                gain: 1.0,
            },
        );
        low.process(1.0);
        high.process(1.0);
        let low_tail: Vec<f32> = (0..256).map(|_| low.process(0.0)).collect();
        let high_tail: Vec<f32> = (0..256).map(|_| high.process(0.0)).collect();
        let difference: f32 = low_tail
            .iter()
            .zip(high_tail.iter())
            .map(|(a, b)| (a - b).abs())
            .sum();
        assert!(
            difference > 1e-3,
            "retuning mode 0 did not change the ring: total diff {difference}"
        );
    }

    proptest! {
        /// Whatever mode a caller live-sets, including NaN and +-infinity
        /// fields and an out-of-range index, the board must stay finite.
        #[test]
        fn set_mode_is_total(
            index in 0usize..MODE_COUNT + 2,
            frequency_hz in proptest::num::f32::ANY,
            decay_seconds in proptest::num::f32::ANY,
            gain in proptest::num::f32::ANY,
        ) {
            let mut board = soundboard();
            board.set_mode(index, SoundboardMode { frequency_hz, decay_seconds, gain });
            board.process(1.0);
            for _ in 0..256 {
                prop_assert!(board.process(0.0).is_finite());
            }
        }

        /// Whatever input sequence a caller drives the soundboard with,
        /// including NaN and +-infinity, it must never panic or produce a
        /// non-finite value.
        #[test]
        fn any_input_sequence_is_total(inputs in proptest::collection::vec(proptest::num::f32::ANY, 0..500)) {
            let mut board = soundboard();
            for input in inputs {
                let output = board.process(input);
                prop_assert!(output.is_finite());
            }
        }

        /// Whatever sample rate a caller builds a soundboard with (within
        /// the range `SampleRate` accepts), construction must produce a
        /// stable bank: an impulse followed by silence must decay rather
        /// than diverge.
        #[test]
        fn any_valid_sample_rate_is_stable(hertz in 4_000.0f32..200_000.0) {
            let rate = SampleRate::new(hertz).expect("generated rate is valid");
            let mut board = Soundboard::new(rate);
            board.process(1.0);
            for _ in 0..2_000 {
                let sample = board.process(0.0);
                prop_assert!(sample.is_finite());
                prop_assert!(sample.abs() < 8.0);
            }
        }
    }
}
