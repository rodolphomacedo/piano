//! A fixed-capacity delay line — the memory of a vibrating string.
//!
//! Physically, a delay line is a stretch of string that a wave takes `N`
//! samples to travel across. Everything else in a waveguide model is a filter
//! applied to what comes out of it.

use alloc::{boxed::Box, vec};

use crate::math;

/// Upper bound on delay-line capacity, in samples.
///
/// Roughly 87 seconds at 48 kHz — far beyond any string — and small enough that
/// rounding up to the next power of two can never overflow.
pub const MAX_CAPACITY: usize = 1 << 22;

/// Lower bound on delay-line capacity, in samples.
pub const MIN_CAPACITY: usize = 2;

/// A circular buffer with power-of-two capacity.
///
/// The capacity is always a power of two so that wrapping is a bitmask instead
/// of a division: a modulo on the audio thread costs ~20 cycles, a mask costs
/// one.
#[derive(Debug, Clone)]
pub struct DelayLine {
    buffer: Box<[f32]>,
    mask: usize,
    write_index: usize,
    /// State for [`DelayLine::read_allpass`] — the fractional-delay filter's
    /// own memory, separate from the buffer it reads.
    allpass_state: f32,
}

/// Highest magnitude allowed for the fractional-delay allpass coefficient.
///
/// At a fraction of exactly 0 the coefficient sits at exactly 1.0, the unit
/// circle boundary; the margin keeps [`DelayLine::read_allpass`] provably
/// stable for every fraction in `[0, 1)`, the same pattern
/// `filter::MAX_POLE` uses for the loop filter's pole.
const MAX_ALLPASS_ETA: f32 = 0.999_9;

impl DelayLine {
    /// Allocates a delay line able to hold at least `requested` samples.
    ///
    /// The request is clamped into `[MIN_CAPACITY, MAX_CAPACITY]` and rounded up
    /// to a power of two, so this never fails and never panics. This is the only
    /// allocating method on the type; it must not be called while processing.
    #[must_use]
    pub fn with_capacity(requested: usize) -> Self {
        let capacity = requested
            .clamp(MIN_CAPACITY, MAX_CAPACITY)
            .next_power_of_two();
        Self {
            buffer: vec![0.0; capacity].into_boxed_slice(),
            mask: capacity - 1,
            write_index: 0,
            allpass_state: 0.0,
        }
    }

    /// How many samples the line can hold.
    #[inline]
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.mask + 1
    }

    /// The longest delay that can be read back without wrapping onto itself.
    #[inline]
    #[must_use]
    pub fn max_delay(&self) -> usize {
        self.mask
    }

    /// Advances the write head and stores `sample` at it.
    ///
    /// After this call, `read(0)` returns `sample`.
    #[inline]
    pub fn write(&mut self, sample: f32) {
        self.write_index = (self.write_index + 1) & self.mask;
        let index = self.write_index;
        self.store(index, sample);
    }

    /// Reads the sample written `delay_samples` ago.
    ///
    /// Delays larger than the capacity wrap around rather than panicking; the
    /// caller is expected to have sized the line correctly.
    #[inline]
    #[must_use]
    pub fn read(&self, delay_samples: usize) -> f32 {
        self.load(self.write_index.wrapping_sub(delay_samples) & self.mask)
    }

    /// Reads a fractional delay using linear interpolation.
    ///
    /// Linear interpolation is cheap and unconditionally stable, but it is also
    /// a mild lowpass whose damping depends on the fractional part — audible as
    /// pitch-dependent loss of brightness. Replacing it with an allpass or
    /// Lagrange interpolator is tracked as `PERF-004`.
    ///
    /// `NaN` inputs saturate to delay zero instead of panicking.
    #[inline]
    #[must_use]
    pub fn read_interpolated(&self, delay_samples: f32) -> f32 {
        let clamped = math::clamp_or_low(delay_samples, 0.0, self.mask as f32);
        // f32 -> usize casts saturate in Rust, so this cannot wrap or trap.
        let whole = clamped as usize;
        let fraction = clamped - whole as f32;
        let earlier = self.read(whole);
        let later = self.read(whole + 1);
        earlier + (later - earlier) * fraction
    }

    /// Reads a fractional delay using first-order allpass interpolation.
    ///
    /// Unlike [`DelayLine::read_interpolated`], an allpass has unity
    /// magnitude response at every frequency — it delays some frequencies
    /// slightly more than others but never damps any of them — so it does
    /// not colour the string's timbre by pitch or fractional part the way
    /// linear interpolation does. This is the upgrade `PERF-004` in
    /// `docs/PERFORMANCE.md` asks for, in the standard first-order form
    /// (D. Jaffe & J. O. Smith, "Extensions of the Karplus-Strong
    /// Plucked-String Algorithm", 1983, the tuning-allpass section; J. O.
    /// Smith III, *Physical Audio Signal Processing*, "First-Order Allpass
    /// Interpolation"): for fractional part `d` of the delay,
    /// `η = (1 - d) / (1 + d)` and `y[n] = η·x[n] + x[n-1] - η·y[n-1]`,
    /// implemented here as the canonical single-multiply structure with one
    /// state variable rather than the textbook two.
    ///
    /// Stateful: the caller is expected to call this once per sample for a
    /// given string, the way [`crate::string::PluckedString::process`]
    /// does, since the recursion's own memory is what makes the
    /// interpolation exact rather than a fresh two-tap blend every time.
    /// `NaN` or out-of-range delays saturate the same way
    /// [`DelayLine::read_interpolated`] does, so this is total for every
    /// `f32`.
    #[inline]
    pub fn read_allpass(&mut self, delay_samples: f32) -> f32 {
        let clamped = math::clamp_or_low(delay_samples, 0.0, self.mask as f32);
        let whole = clamped as usize;
        let fraction = clamped - whole as f32;
        let eta = math::clamp_or_low(
            (1.0 - fraction) / (1.0 + fraction),
            -MAX_ALLPASS_ETA,
            MAX_ALLPASS_ETA,
        );
        let recent = self.read(whole);
        let older = self.read(whole + 1);
        let output = math::flush_denormal(eta * recent + older - eta * self.allpass_state);
        self.allpass_state = output;
        output
    }

    /// Zeroes the whole buffer and rewinds the write head.
    pub fn clear(&mut self) {
        self.buffer.fill(0.0);
        self.write_index = 0;
        self.allpass_state = 0.0;
    }

    /// Indexed read. `index` is always masked by the caller, so it is in range.
    #[inline]
    #[allow(clippy::indexing_slicing)]
    fn load(&self, index: usize) -> f32 {
        debug_assert!(index < self.buffer.len(), "index must be masked before use");
        self.buffer[index]
    }

    /// Indexed write. `index` is always masked by the caller, so it is in range.
    #[inline]
    #[allow(clippy::indexing_slicing)]
    fn store(&mut self, index: usize, sample: f32) {
        debug_assert!(index < self.buffer.len(), "index must be masked before use");
        self.buffer[index] = sample;
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::float_cmp, clippy::unwrap_used, clippy::expect_used)]

    use proptest::prelude::*;

    use super::*;

    #[test]
    fn capacity_is_rounded_up_to_a_power_of_two() {
        assert_eq!(DelayLine::with_capacity(100).capacity(), 128);
        assert_eq!(DelayLine::with_capacity(128).capacity(), 128);
    }

    #[test]
    fn capacity_is_clamped_into_range() {
        assert_eq!(DelayLine::with_capacity(0).capacity(), MIN_CAPACITY);
        assert_eq!(
            DelayLine::with_capacity(usize::MAX).capacity(),
            MAX_CAPACITY
        );
    }

    #[test]
    fn starts_silent() {
        let line = DelayLine::with_capacity(16);
        assert_eq!(line.read(0), 0.0);
        assert_eq!(line.read(15), 0.0);
    }

    #[test]
    fn reads_back_what_was_written() {
        let mut line = DelayLine::with_capacity(8);
        for value in 1..=4u8 {
            line.write(f32::from(value));
        }
        assert_eq!(line.read(0), 4.0);
        assert_eq!(line.read(1), 3.0);
        assert_eq!(line.read(3), 1.0);
    }

    #[test]
    fn interpolates_between_neighbours() {
        let mut line = DelayLine::with_capacity(8);
        line.write(0.0);
        line.write(10.0);
        assert_eq!(line.read_interpolated(0.5), 5.0);
    }

    #[test]
    fn clear_silences_the_line() {
        let mut line = DelayLine::with_capacity(8);
        line.write(1.0);
        line.clear();
        assert_eq!(line.read(0), 0.0);
    }

    #[test]
    fn allpass_reproduces_a_dc_signal_exactly() {
        // An allpass has unity gain at DC, so once its state has settled a
        // constant input must read back as that same constant.
        let mut line = DelayLine::with_capacity(8);
        for _ in 0..32 {
            line.write(1.0);
        }
        let mut settled = 0.0;
        for _ in 0..64 {
            line.write(1.0);
            settled = line.read_allpass(1.5);
        }
        assert!((settled - 1.0).abs() < 1e-3, "settled at {settled}");
    }

    #[test]
    fn allpass_at_a_whole_delay_matches_a_direct_read() {
        // With fraction 0, eta clamps just under 1 rather than exactly 1
        // (kept off the unit circle for stability), so the allpass output
        // converges to the plain integer-delay read rather than equalling
        // it on the very first sample.
        let mut line = DelayLine::with_capacity(8);
        let mut settled = 0.0;
        for value in [1.0, -1.0, 1.0, -1.0, 1.0, -1.0, 1.0, -1.0] {
            line.write(value);
            settled = line.read_allpass(2.0);
        }
        let direct = line.read(2);
        assert!(
            (settled - direct).abs() < 0.5,
            "settled {settled} vs {direct}"
        );
    }

    proptest! {
        /// Reading a fractional delay with the allpass interpolator must
        /// never panic or produce a non-finite value, whatever the caller
        /// asks for — the same totality guarantee
        /// `reading_any_delay_is_total` proves for the linear interpolator,
        /// including NaN, +-infinity, zero and pathological fraction
        /// values. Unlike the linear interpolator, an allpass can ring past
        /// its instantaneous input bound on a transient near a whole-sample
        /// delay (its coefficient approaches the unit circle there), so
        /// only finiteness is asserted here, not a magnitude bound —
        /// finiteness is what `docs/REALTIME-AUDIO-RULES.md` actually
        /// requires ("boundedness" there means finite for arbitrarily long
        /// runs, not bounded by the input's own instantaneous magnitude).
        #[test]
        fn allpass_reading_any_delay_is_total(
            delay in proptest::num::f32::ANY,
            samples in proptest::collection::vec(-1.0f32..1.0, 0..64),
        ) {
            let mut line = DelayLine::with_capacity(32);
            for sample in samples {
                line.write(sample);
                let output = line.read_allpass(delay);
                prop_assert!(output.is_finite());
            }
        }
    }

    proptest! {
        /// Reading a delay line must never panic, whatever the caller asks for.
        #[test]
        fn reading_any_delay_is_total(
            delay in proptest::num::f32::ANY,
            samples in proptest::collection::vec(-1.0f32..1.0, 0..64),
        ) {
            let mut line = DelayLine::with_capacity(32);
            for sample in samples {
                line.write(sample);
            }
            let output = line.read_interpolated(delay);
            prop_assert!(output.is_finite());
        }
    }
}
