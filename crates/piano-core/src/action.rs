//! The piano action's own noise: the thump of a key hitting its keybed.
//!
//! A. Askenfelt, "Observations on the transient components of the piano
//! tone" (STL-QPSR 34(4), 1993), separates a struck note's onset into the
//! string's tone and two mechanical transients that never pass through a
//! string: the finger meeting the key and, larger, the key bottoming on the
//! keybed. The keybed impact drives the instrument's body directly — it is
//! heard as a dull knock *under* the tone, a few milliseconds into the note
//! (Askenfelt & Jansson, "From touch to string vibrations", JASA 1990, time
//! the key bottoming within a few milliseconds of hammer-string contact),
//! stronger and shorter the harder the key is played, and most audible in
//! the treble, where the string's own tone is thin and brief. Without it a
//! synthesised piano note starts like a plucked string in free space rather
//! than a mechanism in a wooden case.
//!
//! [`ActionNoise`] synthesises — never samples — that knock as a smooth
//! half-sine force pulse, which the caller feeds into the soundboard rather
//! than into any string: the board's modes give it the body's colour. A
//! fixed pool of [`MAX_PENDING_THUMPS`] pulses keeps it allocation-free and
//! bounded on the audio thread.

use crate::math;

/// Pulses that can be pending or sounding at once. A thump lasts a few
/// milliseconds, so even a fast two-handed passage keeps only a handful
/// alive; a strike that finds the pool full replaces the oldest pulse.
pub const MAX_PENDING_THUMPS: usize = 16;

/// How long after the hammer meets the string the key reaches its bed.
const KEY_BOTTOM_DELAY_SECONDS: f32 = 0.003;

/// Keybed impact duration for the softest strike, in seconds — felt
/// against wood, compressed gently.
const SOFT_THUMP_SECONDS: f32 = 0.004;

/// Keybed impact duration for the hardest strike, in seconds.
const HARD_THUMP_SECONDS: f32 = 0.0015;

/// Peak thump force at full velocity, in the engine's output units. Set by
/// measurement so a *fortissimo* A4's knock, rendered through the default
/// soundboard, sits about 25 dB under the note over its first 50 ms —
/// present as a knock, never competing with the tone (see `piano-audio`'s
/// `action_noise` test). The board's modes pass only a little of a pulse
/// this short, which is why the force itself is large.
pub const DEFAULT_THUMP_GAIN: f32 = 0.8;

/// Largest thump level [`ActionNoise::set_gain`] accepts — about 20 dB over
/// the default, for a deliberately clattery action.
pub const MAX_THUMP_GAIN: f32 = 8.0;

/// A damper landing's level relative to a full-velocity keybed knock. The
/// felt drops onto the strings under its own spring and weight, the same
/// way however the key was played, so it is fixed — and soft: a muffled
/// touch on the string, not a knock.
const DAMPER_THUMP_FRACTION: f32 = 0.05;

/// How long a damper landing's pulse lasts, in seconds: soft felt meeting
/// a moving string compresses more slowly than a key meeting its bed.
const DAMPER_THUMP_SECONDS: f32 = 0.006;

/// How steeply the knock grows with velocity. Impact *energy* grows with
/// the square of key speed, so amplitude grows as its first power at least;
/// the key also bottoms out harder relative to the hammer's own travel as a
/// player presses through the key, so the knock is well under the tone at
/// *piano* and plainly there at *forte*.
const THUMP_VELOCITY_EXPONENT: f32 = 1.5;

#[derive(Debug, Clone, Copy, Default)]
struct Thump {
    /// Samples until the pulse starts.
    delay: u32,
    /// Samples into the pulse.
    elapsed: u32,
    /// Pulse length in samples; `0` marks a free slot.
    length: u32,
    amplitude: f32,
}

/// A bounded pool of keybed thumps — see the module docs.
#[derive(Debug, Clone, Copy)]
pub struct ActionNoise {
    thumps: [Thump; MAX_PENDING_THUMPS],
    sample_rate: f32,
    gain: f32,
    next_slot: usize,
}

impl ActionNoise {
    /// An idle pool for `sample_rate_hz`, at [`DEFAULT_THUMP_GAIN`].
    #[must_use]
    pub fn new(sample_rate_hz: f32) -> Self {
        Self {
            thumps: [Thump::default(); MAX_PENDING_THUMPS],
            sample_rate: math::clamp_or_low(sample_rate_hz, 1.0, f32::MAX),
            gain: DEFAULT_THUMP_GAIN,
            next_slot: 0,
        }
    }

    /// Sets the thump's peak level at full velocity; `0` silences the
    /// action. Clamped into `[0, MAX_THUMP_GAIN]`, `NaN` to `0`.
    pub fn set_gain(&mut self, gain: f32) {
        self.gain = math::clamp_or_low(gain, 0.0, MAX_THUMP_GAIN);
    }

    /// Schedules the keybed thump of a key struck at `velocity` (`[0, 1]`).
    pub fn strike(&mut self, velocity: f32) {
        let velocity = math::clamp_or_low(velocity, 0.0, 1.0);
        let seconds = SOFT_THUMP_SECONDS + (HARD_THUMP_SECONDS - SOFT_THUMP_SECONDS) * velocity;
        self.schedule(Thump {
            delay: samples_for(KEY_BOTTOM_DELAY_SECONDS, self.sample_rate),
            elapsed: 0,
            length: samples_for(seconds, self.sample_rate).max(1),
            amplitude: self.gain * math::powf(velocity, THUMP_VELOCITY_EXPONENT),
        });
    }

    /// Schedules the soft thump of a damper landing on a sounding string —
    /// a key coming up, or the sustain pedal letting go.
    pub fn damp(&mut self) {
        self.schedule(Thump {
            delay: 0,
            elapsed: 0,
            length: samples_for(DAMPER_THUMP_SECONDS, self.sample_rate).max(1),
            amplitude: self.gain * DAMPER_THUMP_FRACTION,
        });
    }

    /// Puts `thump` in the next slot, replacing the oldest if all are busy.
    fn schedule(&mut self, thump: Thump) {
        let slot = self.next_slot % MAX_PENDING_THUMPS;
        self.next_slot = (slot + 1) % MAX_PENDING_THUMPS;
        if let Some(free) = self.thumps.get_mut(slot) {
            *free = thump;
        }
    }

    /// The summed force of every sounding thump this sample, advancing each.
    #[inline]
    pub fn next_sample(&mut self) -> f32 {
        self.thumps.iter_mut().map(advance).sum()
    }
}

/// One sample of `thump`'s half-sine pulse, advancing it; `0.0` while it
/// waits and once it has finished (which also frees the slot).
#[inline]
fn advance(thump: &mut Thump) -> f32 {
    if thump.length == 0 {
        return 0.0;
    }
    if thump.delay > 0 {
        thump.delay -= 1;
        return 0.0;
    }
    let phase = core::f32::consts::PI * thump.elapsed as f32 / thump.length as f32;
    thump.elapsed += 1;
    if thump.elapsed >= thump.length {
        thump.length = 0;
    }
    thump.amplitude * math::sin(phase)
}

/// `seconds` at `sample_rate`, rounded, saturating rather than wrapping.
fn samples_for(seconds: f32, sample_rate: f32) -> u32 {
    let samples = math::round(seconds * sample_rate);
    if samples >= u32::MAX as f32 {
        u32::MAX
    } else {
        math::clamp_or_low(samples, 0.0, f32::MAX) as u32
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::float_cmp)]

    use super::*;
    use proptest::prelude::*;

    const RATE: f32 = 48_000.0;

    fn render(noise: &mut ActionNoise) -> [f32; 1024] {
        let mut out = [0.0f32; 1024];
        for slot in &mut out {
            *slot = noise.next_sample();
        }
        out
    }

    #[test]
    fn a_strike_knocks_once_after_the_key_bottom_delay_and_then_falls_silent() {
        let mut noise = ActionNoise::new(RATE);
        noise.strike(1.0);
        let out = render(&mut noise);
        let delay = samples_for(KEY_BOTTOM_DELAY_SECONDS, RATE) as usize;
        assert!(out.iter().take(delay).all(|&sample| sample == 0.0));
        assert!(out.iter().any(|&sample| sample > 0.0));
        assert!(out.iter().skip(delay + 100).all(|&sample| sample == 0.0));
    }

    #[test]
    fn a_harder_strike_knocks_harder_and_shorter() {
        let peak_and_length = |velocity: f32| {
            let mut noise = ActionNoise::new(RATE);
            noise.strike(velocity);
            let out = render(&mut noise);
            let peak = out.iter().copied().fold(0.0f32, f32::max);
            (peak, out.iter().filter(|&&sample| sample > 0.0).count())
        };
        let (soft_peak, soft_length) = peak_and_length(0.3);
        let (hard_peak, hard_length) = peak_and_length(1.0);
        assert!(hard_peak > 3.0 * soft_peak, "{soft_peak} {hard_peak}");
        assert!(hard_length < soft_length, "{soft_length} {hard_length}");
    }

    #[test]
    fn a_damper_landing_is_immediate_and_much_softer_than_a_hard_strike() {
        let peak = |noise: &mut ActionNoise| render(noise).iter().copied().fold(0.0f32, f32::max);
        let mut damper = ActionNoise::new(RATE);
        damper.damp();
        let first = damper.next_sample();
        let damper_peak = peak(&mut damper).max(first);
        let mut strike = ActionNoise::new(RATE);
        strike.strike(1.0);
        let strike_peak = peak(&mut strike);
        assert!(damper_peak > 0.0);
        assert!(
            damper_peak < 0.1 * strike_peak,
            "{damper_peak} {strike_peak}"
        );
    }

    #[test]
    fn a_zero_gain_action_is_silent() {
        let mut noise = ActionNoise::new(RATE);
        noise.set_gain(0.0);
        noise.strike(1.0);
        assert!(render(&mut noise).iter().all(|&sample| sample == 0.0));
    }

    proptest! {
        #[test]
        fn any_strikes_stay_bounded(
            velocities in proptest::collection::vec(proptest::num::f32::ANY, 0..40),
            rate in proptest::num::f32::ANY,
        ) {
            let mut noise = ActionNoise::new(rate);
            for velocity in velocities {
                noise.strike(velocity);
                let sample = noise.next_sample();
                prop_assert!(sample.is_finite());
                prop_assert!(sample.abs() <= MAX_THUMP_GAIN * MAX_PENDING_THUMPS as f32);
            }
        }
    }
}
