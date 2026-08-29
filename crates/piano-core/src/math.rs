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
