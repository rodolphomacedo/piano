//! A felt hammer and the string it strikes, solved together every sample.
//!
//! The hammer is a mass on a nonlinear spring: compressed by `ξ`, the felt
//! pushes back with `F = K·ξ^p`, `p ≈ 2–3` (A. Chaigne & A. Askenfelt,
//! "Numerical simulations of piano strings. I. A physical model for a struck
//! string using finite difference methods", JASA 95(2), 1994, §II). The string
//! is a waveguide: a force `F` applied at one point moves that point at
//! `F / 2Z` on top of the two travelling waves already passing through it,
//! `Z` being the string's characteristic impedance (J. O. Smith, "Physical
//! Audio Signal Processing", the struck-string and piano-hammer chapters).
//!
//! Those two statements are the whole model. Everything a listener calls
//! "touch" follows from them rather than being added: a harder blow compresses
//! the felt further, the stiffer spring throws the hammer back sooner, the
//! shorter pulse carries more high partials. Reflections that come back from
//! the agraffe while the felt is still on the string push the hammer off
//! early, which is what makes the treble's short strings bright and the long
//! bass strings round.
//!
//! Every quantity is in SI units, so the values in the literature can be used
//! as they are printed.
//!
//! # The solve
//!
//! One step is implicit: the force at the end of the sample depends on the
//! compression at the end of the sample, which depends on the force. The
//! trapezoidal rule turns that into one scalar equation, `ξ + c·ξ^p = a`,
//! whose left side is convex and increasing. Newton's method started above
//! the root of such a function walks down to it without ever overshooting, so
//! a fixed, small number of steps is enough and there is no `while
//! !converged` anywhere — see `docs/REALTIME-AUDIO-RULES.md` and `PERF-007`.

use crate::math;

/// Softest and hardest felt nonlinearity [`HammerConfig::contact_exponent`]
/// may take. Below 1 the contact equation stops being convex and the solve's
/// guarantee goes with it; 8 is far beyond any felt and only bounds `powf`.
pub const MIN_CONTACT_EXPONENT: f32 = 1.0;
/// See [`MIN_CONTACT_EXPONENT`].
pub const MAX_CONTACT_EXPONENT: f32 = 8.0;

/// Range of [`HammerConfig::stiffness`], in N/m^p. Chaigne & Askenfelt's
/// hammers span `4e8` (C2) to `1e12` (C7); the range leaves two decades
/// either side.
pub const MIN_STIFFNESS: f32 = 1.0e6;
/// See [`MIN_STIFFNESS`].
pub const MAX_STIFFNESS: f32 = 1.0e14;

/// Range of [`HammerConfig::mass`], in kilograms: a tenth of a gram to a
/// hundred grams. A concert grand's hammers weigh about 4 to 11 g.
pub const MIN_MASS: f32 = 1.0e-4;
/// See [`MIN_MASS`].
pub const MAX_MASS: f32 = 0.1;

/// Range of [`HammerConfig::string_impedance`], in kg/s. Piano strings run
/// from about 1.5 (top treble) to 14 (lowest wound string).
pub const MIN_STRING_IMPEDANCE: f32 = 0.1;
/// See [`MIN_STRING_IMPEDANCE`].
pub const MAX_STRING_IMPEDANCE: f32 = 1.0e3;

/// Hammer speed, in m/s, of a note played at full velocity.
///
/// Calibrated together with [`STRIKE_CURVE_EXPONENT`]: with these two, the
/// model's change of spectrum between soft, medium and loud notes matches a
/// measured concert grand's (`docs/CALIBRATION.md`), and the speeds
/// themselves land where Askenfelt & Jansson measured real hammers —
/// roughly 1 m/s *piano*, 2 m/s *mezzo-forte*, 4–5 m/s *fortissimo*.
pub const MAX_STRIKE_MPS: f32 = 4.7;

/// How hammer speed grows with normalised key velocity. See
/// [`MAX_STRIKE_MPS`].
pub const STRIKE_CURVE_EXPONENT: f32 = 1.25;

/// The force at the bridge, in newtons, that a string reports as `1.0`.
///
/// A string's output is the force it exerts on the bridge — the quantity
/// that actually drives the soundboard — divided by this, so that a loud
/// note stays near full scale instead of at tens of newtons.
pub const FULL_SCALE_BRIDGE_NEWTONS: f32 = 50.0;

/// The felt may not be compressed further than this many metres. No hammer
/// comes near it; the bound only keeps `powf` finite for hostile input.
const MAX_COMPRESSION_METRES: f32 = 0.05;

/// Once the hammer has turned back and opened this much air between felt and
/// string, the strike is over.
const RELEASE_GAP_METRES: f32 = 1.0e-4;

/// A strike is abandoned after this long whatever the hammer is doing —
/// the bound that keeps a contact from outliving its note. Real contacts
/// last 0.5–5 ms.
const MAX_CONTACT_SECONDS: f32 = 0.03;

/// Newton steps per sample. Started within a factor of two above the root of
/// a convex function, four steps reach `f32` precision; see the module docs.
const NEWTON_STEPS: usize = 4;

const MIN_SAMPLE_RATE_HZ: f32 = 1_000.0;
const MAX_SAMPLE_RATE_HZ: f32 = 10_000_000.0;
const FALLBACK_SAMPLE_RATE_HZ: f32 = 48_000.0;

/// The part of a hammer that one string feels, and the string it strikes.
///
/// A hammer strikes every string of its unison at once. Identical strings
/// share it equally, so each behaves exactly as if struck alone by a hammer
/// of `1/n` the mass and `1/n` the felt stiffness — and that share is what
/// this describes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HammerConfig {
    /// The felt's nonlinearity `p` in `F = K·ξ^p`.
    pub contact_exponent: f32,
    /// The felt's stiffness `K`, in N/m^p.
    pub stiffness: f32,
    /// Mass, in kilograms.
    pub mass: f32,
    /// The struck string's characteristic impedance `Z = √(T·μ)`, in kg/s.
    pub string_impedance: f32,
}

/// One string's share of a middle-C hammer: Chaigne & Askenfelt's C4 values
/// (2.97 g, `4.5e9`, `p = 2.5`) split between three strings.
pub const DEFAULT_HAMMER: HammerConfig = HammerConfig {
    contact_exponent: 2.5,
    stiffness: 1.5e9,
    mass: 0.99e-3,
    string_impedance: 2.2,
};

fn sanitize_hammer(hammer: HammerConfig) -> HammerConfig {
    HammerConfig {
        contact_exponent: math::clamp_or_low(
            hammer.contact_exponent,
            MIN_CONTACT_EXPONENT,
            MAX_CONTACT_EXPONENT,
        ),
        stiffness: math::clamp_or_low(hammer.stiffness, MIN_STIFFNESS, MAX_STIFFNESS),
        mass: math::clamp_or_low(hammer.mass, MIN_MASS, MAX_MASS),
        string_impedance: math::clamp_or_low(
            hammer.string_impedance,
            MIN_STRING_IMPEDANCE,
            MAX_STRING_IMPEDANCE,
        ),
    }
}

fn usable_sample_rate(sample_rate_hz: f32) -> f32 {
    if sample_rate_hz.is_finite()
        && (MIN_SAMPLE_RATE_HZ..=MAX_SAMPLE_RATE_HZ).contains(&sample_rate_hz)
    {
        sample_rate_hz
    } else {
        FALLBACK_SAMPLE_RATE_HZ
    }
}

/// Hammer speed in m/s for a normalised key velocity in `0.0..=1.0`.
///
/// Zero velocity is zero speed: such a strike leaves the string as it was.
#[inline]
#[must_use]
pub fn strike_mps_for(velocity: f32) -> f32 {
    let velocity = math::clamp_or_low(velocity, 0.0, 1.0);
    MAX_STRIKE_MPS * math::powf(velocity, STRIKE_CURVE_EXPONENT)
}

/// A hammer and its string, reduced to the constants one contact step needs
/// at one sample rate.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ContactSolver {
    exponent: f32,
    /// `c` in `ξ + c·ξ^p = a`: stiffness times `compliance`.
    stiffness_gain: f32,
    /// Metres the contact point yields per newton over one trapezoidal step:
    /// `dt²/4m + dt/4Z`, the hammer's part and the string's.
    compliance: f32,
    half_step_seconds: f32,
    /// `dt / 2m`: hammer speed lost per newton per half step.
    speed_per_newton: f32,
    /// `1 / 2Z`: string velocity gained per newton.
    injection_per_newton: f32,
    bridge_gain: f32,
    max_steps: u32,
}

impl ContactSolver {
    /// Prepares `hammer` for `sample_rate_hz`. Out-of-range fields are
    /// clamped; an unusable sample rate falls back to 48 kHz.
    #[must_use]
    pub fn new(hammer: HammerConfig, sample_rate_hz: f32) -> Self {
        let hammer = sanitize_hammer(hammer);
        let step = 1.0 / usable_sample_rate(sample_rate_hz);
        let compliance = 0.25 * step * step / hammer.mass + 0.25 * step / hammer.string_impedance;
        Self {
            exponent: hammer.contact_exponent,
            stiffness_gain: hammer.stiffness * compliance,
            compliance,
            half_step_seconds: 0.5 * step,
            speed_per_newton: 0.5 * step / hammer.mass,
            injection_per_newton: 0.5 / hammer.string_impedance,
            bridge_gain: 2.0 * hammer.string_impedance / FULL_SCALE_BRIDGE_NEWTONS,
            max_steps: (MAX_CONTACT_SECONDS / step) as u32 + 1,
        }
    }

    /// What a velocity wave arriving at the bridge is worth as output: the
    /// force `2Z·v` it exerts there, over [`FULL_SCALE_BRIDGE_NEWTONS`].
    #[inline]
    #[must_use]
    pub fn bridge_gain(&self) -> f32 {
        self.bridge_gain
    }

    /// Solves `ξ + c·ξ^p = unloaded` and returns `(ξ, force)`.
    ///
    /// `unloaded` is where the felt would be were there no contact force
    /// this step; when it is not positive, felt and string do not touch.
    #[inline]
    fn resolve(&self, unloaded: f32) -> (f32, f32) {
        if unloaded.is_nan() || unloaded <= 0.0 {
            return (unloaded.max(-MAX_COMPRESSION_METRES), 0.0);
        }
        let unloaded = unloaded.min(MAX_COMPRESSION_METRES);
        let mut compression = self.starting_compression(unloaded);
        for _ in 0..NEWTON_STEPS {
            compression = self.newton_step(compression, unloaded);
        }
        (compression, (unloaded - compression) / self.compliance)
    }

    /// The smaller of the equation's two limiting solutions — all spring or
    /// all yield. Each lies above the root and one is within a factor of two.
    #[inline]
    fn starting_compression(&self, unloaded: f32) -> f32 {
        let all_spring = math::powf(unloaded / self.stiffness_gain, 1.0 / self.exponent);
        unloaded.min(all_spring)
    }

    #[inline]
    fn newton_step(&self, compression: f32, unloaded: f32) -> f32 {
        let slope_power = math::powf(compression, self.exponent - 1.0);
        let spring = self.stiffness_gain * slope_power;
        let residual = compression + spring * compression - unloaded;
        let slope = 1.0 + self.exponent * spring;
        math::clamp_or_low(compression - residual / slope, 0.0, unloaded)
    }
}

/// The hammer during one strike: where it is, how fast, how hard it pushes.
///
/// At rest it costs one branch per sample. [`Self::strike`] throws it at the
/// string; [`Self::step`] then advances hammer and string together until the
/// hammer has left, and returns it to rest.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HammerJunction {
    /// Felt compression in metres; negative is an air gap.
    compression: f32,
    /// Hammer velocity towards the string, in m/s.
    velocity: f32,
    force: f32,
    /// Rate of change of `compression` at the end of the previous step.
    closing_rate: f32,
    steps_left: u32,
}

impl Default for HammerJunction {
    fn default() -> Self {
        Self::resting()
    }
}

impl HammerJunction {
    /// A hammer away from the string.
    #[must_use]
    pub const fn resting() -> Self {
        Self {
            compression: 0.0,
            velocity: 0.0,
            force: 0.0,
            closing_rate: 0.0,
            steps_left: 0,
        }
    }

    /// Throws the hammer at the string at `strike_mps`, felt just touching.
    /// A speed that is not positive leaves the hammer at rest.
    pub fn strike(&mut self, strike_mps: f32, solver: &ContactSolver) {
        *self = Self::resting();
        if strike_mps.is_nan() || strike_mps <= 0.0 {
            return;
        }
        self.velocity = strike_mps.min(MAX_STRIKE_MPS);
        self.closing_rate = self.velocity;
        self.steps_left = solver.max_steps;
    }

    /// Whether a strike is in progress.
    #[inline]
    #[must_use]
    pub fn is_striking(&self) -> bool {
        self.steps_left > 0
    }

    /// The force the felt exerted on the string in the last step, in newtons.
    #[inline]
    #[must_use]
    pub fn force(&self) -> f32 {
        self.force
    }

    /// Advances the contact by one sample.
    ///
    /// `incoming_velocity` is the string's velocity at the strike point were
    /// the hammer absent: the sum of the two waves arriving there. Returns
    /// the velocity the hammer adds to each wave leaving the point, `F / 2Z`.
    /// Total: any input, `NaN` and `±∞` included, yields a finite result.
    #[inline]
    pub fn step(&mut self, solver: &ContactSolver, incoming_velocity: f32) -> f32 {
        if self.steps_left == 0 {
            return 0.0;
        }
        let incoming = if incoming_velocity.is_finite() {
            incoming_velocity
        } else {
            0.0
        };
        let coasting = self.velocity - solver.speed_per_newton * self.force;
        let unloaded =
            self.compression + solver.half_step_seconds * (self.closing_rate + coasting - incoming);
        let (compression, force) = solver.resolve(unloaded);
        self.advance(solver, coasting - incoming, compression, force)
    }

    #[inline]
    fn advance(
        &mut self,
        solver: &ContactSolver,
        coasting_rate: f32,
        compression: f32,
        force: f32,
    ) -> f32 {
        let injected = force * solver.injection_per_newton;
        self.velocity -= solver.speed_per_newton * (self.force + force);
        self.closing_rate = coasting_rate - solver.speed_per_newton * force - injected;
        self.compression = compression;
        self.force = force;
        self.steps_left -= 1;
        if self.has_left() || !self.is_finite() {
            *self = Self::resting();
        }
        if injected.is_finite() { injected } else { 0.0 }
    }

    #[inline]
    fn has_left(&self) -> bool {
        self.velocity < 0.0 && self.compression < -RELEASE_GAP_METRES
    }

    #[inline]
    fn is_finite(&self) -> bool {
        self.compression.is_finite() && self.velocity.is_finite() && self.closing_rate.is_finite()
    }
}

/// Semitones from A0 (27.5 Hz) to the top of the keyboard.
const KEYBOARD_SEMITONES: f32 = 87.0;
const A0_HZ: f32 = 27.5;
const SEMITONES_PER_OCTAVE: f32 = 12.0;

/// Semitones above A0 of F♯4, where the treble hammers start to depart
/// from the trend of the rest of the set. See [`MASS_KNOTS`].
const TREBLE_HAMMERS_FROM: f32 = 45.0;

/// Whole-hammer mass in kilograms against semitones above A0: 10.5 g in the
/// bass falling evenly to 7.2 g at F♯4, the usual trend of a concert grand's
/// hammer set, then faster to 1.9 g at C8.
///
/// The upper half is set by one requirement. A hammer that stays on its
/// string for about a period and a half delivers a force pulse with a
/// spectral null at the fundamental, and the note all but vanishes. With
/// the bass trend carried to the top, that null crossed the keyboard
/// between A6 and F♯7 as the touch went from soft to loud: 30 dB holes one
/// key wide, at a different key for each dynamic. A lighter hammer on
/// stiffer felt ([`LOG10_STIFFNESS_KNOTS`]) leaves sooner, and from a key
/// velocity of 0.25 up no key's fundamental drops out. Softer than that the
/// top two octaves still do: a power-law felt has no stiffness left at so
/// small a compression. Chaigne & Askenfelt's own treble hammer is as
/// light: 2.2 g at C7.
const MASS_KNOTS: [(f32, f32); 3] = [
    (0.0, 10.5e-3),
    (TREBLE_HAMMERS_FROM, 7.24e-3),
    (KEYBOARD_SEMITONES, 1.9e-3),
];

/// `log10` of whole-hammer felt stiffness against semitones above A0.
///
/// Fitted so that the model's spectrum changes between soft, medium and loud
/// notes the way a measured concert grand's does (`docs/CALIBRATION.md`); up
/// to F♯4 the fit lands within a factor of two of Chaigne & Askenfelt's
/// `4e8` (C2) and `4.5e9` (C4). Above it the felt stiffens faster, to
/// `7e12` at C7 against their `1e12`, for the reason given at
/// [`MASS_KNOTS`].
const LOG10_STIFFNESS_KNOTS: [(f32, f32); 5] = [
    (0.0, 8.2),
    (24.0, 9.1),
    (TREBLE_HAMMERS_FROM, 10.24),
    (72.0, 12.6),
    (KEYBOARD_SEMITONES, 13.9),
];

/// Felt exponent against semitones above A0, from the same fit as
/// [`LOG10_STIFFNESS_KNOTS`]. Chaigne & Askenfelt: 2.3 (C2), 2.5 (C4), 3.0
/// (C7).
const EXPONENT_KNOTS: [(f32, f32); 3] = [(0.0, 2.2), (43.0, 2.6), (KEYBOARD_SEMITONES, 3.2)];

/// `log10` of one string's impedance in kg/s against semitones above A0.
///
/// `Z = 2·L·f·μ` for a generic concert-grand scale: plain steel of 1.0–0.8 mm
/// from the middle up (about 2.2 falling to 1.6 kg/s), copper-wound strings
/// growing to 13.7 kg/s at A0. No particular instrument's stringing table.
const LOG10_IMPEDANCE_KNOTS: [(f32, f32); 9] = [
    (0.0, 1.137),
    (7.0, 0.978),
    (15.0, 0.690),
    (23.0, 0.462),
    (29.0, 0.367),
    (48.0, 0.348),
    (63.0, 0.267),
    (75.0, 0.230),
    (KEYBOARD_SEMITONES, 0.199),
];

/// One string's share of the hammer a concert grand would carry at
/// `fundamental_hz`, and that string's impedance.
///
/// The share assumes the usual stringing at that pitch —
/// [`crate::unison::unison_count_for_key_index`].
#[must_use]
pub fn hammer_for_frequency(fundamental_hz: f32) -> HammerConfig {
    let semitones = semitones_above_a0(fundamental_hz);
    let strings = crate::unison::unison_count_for_key_index(semitones as u8) as f32;
    let whole_stiffness = math::powf(10.0, piecewise(semitones, &LOG10_STIFFNESS_KNOTS));
    HammerConfig {
        contact_exponent: piecewise(semitones, &EXPONENT_KNOTS),
        stiffness: whole_stiffness / strings,
        mass: piecewise(semitones, &MASS_KNOTS) / strings,
        string_impedance: math::powf(10.0, piecewise(semitones, &LOG10_IMPEDANCE_KNOTS)),
    }
}

fn semitones_above_a0(fundamental_hz: f32) -> f32 {
    let ratio = math::clamp_or_low(fundamental_hz / A0_HZ, 1.0, f32::MAX);
    let semitones = SEMITONES_PER_OCTAVE * math::ln(ratio) / core::f32::consts::LN_2;
    math::round(math::clamp_or_low(semitones, 0.0, KEYBOARD_SEMITONES))
}

/// Linear interpolation through `knots`, held flat beyond either end.
fn piecewise(position: f32, knots: &[(f32, f32)]) -> f32 {
    let mut previous = knots.first().copied().unwrap_or((0.0, 0.0));
    if position.is_nan() || position <= previous.0 {
        return previous.1;
    }
    for &knot in knots.iter().skip(1) {
        if position <= knot.0 {
            let share = (position - previous.0) / (knot.0 - previous.0);
            return previous.1 + share * (knot.1 - previous.1);
        }
        previous = knot;
    }
    previous.1
}

#[cfg(test)]
#[path = "hammer_tests.rs"]
mod tests;
