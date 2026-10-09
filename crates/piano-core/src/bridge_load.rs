//! The soundboard as a load on the strings (issues #90, #91).
//!
//! [`crate::soundboard::Soundboard`] radiates: it colours what a listener
//! hears and never acts back on a string. A real bridge does both. The
//! velocity the board's modes impose on it is a boundary condition on every
//! string resting there, so a string loses energy fastest where the board
//! answers it most — G. Weinreich, "Coupled piano strings", JASA 62(6),
//! 1977. X. Boutillon & K. Ege, "Vibroacoustics of the piano soundboard",
//! arXiv:1305.3057, 2013, split the bridge mobility into a **modal** low
//! regime and a smooth, nearly flat high regime. This bank is the modal
//! part; [`crate::bridge::BridgeBus`] already plays the flat one.
//!
//! Each mode here is the same resonance as the radiating board's mode with
//! the same index, in its constant-peak bandpass form `(1 − z⁻²)·R(z)`,
//! weighted by [`SoundboardMode::bridge_coupling`]. Its real part is never
//! negative, so fed back negatively it only ever removes energy.
//!
//! # Why every key carries its own bank
//!
//! The engine renders one voice's whole block before the next, so a single
//! board shared by all keys could only answer a string a block late. That
//! was tried first: the board's free response was predicted over each block
//! and read by every voice, with only the block's newest forces arriving
//! late. It measured as **not passive**. Driven off resonance, a mode's free
//! continuation rings at the *mode's* frequency rather than the string's.
//! A single string's partials near a mode then decayed up to 1.5× *slower*
//! with the load on than off: energy injected, not removed. So each key
//! drives its own copy of the bank with its own strings, sample by sample,
//! with no latency. The cross-key half of the bridge stays on the flat,
//! block-latent [`crate::bridge::BridgeBus`], where latency only delays a
//! slowly building sympathetic resonance. See
//! `docs/superpowers/specs/2026-10-09-soundboard-bridge-load-design.md` and
//! `PERF-015`.
//!
//! # Which modes a key carries
//!
//! Only the ones that can take energy from it. Boutillon & Ege's modal
//! regime ends around a kilohertz ([`MODAL_CEILING_HZ`]); above it the flat
//! bus is the model. Below the key's fundamental there is no string motion
//! to load, and a mode with `Q ≥ 16` at [`LOWEST_LOADING_RATIO`] of it
//! already has under 2% in-phase response at the fundamental. A bass key
//! carries most of the bank, a key above a kilohertz carries none: at full
//! polyphony that is about a quarter of the cost of every key carrying all
//! [`MODE_COUNT`] modes (`PERF-015`).

use crate::soundboard::{DEFAULT_MODES, MODE_COUNT, Resonator, SoundboardMode};
use crate::{math, units::SampleRate};

/// How strongly the board loads a string by default. See the measurement
/// in `piano-audio/tests/board_load.rs`.
pub const DEFAULT_BOARD_LOAD_GAIN: f32 = 1.0;

/// Largest load gain accepted: a deliberately generous ceiling, so the
/// stability tests can exercise it.
pub const MAX_BOARD_LOAD_GAIN: f32 = 4.0;

/// Upper edge of the bridge mobility's modal regime (Boutillon & Ege 2013:
/// "a few hundred hertz to about a kilohertz").
pub const MODAL_CEILING_HZ: f32 = 1_000.0;

/// Lowest mode frequency, as a fraction of the key's fundamental, a key's
/// bank carries. See the module docs.
pub const LOWEST_LOADING_RATIO: f32 = 0.8;

/// Largest per-mode bridge coupling accepted from a live or file-supplied
/// mode; the default table tops out at `0.95`.
const MAX_BRIDGE_COUPLING: f32 = 2.0;

/// Bound on the drive, as in `Soundboard::process`: any finite string sum
/// passes, `NaN` and infinities cannot reach a recursion.
const MAX_DRIVE: f32 = 1.0e6;

/// One mode of the load: a unit-peak resonator and the weight that turns
/// its free bandpass response into bridge velocity.
#[derive(Debug, Clone, Copy, Default)]
struct LoadMode {
    resonator: Resonator,
    weight: f32,
    /// Which [`DEFAULT_MODES`] slot this is, so a live retune finds it.
    board_index: usize,
}

impl LoadMode {
    fn new(mode: SoundboardMode, sample_rate: f32, board_index: usize) -> Self {
        let resonator = Resonator::new(SoundboardMode { gain: 1.0, ..mode }, sample_rate);
        let coupling = math::clamp_or_low(mode.bridge_coupling, 0.0, MAX_BRIDGE_COUPLING);
        Self {
            resonator,
            weight: coupling * resonator.bandpass_normaliser(),
            board_index,
        }
    }
}

/// The modal half of the bridge's mobility, as one key's strings see it —
/// see the module docs.
///
/// Holds no heap memory. [`BridgeLoad::process`] is allocation-free,
/// lock-free and panic-free.
#[derive(Debug, Clone)]
pub struct BridgeLoad {
    modes: [LoadMode; MODE_COUNT],
    sample_rate: f32,
    fundamental_hz: f32,
    /// How many leading entries of `modes` this key carries; the rest are
    /// never processed. Kept compact so the per-sample loop has no branch.
    active_count: usize,
}

impl BridgeLoad {
    /// The load a key sounding at `fundamental_hz` feels, built from
    /// [`DEFAULT_MODES`] for a stream at `sample_rate`, at rest.
    #[must_use]
    pub fn new(sample_rate: SampleRate, fundamental_hz: f32) -> Self {
        let mut load = Self {
            modes: [LoadMode::default(); MODE_COUNT],
            sample_rate: sample_rate.hertz(),
            fundamental_hz,
            active_count: 0,
        };
        for (index, mode) in DEFAULT_MODES.into_iter().enumerate() {
            load.set_mode(index, mode);
        }
        load
    }

    /// Rebuilds board mode `index` from `mode`, as
    /// [`crate::soundboard::Soundboard::set_mode`] does for the radiating
    /// board, so the two stay the same resonances. A mode outside this
    /// key's loading band is dropped. Out of range is a no-op.
    pub fn set_mode(&mut self, index: usize, mode: SoundboardMode) {
        if index >= MODE_COUNT {
            return;
        }
        self.remove_board_mode(index);
        if !self.loads(mode.frequency_hz) {
            return;
        }
        if let Some(slot) = self.modes.get_mut(self.active_count) {
            *slot = LoadMode::new(mode, self.sample_rate, index);
            self.active_count += 1;
        }
    }

    /// How many board modes this key carries.
    #[must_use]
    pub fn active_mode_count(&self) -> usize {
        self.active_count
    }

    fn loads(&self, frequency_hz: f32) -> bool {
        let lowest = LOWEST_LOADING_RATIO * self.fundamental_hz;
        (lowest..=MODAL_CEILING_HZ).contains(&frequency_hz)
    }

    /// Drops board mode `index` from the active set, keeping it compact.
    fn remove_board_mode(&mut self, index: usize) {
        let active = self.modes.get_mut(..self.active_count).unwrap_or_default();
        let Some(position) = active.iter().position(|mode| mode.board_index == index) else {
            return;
        };
        active.swap(position, active.len() - 1);
        self.active_count -= 1;
    }

    /// The bridge velocity this sample, given `force`, the key's strings'
    /// summed signal at the bridge this same sample. Unit gain and zero
    /// phase at each mode's resonance, scaled by its coupling.
    #[inline]
    pub fn process(&mut self, force: f32) -> f32 {
        let force = math::clamp_or_low(force, -MAX_DRIVE, MAX_DRIVE);
        let mut velocity = 0.0f32;
        for mode in self.modes.iter_mut().take(self.active_count) {
            velocity += mode.weight * mode.resonator.process_bandpass(force);
        }
        velocity
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::float_cmp, clippy::unwrap_used, clippy::expect_used)]

    use alloc::vec::Vec;
    use proptest::prelude::*;

    use super::*;

    const RATE: f32 = 48_000.0;

    fn load() -> BridgeLoad {
        BridgeLoad::new(SampleRate::new(RATE).expect("48 kHz is valid"), 27.5)
    }

    /// A load whose only coupled mode is one at `frequency_hz`.
    fn one_mode_load(frequency_hz: f32) -> BridgeLoad {
        let mut load = load();
        for (index, mode) in DEFAULT_MODES.into_iter().enumerate() {
            load.set_mode(
                index,
                SoundboardMode {
                    bridge_coupling: 0.0,
                    ..mode
                },
            );
        }
        load.set_mode(
            0,
            SoundboardMode {
                frequency_hz,
                decay_seconds: 0.05,
                gain: 0.3,
                bridge_coupling: 1.0,
            },
        );
        load
    }

    fn sine(frequency_hz: f32, samples: usize) -> Vec<f32> {
        let omega = core::f32::consts::TAU * frequency_hz / RATE;
        (0..samples).map(|n| libm::sinf(omega * n as f32)).collect()
    }

    /// Correlation of the velocity with the force producing it over the
    /// second half of a steady sine, normalised by the force's power: the
    /// real part of the load's admittance at that frequency.
    fn in_phase_gain(load: &mut BridgeLoad, frequency_hz: f32) -> f32 {
        let force = sine(frequency_hz, 24_000);
        let (mut cross, mut power) = (0.0f32, 0.0f32);
        for (n, &f) in force.iter().enumerate() {
            let velocity = load.process(f);
            if n >= 12_000 {
                cross += velocity * f;
                power += f * f;
            }
        }
        cross / power
    }

    #[test]
    fn a_key_carries_only_the_modes_between_its_fundamental_and_the_modal_ceiling() {
        let rate = SampleRate::new(RATE).expect("48 kHz is valid");
        let carried = |fundamental| BridgeLoad::new(rate, fundamental).active_mode_count();
        let below_ceiling = DEFAULT_MODES
            .iter()
            .filter(|mode| mode.frequency_hz <= MODAL_CEILING_HZ)
            .count();
        assert_eq!(carried(27.5), below_ceiling);
        assert!(carried(440.0) < below_ceiling && carried(440.0) > 0);
        assert_eq!(carried(2_000.0), 0);
    }

    #[test]
    fn retuning_a_mode_out_of_the_band_and_back_keeps_the_count_honest() {
        let mut load = load();
        let before = load.active_mode_count();
        let out = SoundboardMode {
            frequency_hz: 5_000.0,
            ..DEFAULT_MODES[3]
        };
        load.set_mode(3, out);
        assert_eq!(load.active_mode_count(), before - 1);
        load.set_mode(3, DEFAULT_MODES[3]);
        load.set_mode(3, DEFAULT_MODES[3]);
        assert_eq!(load.active_mode_count(), before);
    }

    #[test]
    fn at_rest_with_no_force_there_is_no_motion() {
        let mut load = load();
        assert!((0..256).all(|_| load.process(0.0) == 0.0));
    }

    #[test]
    fn a_steady_drive_at_resonance_yields_an_in_phase_unit_velocity() {
        let gain = in_phase_gain(&mut one_mode_load(200.0), 200.0);
        assert!((0.9..1.1).contains(&gain), "in-phase gain {gain}");
    }

    #[test]
    fn the_load_never_returns_energy_at_any_frequency() {
        for frequency in [
            20.0f32, 50.0, 150.0, 199.0, 200.0, 201.0, 600.0, 3_000.0, 20_000.0,
        ] {
            let gain = in_phase_gain(&mut one_mode_load(200.0), frequency);
            assert!(gain >= -1e-4, "{frequency} Hz: in-phase gain {gain}");
        }
    }

    #[test]
    fn the_full_default_bank_never_returns_energy_either() {
        for frequency in [27.5f32, 55.0, 110.0, 220.0, 440.0, 880.0, 1_760.0, 4_186.0] {
            let gain = in_phase_gain(&mut load(), frequency);
            assert!(gain >= -1e-4, "{frequency} Hz: in-phase gain {gain}");
        }
    }

    proptest! {
        #[test]
        fn any_drive_and_any_mode_stay_finite(
            forces in proptest::collection::vec(proptest::num::f32::ANY, 0..256),
            frequency_hz in proptest::num::f32::ANY,
            decay_seconds in proptest::num::f32::ANY,
            bridge_coupling in proptest::num::f32::ANY,
            index in proptest::num::usize::ANY,
        ) {
            let mut load = load();
            let mode = SoundboardMode { frequency_hz, decay_seconds, gain: 1.0, bridge_coupling };
            load.set_mode(index, mode);
            load.set_mode(0, mode);
            for force in forces {
                prop_assert!(load.process(force).is_finite());
            }
        }
    }
}
