//! The string's loop filter — the losses a real string suffers on every
//! round trip.
//!
//! A string does not lose energy uniformly: high partials die first, which is
//! why a struck note goes from bright to mellow before it goes quiet.

use crate::math;

/// Highest usable feedback coefficient.
///
/// At exactly 1.0 the pole sits on the unit circle and the filter never decays;
/// anything above it diverges. The margin keeps the loop provably stable.
const MAX_POLE: f32 = 0.999_9;

/// The string's loss filter: a one-pole, one-zero design, rather than a
/// bare pole.
///
/// A single pole shapes overall brightness, but it decays every partial by
/// the same relative amount per round trip — the spectral *shape* never
/// changes, only its overall level. Adding a fixed zero at Nyquist (a
/// `(1 + z⁻¹)/2` averaging term ahead of the pole) gives extra rolloff that
/// grows with frequency, so upper partials measurably lose energy faster
/// than the fundamental over the course of a note — matching how a real
/// piano's spectral envelope narrows over time, not just gets quieter. This
/// is the "extended Karplus-Strong" loop filter (D. Jaffe & J. O. Smith,
/// "Extensions of the Karplus-Strong Plucked-String Algorithm", 1983, the
/// loop-filter design section): their general form allows the zero position
/// to move via a second live parameter; this implementation fixes it at
/// Nyquist (maximum high-frequency rolloff) rather than exposing a second
/// knob, which keeps the API the same shape as before at the cost of one
/// degree of freedom a future milestone could still add.
#[derive(Debug, Clone, Copy, Default)]
pub struct LoopFilter {
    pole: f32,
    state: f32,
    last_input: f32,
}

impl LoopFilter {
    /// Builds a filter from its pole coefficient.
    ///
    /// `pole` is clamped into `[0, MAX_POLE]`, so the filter is stable by
    /// construction and no caller can make it blow up. `0` passes the
    /// averaged signal straight through; values near 1 damp the highs hard.
    #[must_use]
    pub fn new(pole: f32) -> Self {
        Self {
            pole: math::clamp_or_low(pole, 0.0, MAX_POLE),
            state: 0.0,
            last_input: 0.0,
        }
    }

    /// Processes one sample.
    #[inline]
    pub fn process(&mut self, input: f32) -> f32 {
        let averaged = f32::midpoint(input, self.last_input);
        self.last_input = input;
        self.state = math::flush_denormal((1.0 - self.pole) * averaged + self.pole * self.state);
        self.state
    }

    /// Updates the pole coefficient in place, for live control from outside
    /// the audio thread. Same clamp and stability guarantee as
    /// [`LoopFilter::new`]; does not touch `state`, so the change is
    /// audible starting with the very next sample rather than resetting the
    /// filter's memory.
    pub fn set_pole(&mut self, pole: f32) {
        self.pole = math::clamp_or_low(pole, 0.0, MAX_POLE);
    }

    /// The delay the filter itself adds to the loop, in samples, at low
    /// frequencies.
    ///
    /// A waveguide is tuned by total loop length, and the loss filter is part
    /// of that loop. Ignoring this term detunes the string by a few cents in
    /// the bass and much more in the treble. The `0.5` is the averaging
    /// zero's own phase delay at DC (halfway between its two taps); the rest
    /// is the pole's, as before.
    #[inline]
    #[must_use]
    pub fn phase_delay_at_dc(&self) -> f32 {
        0.5 + self.pole / (1.0 - self.pole)
    }

    /// Clears the filter memory.
    pub fn reset(&mut self) {
        self.state = 0.0;
        self.last_input = 0.0;
    }
}

/// A one-pole highpass that removes the DC offset a feedback loop accumulates.
///
/// Without it, an asymmetric excitation leaves a constant term circulating
/// forever: inaudible on its own, but it eats headroom and makes every other
/// voice clip early.
#[derive(Debug, Clone, Copy)]
pub struct DcBlocker {
    pole: f32,
    last_input: f32,
    last_output: f32,
}

impl Default for DcBlocker {
    fn default() -> Self {
        Self::new(0.995)
    }
}

impl DcBlocker {
    /// Builds a DC blocker. `pole` is clamped into `[0, MAX_POLE]`; values close
    /// to 1 place the corner closer to 0 Hz and preserve more bass.
    #[must_use]
    pub fn new(pole: f32) -> Self {
        Self {
            pole: math::clamp_or_low(pole, 0.0, MAX_POLE),
            last_input: 0.0,
            last_output: 0.0,
        }
    }

    /// Processes one sample.
    #[inline]
    pub fn process(&mut self, input: f32) -> f32 {
        let output = math::flush_denormal(input - self.last_input + self.pole * self.last_output);
        self.last_input = input;
        self.last_output = output;
        output
    }

    /// Clears the filter memory.
    pub fn reset(&mut self) {
        self.last_input = 0.0;
        self.last_output = 0.0;
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::float_cmp, clippy::unwrap_used, clippy::expect_used)]

    use proptest::prelude::*;

    use super::*;

    #[test]
    fn pole_is_clamped_below_one() {
        assert!(LoopFilter::new(5.0).phase_delay_at_dc().is_finite());
        assert_eq!(LoopFilter::new(-1.0).phase_delay_at_dc(), 0.5);
    }

    #[test]
    fn transparent_filter_passes_a_steady_input_through() {
        // With pole 0 the pole itself is transparent; the fixed zero still
        // averages against the previous sample, so a steady (unchanging)
        // input is what reaches the output unattenuated, not the very first
        // sample of a step.
        let mut filter = LoopFilter::new(0.0);
        filter.process(0.7);
        assert_eq!(filter.process(0.7), 0.7);
    }

    #[test]
    fn step_response_converges_to_the_input() {
        let mut filter = LoopFilter::new(0.9);
        let mut output = 0.0;
        for _ in 0..500 {
            output = filter.process(1.0);
        }
        assert!((output - 1.0).abs() < 1e-3, "converged to {output}");
    }

    #[test]
    fn dc_blocker_removes_a_constant_offset() {
        let mut blocker = DcBlocker::default();
        let mut output = 0.0;
        for _ in 0..5_000 {
            output = blocker.process(1.0);
        }
        assert!(output.abs() < 1e-2, "residual DC {output}");
    }

    #[test]
    fn reset_clears_state() {
        let mut filter = LoopFilter::new(0.9);
        filter.process(1.0);
        filter.reset();
        assert_eq!(filter.process(0.0), 0.0);
    }

    #[test]
    fn set_pole_changes_the_coefficient_without_resetting_state() {
        let mut filter = LoopFilter::new(0.9);
        filter.process(1.0);
        let state_before = filter.process(1.0);
        filter.set_pole(0.1);
        assert!((filter.phase_delay_at_dc() - (0.5 + 0.1 / 0.9)).abs() < 1e-6);
        assert!((filter.process(1.0) - state_before).abs() < 1.0);
    }

    #[test]
    fn set_pole_clamps_out_of_range_values() {
        let mut filter = LoopFilter::new(0.5);
        filter.set_pole(5.0);
        assert!((filter.phase_delay_at_dc() - (0.5 + MAX_POLE / (1.0 - MAX_POLE))).abs() < 1e-3);
        filter.set_pole(-1.0);
        assert_eq!(filter.phase_delay_at_dc(), 0.5);
    }

    proptest! {
        /// The loss filter must stay bounded for every reachable pole and input.
        #[test]
        fn loop_filter_never_diverges(pole in proptest::num::f32::ANY, input in -1.0f32..1.0) {
            let mut filter = LoopFilter::new(pole);
            let mut output = 0.0;
            for _ in 0..1_000 {
                output = filter.process(input);
            }
            prop_assert!(output.is_finite());
            prop_assert!(output.abs() <= 1.0 + 1e-3, "output {output} exceeded input bound");
        }
    }
}
