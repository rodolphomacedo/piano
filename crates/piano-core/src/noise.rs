//! A deterministic noise source for string excitation.
//!
//! The excitation is noise, but the tests are not allowed to be: a seeded
//! xorshift gives byte-identical output for a given seed, so a rendered note can
//! be compared against a reference. It is also branch-free and allocation-free,
//! unlike anything from `rand`'s default stack.

/// A 32-bit xorshift generator (Marsaglia, 2003).
#[derive(Debug, Clone, Copy)]
pub struct Xorshift32 {
    state: u32,
}

impl Default for Xorshift32 {
    fn default() -> Self {
        Self::new(0x2545_F491)
    }
}

impl Xorshift32 {
    /// Seeds the generator. A zero seed is replaced, because xorshift is stuck
    /// at zero forever.
    #[must_use]
    pub const fn new(seed: u32) -> Self {
        Self {
            state: if seed == 0 { 0x9E37_79B9 } else { seed },
        }
    }

    /// Returns the next raw 32-bit value.
    #[inline]
    pub const fn next_u32(&mut self) -> u32 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.state = x;
        x
    }

    /// Returns the next sample uniformly distributed in `[-1, 1)`.
    #[inline]
    pub fn next_bipolar(&mut self) -> f32 {
        // 2^-31 scaling of a u32 lands in [0, 2); subtracting one centres it.
        (self.next_u32() as f32) * (1.0 / 2_147_483_648.0) - 1.0
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::float_cmp)]

    use super::*;

    #[test]
    fn is_reproducible_for_a_given_seed() {
        let mut left = Xorshift32::new(7);
        let mut right = Xorshift32::new(7);
        for _ in 0..64 {
            assert_eq!(left.next_u32(), right.next_u32());
        }
    }

    #[test]
    fn zero_seed_does_not_stick() {
        let mut rng = Xorshift32::new(0);
        assert_ne!(rng.next_u32(), 0);
    }

    #[test]
    fn stays_within_the_bipolar_range() {
        let mut rng = Xorshift32::default();
        for _ in 0..10_000 {
            let sample = rng.next_bipolar();
            assert!(
                (-1.0..1.0).contains(&sample),
                "sample {sample} out of range"
            );
        }
    }

    #[test]
    fn has_roughly_zero_mean() {
        let mut rng = Xorshift32::default();
        let count = 100_000;
        let sum: f32 = (0..count).map(|_| rng.next_bipolar()).sum();
        let mean = sum / count as f32;
        assert!(mean.abs() < 0.02, "mean {mean} is too far from zero");
    }
}
