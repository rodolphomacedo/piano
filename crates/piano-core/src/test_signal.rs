//! Measurements shared by this crate's tests.

/// Steady-state gain of `process`, in decibels, at `frequency_hz` for a
/// stream at `rate_hz`.
///
/// A sine is run for a second so the transient has died, then the output's
/// mean square over a further second is compared with the input's. The
/// largest output sample would not do: a sine a few samples long never
/// lands a sample on its own crest.
pub(crate) fn sine_gain_db(
    rate_hz: f32,
    frequency_hz: f32,
    mut process: impl FnMut(f32) -> f32,
) -> f32 {
    let settle = rate_hz as usize;
    let step = core::f64::consts::TAU * f64::from(frequency_hz) / f64::from(rate_hz);
    let (mut power_in, mut power_out) = (0.0_f64, 0.0_f64);
    for n in 0..2 * settle {
        let input = (step * n as f64).sin() as f32;
        let output = process(input);
        if n >= settle {
            power_in += f64::from(input).powi(2);
            power_out += f64::from(output).powi(2);
        }
    }
    (10.0 * (power_out / power_in).log10()) as f32
}
