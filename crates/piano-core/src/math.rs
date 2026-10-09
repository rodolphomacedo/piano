//! Float math that works with and without `std`.
//!
//! `core` does not provide transcendental functions, so every call site in this
//! crate goes through this module: it forwards to the `std` inherent methods
//! when they are available and to `libm` otherwise. Routing everything through
//! one module is also what keeps the `no_std` build honest — a stray `x.exp()`
//! elsewhere would only break on the WASM/embedded target, long after it was
//! written.

macro_rules! float_fn {
    ($(#[$doc:meta])* $name:ident, $method:ident, $fallback:path) => {
        $(#[$doc])*
        #[inline]
        #[must_use]
        pub fn $name(x: f32) -> f32 {
            #[cfg(feature = "std")]
            {
                x.$method()
            }
            #[cfg(not(feature = "std"))]
            {
                $fallback(x)
            }
        }
    };
}

float_fn!(
    /// Absolute value.
    abs, abs, libm::fabsf
);
float_fn!(
    /// Natural exponential, `e^x`.
    exp, exp, libm::expf
);
float_fn!(
    /// Natural logarithm.
    ln, ln, libm::logf
);
float_fn!(
    /// Square root.
    sqrt, sqrt, libm::sqrtf
);
float_fn!(
    /// Sine of an angle in radians.
    sin, sin, libm::sinf
);
float_fn!(
    /// Cosine of an angle in radians.
    cos, cos, libm::cosf
);
float_fn!(
    /// Tangent of an angle in radians.
    tan, tan, libm::tanf
);
float_fn!(
    /// Rounds to the nearest integer, half away from zero.
    round, round, libm::roundf
);
float_fn!(
    /// Hyperbolic tangent — used as a soft-clip curve, not for any physical
    /// model: linear near zero, saturating smoothly to `±1` beyond it.
    tanh, tanh, libm::tanhf
);

/// Raises `base` to `exponent`.
#[inline]
#[must_use]
pub fn powf(base: f32, exponent: f32) -> f32 {
    #[cfg(feature = "std")]
    {
        base.powf(exponent)
    }
    #[cfg(not(feature = "std"))]
    {
        libm::powf(base, exponent)
    }
}

/// Four-quadrant arctangent of `y / x`, in `(-π, π]`.
#[inline]
#[must_use]
pub fn atan2(y: f32, x: f32) -> f32 {
    #[cfg(feature = "std")]
    {
        y.atan2(x)
    }
    #[cfg(not(feature = "std"))]
    {
        libm::atan2f(y, x)
    }
}

/// Phase delay, in samples, of the first-order allpass
/// `H(z) = (a + z⁻¹) / (1 + a·z⁻¹)` at `omega` radians per sample.
///
/// Both the dispersion cascade and the delay line's fractional-delay
/// interpolator are this exact section, and a string is only in tune if the
/// loop is tuned by their delay *at the fundamental* — at DC their delay is
/// larger, by enough to put A5 22 cents sharp (issue #96). Returns the DC
/// limit `(1 - a)/(1 + a)` for `omega` too small to divide by.
#[inline]
#[must_use]
pub fn allpass_phase_delay(coefficient: f32, omega: f32) -> f32 {
    if omega.is_nan() || omega <= MIN_PHASE_OMEGA {
        return (1.0 - coefficient) / (1.0 + coefficient);
    }
    let (sine, cosine) = (sin(omega), cos(omega));
    let numerator = atan2(-sine, coefficient + cosine);
    let denominator = atan2(-coefficient * sine, 1.0 + coefficient * cosine);
    -(numerator - denominator) / omega
}

/// Below this many radians per sample a phase delay is taken at its DC
/// limit: dividing a phase this small by `omega` loses all precision in `f32`.
pub const MIN_PHASE_OMEGA: f32 = 1e-4;

/// Magnitudes below this are treated as silence.
///
/// A decaying string tail eventually produces denormal floats, and on x86 every
/// denormal operation costs tens of cycles. Flushing them keeps the cost of a
/// silent voice flat instead of exploding at the end of every note.
pub const DENORMAL_FLOOR: f32 = 1e-25;

/// Replaces denormal and near-silent values with exact zero.
///
/// This is the portable, safe-Rust version. Setting the CPU's flush-to-zero mode
/// is cheaper but requires `unsafe` and is platform specific, so it lives in the
/// host layer instead — see `docs/PERFORMANCE.md`, entry `PERF-002`.
#[inline]
#[must_use]
pub fn flush_denormal(x: f32) -> f32 {
    if abs(x) < DENORMAL_FLOOR { 0.0 } else { x }
}

/// Clamps `value` into `[low, high]`, mapping `NaN` to `low`.
///
/// `f32::clamp` propagates `NaN`, which then poisons a feedback loop forever.
/// Every parameter that reaches a recursive filter goes through this instead.
#[inline]
#[must_use]
pub fn clamp_or_low(value: f32, low: f32, high: f32) -> f32 {
    if value.is_nan() || value < low {
        low
    } else if value > high {
        high
    } else {
        value
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::float_cmp)]

    use super::*;

    #[test]
    fn flushes_denormals_to_zero() {
        assert_eq!(flush_denormal(1e-30), 0.0);
        assert_eq!(flush_denormal(-1e-30), 0.0);
    }

    #[test]
    fn leaves_audible_values_untouched() {
        assert_eq!(flush_denormal(0.5), 0.5);
        assert_eq!(flush_denormal(-0.5), -0.5);
    }

    #[test]
    fn allpass_phase_delay_matches_its_dc_limit_at_low_frequency() {
        for coefficient in [-0.9, -0.5, 0.0, 0.5] {
            let dc = (1.0 - coefficient) / (1.0 + coefficient);
            let low = allpass_phase_delay(coefficient, 2e-3);
            assert!(
                (low - dc).abs() < 1e-2 * dc.max(1.0),
                "{coefficient}: {low} vs {dc}"
            );
        }
    }

    #[test]
    fn a_negative_coefficient_delays_high_frequencies_less() {
        let low = allpass_phase_delay(-0.8, 0.05);
        let high = allpass_phase_delay(-0.8, 1.0);
        assert!(high < low, "low {low} high {high}");
    }

    #[test]
    fn clamp_maps_nan_to_the_low_bound() {
        assert_eq!(clamp_or_low(f32::NAN, 0.1, 0.9), 0.1);
    }

    #[test]
    fn clamp_bounds_both_ends() {
        assert_eq!(clamp_or_low(-5.0, 0.0, 1.0), 0.0);
        assert_eq!(clamp_or_low(5.0, 0.0, 1.0), 1.0);
        assert_eq!(clamp_or_low(0.25, 0.0, 1.0), 0.25);
    }

    #[test]
    fn exp_matches_known_values() {
        assert!(abs(exp(0.0) - 1.0) < 1e-6);
        assert!(abs(ln(exp(2.0)) - 2.0) < 1e-5);
    }
}
