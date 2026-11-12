//! A note's own unison strings, coupled through their shared bridge contact.
//!
//! Most piano notes are struck by more than one physical string tuned very
//! slightly apart. This is what produces the beating and the two-stage decay
//! real piano notes have that a single Karplus-Strong-style string never
//! can: G. Weinreich, "Coupled Piano Strings" (JASA 62(6), 1974-1484, 1977)
//! is the seminal measurement and model of exactly this — near-unison
//! strings coupled through the bridge's shared, finite admittance produce a
//! fast initial "pre-decay" (the strings beating against and dephasing from
//! each other) followed by a slower "aftersound" (once they settle into a
//! shared decaying mode). This module reuses [`PluckedString`] rather than
//! forking it — a unison group is a small number of independent strings,
//! not a new DSP primitive.
//!
//! # Two couplings, two honesty notes
//!
//! Weinreich's own model treats every string, whether same-note unison or a
//! different key entirely, as coupled through one shared bridge admittance.
//! This project splits that into two engineering tiers for feasibility:
//!
//! - **Local coupling** (this module): a note's own 1-3 strings couple
//!   sample-accurately, every sample, with no extra latency — cheap because
//!   a unison group already processes all its strings together in one call.
//! - **Global coupling** ([`crate::bridge::BridgeBus`]): cross-*key*
//!   sympathetic resonance, e.g. via the sustain pedal, is coupled with one
//!   block of latency (`PERF-008` in `docs/PERFORMANCE.md`) because summing
//!   all 88 keys' contributions every sample would need a full O(N) pass
//!   *before* any string could read back this sample's total, which is not
//!   representable in a per-voice, per-sample call. A few milliseconds of
//!   latency in a slowly-building resonance effect is inaudible; see the
//!   module docs on [`crate::bridge::BridgeBus`] for the full reasoning.
//!
//! # Register boundaries and unison counts
//!
//! A standard modern piano is single-strung (monochord) in the bass,
//! double-strung (bichord) through the tenor, and triple-strung (trichord)
//! for the rest of the treble — A. Reblitz, *Piano Servicing, Tuning, and
//! Rebuilding* (1993), describing the layout piano technicians work to;
//! consistent with the qualitative bass/tenor/treble description in
//! Fletcher & Rossing, *The Physics of Musical Instruments* (already cited
//! in `crate::dispersion` for this project's inharmonicity figures). Exact
//! break points vary by piano model and scale design, so the specific key
//! boundaries `piano_audio::voicing` chooses are this project's own
//! representative choice within that convention (12 single-strung keys, 18
//! double-strung, 58 triple-strung — chosen to land on the full 88-key
//! total), not a measurement from one specific instrument.

use crate::{
    bridge::BridgeBus,
    bridge_load::{BridgeLoad, DEFAULT_BOARD_LOAD_GAIN, MAX_BOARD_LOAD_GAIN},
    error::ParamError,
    hammer, math,
    string::{PluckedString, StringConfig},
    units::{Hz, SampleRate},
};

/// The most strings a real piano note is ever struck by.
pub const MAX_UNISON_STRINGS: usize = 3;

/// How much of a string's round-trip loss its note's slow modes are spared.
/// See [`spare_slow_modes`].
///
/// G. Weinreich, "Coupled piano strings", JASA 62(6), 1977: the motion in
/// which a note's strings push the bridge together drains into the
/// soundboard fast — the loud, quickly-decaying *prompt sound*. Every other
/// motion of the group barely moves the bridge and rings on far longer — the
/// quiet *aftersound* that makes a piano note sing. At `0.87` those slow
/// modes lose about an eighth of what the prompt mode does: resolving a
/// grand's partials into their decaying modes gives fundamentals falling
/// 17-18 dB/s with a companion mode near 2-6 dB/s through the middle of the
/// keyboard, and that ratio holds roughly from the tenor to the top octave
/// (`docs/CALIBRATION.md`).
pub const DEFAULT_LOCAL_COUPLING_GAIN: f32 = 0.87;

/// Highest share accepted. At `1.0` a slow mode would lose nothing at DC
/// (the loss filter passes DC unchanged), so a struck group could ring
/// forever and never free its voice.
const MAX_LOCAL_COUPLING_GAIN: f32 = 0.95;

/// Total mistuning, in cents, between the flattest and the sharpest string
/// of a group nobody has tuned yet. `piano_audio::voicing` replaces it per
/// key ([`UnisonGroup::set_unison_spread`]); this is the middle of what it
/// chooses, for a group built on its own.
pub const DEFAULT_UNISON_SPREAD_CENTS: f32 = 0.5;

/// Largest spread [`UnisonGroup::set_unison_spread`] accepts: far past any
/// tuned unison, well inside [`crate::string::MAX_LIVE_DETUNE_CENTS`].
pub const MAX_UNISON_SPREAD_CENTS: f32 = 50.0;

/// How a bichord's spread is divided between its two strings.
const MISTUNING_SHARE_BICHORD: [f32; 2] = [-0.5, 0.5];

/// How a trichord's spread is divided among its three strings: one flat,
/// two sharp by different amounts. Three strings at `-φ, 0, +φ` sum to
/// `1 + 2·cos φ`, which is exactly zero once `φ` reaches `2π/3`, so a
/// symmetric trichord beats to a periodic dead null no unison tuned by ear
/// ever lands on (issue #96).
const MISTUNING_SHARE_TRICHORD: [f32; 3] = [-0.6, 0.25, 0.4];

/// How unequally a bichord's two strings load the bridge. See
/// [`BRIDGE_LOAD_TRICHORD`].
const BRIDGE_LOAD_BICHORD: [f32; 2] = [1.2, 0.8];

/// How unequally a trichord's three strings load the bridge.
///
/// With equal loads the prompt mode is the plain average of the strings, a
/// hammer that strikes them alike excites nothing else, and the only way
/// into the aftersound is mistuning — which reaches it through a phase
/// reversal, a dead notch between the two stages, and only for partials
/// whose mistuning is small against their decay rate. A real unison's
/// aftersound is there in every partial at 10-30 dB under the prompt sound,
/// with well under a cent between its strings: its strings also move along
/// the bridge, where the board barely answers, and the bridge rocks. This
/// model has one polarisation, so that second, weakly coupled family of
/// motions is stood in for by letting the strings load the bridge
/// unequally: the prompt mode is then a weighted average, and an even
/// strike leaves the fraction `var(load)/mean(load)²` of its amplitude in
/// the slow modes — about 24 dB down here, 28 dB for a bichord. A reasoned
/// stand-in calibrated to that measurement, not a measured admittance.
const BRIDGE_LOAD_TRICHORD: [f32; 3] = [1.35, 0.9, 0.75];

/// A group of 1-3 [`PluckedString`]s struck together by one hammer,
/// coupled to each other through a local bridge contact.
///
/// Allocates once, in [`UnisonGroup::new`] (each constituent
/// [`PluckedString`] allocates its own delay line there). Every other
/// method is allocation-free, lock-free and panic-free.
#[derive(Debug, Clone)]
pub struct UnisonGroup {
    strings: [Option<PluckedString>; MAX_UNISON_STRINGS],
    count: usize,
    /// See [`UnisonGroup::set_local_coupling_gain`].
    local_coupling_gain: f32,
    /// See [`UnisonGroup::set_global_coupling_gain`].
    global_coupling_gain: f32,
    /// `base_config.frequency` as given to [`UnisonGroup::new`], before any
    /// per-string detuning — the reference [`UnisonGroup::set_string_detune`]
    /// computes an absolute retuned frequency from.
    base_frequency: Hz,
    /// The soundboard's modes as this key's strings feel them (#90, #91).
    board: BridgeLoad,
    /// See [`UnisonGroup::set_board_load_gain`].
    board_load_gain: f32,
    /// How hard each string loads the bridge ([`BRIDGE_LOAD_TRICHORD`]).
    bridge_loads: [f32; MAX_UNISON_STRINGS],
}

impl UnisonGroup {
    /// Builds a group of `unison_count` strings (clamped into
    /// `[1, MAX_UNISON_STRINGS]`) around `base_config`'s frequency, spread
    /// [`DEFAULT_UNISON_SPREAD_CENTS`] apart ([`detune_cents`]; a single
    /// string gets no mistuning — a monochord has nothing to beat against).
    ///
    /// # Errors
    ///
    /// Returns [`ParamError::FrequencyOutOfRange`] if the detuned frequency
    /// cannot be represented at `sample_rate` — same condition
    /// [`PluckedString::new`] rejects, propagated from whichever
    /// constituent string hits it first.
    pub fn new(
        base_config: StringConfig,
        unison_count: usize,
        sample_rate: SampleRate,
    ) -> Result<Self, ParamError> {
        let count = unison_count.clamp(1, MAX_UNISON_STRINGS);
        let mut strings: [Option<PluckedString>; MAX_UNISON_STRINGS] = [None, None, None];
        let mut bridge_loads = [1.0; MAX_UNISON_STRINGS];
        let slots = strings.iter_mut().zip(bridge_loads.iter_mut());
        for (index, (slot, load)) in slots.take(count).enumerate() {
            let cents = detune_cents(count, index, DEFAULT_UNISON_SPREAD_CENTS);
            let mut config = base_config;
            config.frequency = detune(base_config.frequency, cents);
            *slot = Some(PluckedString::new(config, sample_rate)?);
            *load = bridge_load_for(count, index);
        }
        Ok(Self {
            strings,
            count,
            local_coupling_gain: DEFAULT_LOCAL_COUPLING_GAIN,
            global_coupling_gain: DEFAULT_GLOBAL_COUPLING_GAIN,
            base_frequency: base_config.frequency,
            board: BridgeLoad::new(sample_rate, base_config.frequency.hertz()),
            board_load_gain: DEFAULT_BOARD_LOAD_GAIN,
            bridge_loads,
        })
    }

    /// Retunes the whole group so its flattest and sharpest strings sit
    /// `spread_cents` apart, divided as [`detune_cents`] divides it. Clamped
    /// into `[0, MAX_UNISON_SPREAD_CENTS]`; a single string is unaffected.
    pub fn set_unison_spread(&mut self, spread_cents: f32) {
        for index in 0..self.count {
            self.set_string_detune(index, detune_cents(self.count, index, spread_cents));
        }
    }

    /// Strikes every string in the group at the same `velocity` — a real
    /// hammer strikes all of one note's unison strings simultaneously.
    pub fn pluck(&mut self, velocity: f32) {
        for string in self.strings.iter_mut().flatten() {
            string.pluck(velocity);
        }
    }

    /// Stops every string dead ([`PluckedString::silence`]).
    pub fn silence(&mut self) {
        for string in self.strings.iter_mut().flatten() {
            string.silence();
        }
    }

    /// The una corda strike (issue #59): the soft pedal shifts the action
    /// sideways so the hammer misses one string of every bichord and
    /// trichord. The missed string is not struck but its damper is lifted
    /// with the rest, so it picks up the struck strings' motion through the
    /// coupling and rings out of phase with them — the veiled, aftersound-
    /// heavy colour the pedal is played for, not merely a quieter note
    /// (G. Weinreich, "Coupled piano strings", JASA 62(6), 1977, §V). A
    /// single-string key has nothing to miss and is struck as usual.
    ///
    /// The string missed is the group's first, the one that loads the
    /// bridge hardest ([`BRIDGE_LOAD_TRICHORD`]). What the struck strings
    /// then leave in the slow modes reaches the listener in phase with the
    /// prompt sound and 11 dB under it, against 24 dB for a full strike.
    /// Missing the lightest string instead leaves it in antiphase: less
    /// aftersound than a full strike, behind a notch.
    pub fn pluck_una_corda(&mut self, velocity: f32) {
        let missed = usize::from(self.count > 1);
        for (index, string) in self.strings.iter_mut().flatten().enumerate() {
            if index < missed {
                string.lift_damper();
            } else {
                string.pluck(velocity);
            }
        }
    }

    /// Rests every string's damper with `pressure`. See
    /// [`PluckedString::set_damper_pressure`].
    pub fn set_damper_pressure(&mut self, pressure: f32) {
        for string in self.strings.iter_mut().flatten() {
            string.set_damper_pressure(pressure);
        }
    }

    /// Sets every string's damper strength. See
    /// [`PluckedString::set_damper_strength`].
    pub fn set_damper_strength(&mut self, strength: f32) {
        for string in self.strings.iter_mut().flatten() {
            string.set_damper_strength(strength);
        }
    }

    /// Releases every string in the group. See [`PluckedString::release`].
    pub fn release(&mut self) {
        for string in self.strings.iter_mut().flatten() {
            string.release();
        }
    }

    /// Lifts every string's damper without a fresh strike. See
    /// [`PluckedString::lift_damper`].
    pub fn lift_damper(&mut self) {
        for string in self.strings.iter_mut().flatten() {
            string.lift_damper();
        }
    }

    /// Applies a new damping to every string in the group. See
    /// [`PluckedString::set_damping`].
    pub fn set_damping(&mut self, damping: f32) {
        for string in self.strings.iter_mut().flatten() {
            string.set_damping(damping);
        }
    }

    /// Applies a new sustain to every string in the group. See
    /// [`PluckedString::set_sustain`].
    pub fn set_sustain(&mut self, sustain: f32) {
        for string in self.strings.iter_mut().flatten() {
            string.set_sustain(sustain);
        }
    }

    /// Adjusts how much of the round-trip loss this group's differential
    /// modes are spared, live — see [`DEFAULT_LOCAL_COUPLING_GAIN`] for the
    /// physical meaning. `0` makes the strings independent. Clamped into
    /// `[0, MAX_LOCAL_COUPLING_GAIN]`.
    pub fn set_local_coupling_gain(&mut self, gain: f32) {
        self.local_coupling_gain = math::clamp_or_low(gain, 0.0, MAX_LOCAL_COUPLING_GAIN);
    }

    /// Adjusts how strongly this group blends with the shared, cross-key
    /// bridge bus's readback, live — see [`DEFAULT_GLOBAL_COUPLING_GAIN`]
    /// for the physical meaning. Clamped the same way as
    /// [`UnisonGroup::set_local_coupling_gain`].
    pub fn set_global_coupling_gain(&mut self, gain: f32) {
        self.global_coupling_gain = math::clamp_or_low(gain, 0.0, 1.0);
    }

    /// How strongly the soundboard's modes load this group's strings, live.
    /// `0` leaves only the flat bridge bus. Clamped into
    /// `[0, MAX_BOARD_LOAD_GAIN]`. See [`crate::bridge_load`].
    pub fn set_board_load_gain(&mut self, gain: f32) {
        self.board_load_gain = math::clamp_or_low(gain, 0.0, MAX_BOARD_LOAD_GAIN);
    }

    /// Retunes the board mode `index` this group's strings feel, so it
    /// tracks the radiating board's. See [`BridgeLoad::set_mode`].
    pub fn set_board_mode(&mut self, index: usize, mode: crate::soundboard::SoundboardMode) {
        self.board.set_mode(index, mode);
    }

    /// Adjusts one string's damping, live, leaving its unison siblings
    /// untouched. `string_index` outside the group's active string count
    /// is silently ignored — the same "no error channel on the audio
    /// thread" contract every other live setter in this crate uses.
    pub fn set_string_damping(&mut self, string_index: usize, damping: f32) {
        let Some(string) = self.strings.get_mut(string_index).and_then(Option::as_mut) else {
            return;
        };
        string.set_damping(damping);
    }

    /// Adjusts one string's sustain, live. See
    /// [`UnisonGroup::set_string_damping`] for the out-of-range contract.
    pub fn set_string_sustain(&mut self, string_index: usize, sustain: f32) {
        let Some(string) = self.strings.get_mut(string_index).and_then(Option::as_mut) else {
            return;
        };
        string.set_sustain(sustain);
    }

    /// Adjusts one string's inharmonicity coefficient, live. See
    /// [`UnisonGroup::set_string_damping`] for the out-of-range contract.
    pub fn set_string_inharmonicity(&mut self, string_index: usize, inharmonicity: f32) {
        let Some(string) = self.strings.get_mut(string_index).and_then(Option::as_mut) else {
            return;
        };
        string.set_inharmonicity(inharmonicity);
    }

    /// Adjusts one string's loss-filter zero, live. See
    /// [`PluckedString::set_loop_zero_mix`] and
    /// [`UnisonGroup::set_string_damping`] for the out-of-range contract.
    pub fn set_string_loop_zero_mix(&mut self, string_index: usize, zero_mix: f32) {
        let Some(string) = self.strings.get_mut(string_index).and_then(Option::as_mut) else {
            return;
        };
        string.set_loop_zero_mix(zero_mix);
    }

    /// Moves where the hammer strikes one string, for its *next* strike.
    /// See [`PluckedString::set_strike_position`] and
    /// [`UnisonGroup::set_string_damping`] for the out-of-range contract.
    pub fn set_string_strike_position(&mut self, string_index: usize, position: f32) {
        let Some(string) = self.strings.get_mut(string_index).and_then(Option::as_mut) else {
            return;
        };
        string.set_strike_position(position);
    }

    /// Changes one string's felt-contact physics, for its *next* strike.
    /// See [`UnisonGroup::set_string_damping`] for the out-of-range
    /// contract.
    pub fn set_string_hammer(&mut self, string_index: usize, hammer: hammer::HammerConfig) {
        let Some(string) = self.strings.get_mut(string_index).and_then(Option::as_mut) else {
            return;
        };
        string.set_hammer(hammer);
    }

    /// Retunes one string to `cents` away from the group's own
    /// [`UnisonGroup::base_frequency`] — an absolute retune from the
    /// group's reference pitch, not additive to whatever detune the
    /// string already carries from [`UnisonGroup::new`]. See
    /// [`UnisonGroup::set_string_damping`] for the out-of-range contract.
    pub fn set_string_detune(&mut self, string_index: usize, cents: f32) {
        let Some(string) = self.strings.get_mut(string_index).and_then(Option::as_mut) else {
            return;
        };
        string.set_frequency(detune(self.base_frequency, cents));
    }

    /// Whether every string in the group has decayed below audibility.
    #[must_use]
    pub fn is_silent(&self) -> bool {
        self.strings.iter().flatten().all(PluckedString::is_silent)
    }

    /// Whether at least one string's damper is lifted — see
    /// [`PluckedString::damper_engaged`] for why the engine needs this
    /// distinct from [`UnisonGroup::is_silent`].
    #[must_use]
    pub fn is_receptive(&self) -> bool {
        self.strings
            .iter()
            .flatten()
            .any(|string| !string.damper_engaged())
    }

    /// The loudest constituent string's envelope, for voice reclaiming.
    #[must_use]
    pub fn envelope(&self) -> f32 {
        self.strings
            .iter()
            .flatten()
            .fold(0.0f32, |loudest, string| loudest.max(string.envelope()))
    }

    /// Produces one output sample with only local (this group's own
    /// strings) coupling — no shared, cross-key bridge. Used where there is
    /// nothing else to couple to, e.g. rendering a single note offline.
    #[inline]
    pub fn process(&mut self) -> f32 {
        self.process_impl(None)
    }

    /// Produces one output sample coupled through both this group's own
    /// strings (local) and the shared, cross-key bridge bus at
    /// `bridge_index` within the block `bridge` is currently accumulating
    /// (global, `PERF-008`). See the module docs for why these are two
    /// separate couplings rather than one.
    #[inline]
    pub fn process_with_bridge(&mut self, bridge: &mut BridgeBus, bridge_index: usize) -> f32 {
        self.process_impl(Some((bridge, bridge_index)))
    }

    /// The shared implementation behind [`UnisonGroup::process`] and
    /// [`UnisonGroup::process_with_bridge`].
    ///
    /// 1. [`UnisonGroup::read_strings`] reads every string's bridge tap and
    ///    adds the receptive ones up at the bridge.
    /// 2. The group's mean drives the board's modes and is exchanged with
    ///    the cross-key bus, if there is one.
    /// 3. [`UnisonGroup::write_strings`] spares each receptive string's slow
    ///    modes, adds what the bridge drives back, and writes it.
    ///
    /// An absent bus drives nothing, which is the same arithmetic as a bus
    /// nobody has contributed to: the coupling is additive
    /// ([`PluckedString::write_mixed_feedback`]), so adding `0.0` is a true
    /// no-op and a missing bus cannot bend the decay.
    fn process_impl(&mut self, bridge: Option<(&mut BridgeBus, usize)>) -> f32 {
        let (samples, at_bridge) = self.read_strings();
        let group_mean = at_bridge.mean();
        // Gain zero skips the bank entirely, so turning the load off also
        // gives back its cost (`PERF-015`).
        let board_velocity = if self.board_load_gain > 0.0 {
            self.board.process(group_mean)
        } else {
            0.0
        };
        let bus_drive = bridge.map_or(0.0, |(bus, index)| {
            let (total, contributors) = bus.add_and_read_total(index, group_mean);
            bridge_drive_for(total, contributors, self.global_coupling_gain)
        });
        let bridge_drive = bus_drive - self.board_load_gain * board_velocity;
        self.write_strings(&samples, at_bridge.prompt_motion(), bridge_drive)
    }

    /// Reads every string's bridge tap and its dispersed signal
    /// ([`PluckedString::disperse`]), and sums the receptive ones — only a
    /// string whose damper is up both drives the bridge and hears it, per
    /// [`PluckedString::write_mixed_feedback`]'s own damper invariant.
    fn read_strings(&mut self) -> ([StringSample; MAX_UNISON_STRINGS], BridgeSum) {
        let mut samples = [StringSample::default(); MAX_UNISON_STRINGS];
        let mut at_bridge = BridgeSum::default();
        let strings = self.strings.iter_mut().zip(self.bridge_loads.iter());
        for ((string, &load), sample) in strings.zip(samples.iter_mut()).take(self.count) {
            let Some(string) = string.as_mut() else {
                continue;
            };
            let tap = string.read_bridge_tap();
            let dispersed = string.disperse(tap);
            let receptive = !string.damper_engaged();
            *sample = StringSample {
                tap,
                dispersed,
                receptive,
                load,
            };
            if receptive {
                at_bridge.add(dispersed, load);
            }
        }
        (samples, at_bridge)
    }

    /// Writes every string's next sample back and returns their summed
    /// output. A damped string neither drives the bridge nor takes anything
    /// back from it, so it writes only its own signal.
    fn write_strings(
        &mut self,
        samples: &[StringSample; MAX_UNISON_STRINGS],
        prompt_motion: f32,
        bridge_drive: f32,
    ) -> f32 {
        let share = self.local_coupling_gain;
        let mut output = 0.0f32;
        for (string, sample) in self.strings.iter_mut().zip(samples.iter()).take(self.count) {
            let Some(string) = string.as_mut() else {
                continue;
            };
            let (mixed, coupling) = if sample.receptive {
                let prompt_part = sample.load * prompt_motion;
                let spared =
                    spare_slow_modes(sample.dispersed, prompt_part, string.sustain(), share);
                (spared, bridge_drive)
            } else {
                (sample.dispersed, 0.0)
            };
            output += string.write_mixed_feedback(sample.tap, mixed, coupling);
        }
        output
    }
}

/// One string's per-sample scratch, filled by [`UnisonGroup::read_strings`]
/// and consumed by [`UnisonGroup::write_strings`] — bundled into one struct instead
/// of three parallel arrays so every access goes through
/// [`core::iter::Iterator::zip`] rather than an indexing expression
/// clippy's `indexing_slicing` lint (denied workspace-wide, per
/// `docs/REALTIME-AUDIO-RULES.md`) would otherwise flag as unproven-safe.
#[derive(Debug, Clone, Copy, Default)]
struct StringSample {
    tap: f32,
    dispersed: f32,
    receptive: bool,
    load: f32,
}

/// What a group's receptive strings add up to at the bridge in one sample.
#[derive(Debug, Clone, Copy, Default)]
struct BridgeSum {
    plain: f32,
    loaded: f32,
    load_energy: f32,
    receptive_count: usize,
}

impl BridgeSum {
    #[inline]
    fn add(&mut self, dispersed: f32, load: f32) {
        self.plain += dispersed;
        self.loaded += load * dispersed;
        self.load_energy += load * load;
        self.receptive_count += 1;
    }

    /// The receptive strings' mean signal — `0.0` when none are receptive,
    /// rather than a division by zero.
    #[inline]
    fn mean(self) -> f32 {
        if self.receptive_count == 0 {
            0.0
        } else {
            self.plain / self.receptive_count as f32
        }
    }

    /// The prompt mode's amplitude: the strings projected onto their own
    /// bridge loads, so string `i`'s part in it is `loadᵢ` times this.
    #[inline]
    fn prompt_motion(self) -> f32 {
        if self.load_energy > 0.0 {
            self.loaded / self.load_energy
        } else {
            0.0
        }
    }
}

/// How hard the shared, cross-key bridge bus's previous-block readback
/// *drives* a receptive string, as an additive term. See
/// [`crate::string::PluckedString::write_mixed_feedback`]'s doc comment for
/// why the two paths need opposite mixing laws, and why scaling this by the
/// string's own round-trip loss keeps any value in `[0, 1]` safe regardless
/// of `sustain`.
///
/// Set by measurement (`piano-audio/tests/sympathetic.rs`): a C5 held
/// silently rings about 37 dB under a struck C4, an audible but discreet
/// answer, and the sustain pedal's bloom sits about 9 dB under what is left
/// of the note one to three seconds after the strike.
/// A reasoned choice, not a measured bridge admittance (#90).
pub const DEFAULT_GLOBAL_COUPLING_GAIN: f32 = 0.2;

/// How hard the bridge drives a receptive string, given last block's
/// `total` over `contributors` keys.
///
/// Physically the bridge moves with the *sum* of every string's force
/// (Weinreich 1977), so a sounding key drives every listening string the
/// same however many others are listening: `gain · total`. The bus once
/// returned the *mean* instead, which kept it bounded but diluted every
/// key's drive by the number of receptive keys — so lifting the sustain
/// pedal, which makes all 88 receptive, cut sympathetic resonance by up to
/// 39 dB, exactly backwards. The sum is used now, capped only where
/// passivity needs it: every string writes back
/// `sustain · (own + (1 - sustain) · drive)`, so a common mode across `N`
/// keys has gain at most `sustain + sustain · (1 - sustain) · g · N`, which
/// stays at or below 1 while `g · N <= 1`. [`BRIDGE_PASSIVITY_MARGIN`]
/// keeps it strictly below.
#[inline]
fn bridge_drive_for(total: f32, contributors: u32, gain: f32) -> f32 {
    let passive_ceiling = BRIDGE_PASSIVITY_MARGIN / contributors.max(1) as f32;
    total * gain.min(passive_ceiling)
}

/// The fraction of the passivity bound [`bridge_drive_for`] lets the
/// shared bridge use, so a full keyboard of receptive strings still loses
/// energy rather than sitting on the edge of stability.
const BRIDGE_PASSIVITY_MARGIN: f32 = 0.9;

/// Spares everything in `own` except its part in the prompt mode `share`
/// of the string's round-trip loss `1 - sustain`, leaving `prompt_part` to
/// lose exactly `sustain` once `write_mixed_feedback` applies it.
///
/// With `ŵ` the group's bridge loads as a unit vector, one round trip
/// scatters the strings by `(1 + k)·I − k·ŵŵᵀ`, `k = share·(1 − sustain)/
/// sustain`: eigenvalue `1` on the prompt mode `ŵ`, `1 + k` on every motion
/// orthogonal to it. After `sustain` those are `sustain` and
/// `sustain + share·(1 − sustain) < 1` — the matrix is symmetric, so its
/// norm is its largest eigenvalue and the group stays passive for any
/// `share < 1`. A single string *is* its own prompt mode, so it is
/// untouched and decays exactly as `piano_audio::voicing` solved it.
#[inline]
fn spare_slow_modes(own: f32, prompt_part: f32, sustain: f32, share: f32) -> f32 {
    let share = math::clamp_or_low(share, 0.0, MAX_LOCAL_COUPLING_GAIN);
    if sustain.is_nan() || sustain <= 0.0 {
        return own;
    }
    let slow_mode_boost = share * (1.0 - sustain) / sustain;
    own + slow_mode_boost * (own - prompt_part)
}

/// How many cents string `string_index` of a `count`-string unison sits
/// from the group's pitch when its strings are `spread_cents` apart in all.
/// `_ => 0.0` covers the single string (nothing to mistune against) and any
/// out-of-table index, total rather than requiring the caller to
/// pre-validate. Public so a caller that retunes strings absolutely
/// ([`UnisonGroup::set_string_detune`]) can start from the same division.
#[must_use]
pub fn detune_cents(count: usize, string_index: usize, spread_cents: f32) -> f32 {
    let spread = math::clamp_or_low(spread_cents, 0.0, MAX_UNISON_SPREAD_CENTS);
    spread
        * table_entry(
            count,
            string_index,
            MISTUNING_SHARE_BICHORD,
            &MISTUNING_SHARE_TRICHORD,
            0.0,
        )
}

/// How hard string `string_index` of a `count`-string unison loads the
/// bridge, relative to the group's average. See [`BRIDGE_LOAD_TRICHORD`].
fn bridge_load_for(count: usize, string_index: usize) -> f32 {
    table_entry(
        count,
        string_index,
        BRIDGE_LOAD_BICHORD,
        &BRIDGE_LOAD_TRICHORD,
        1.0,
    )
}

/// `string_index`'s entry in the table for a `count`-string group, or
/// `single` for a monochord and for any index outside the table.
fn table_entry(
    count: usize,
    string_index: usize,
    bichord: [f32; 2],
    trichord: &[f32; 3],
    single: f32,
) -> f32 {
    let table: &[f32] = match count {
        2 => &bichord,
        3 => trichord,
        _ => &[],
    };
    table.get(string_index).copied().unwrap_or(single)
}

/// Shifts `frequency` by `cents`, falling back to the original frequency in
/// the unreachable case the shifted value is not representable — keeps this
/// total rather than propagating an error for what is, in practice, always
/// a small perturbation of an already-valid frequency.
fn detune(frequency: Hz, cents: f32) -> Hz {
    let ratio = math::powf(2.0, cents / 1200.0);
    Hz::new(frequency.hertz() * ratio).unwrap_or(frequency)
}

/// Standard modern-piano unison count for `key_index` (`0` = A0), per the
/// module docs' register boundaries. `pub(crate)`-visible so
/// `piano_audio::voicing` can reuse the same table rather than duplicating
/// it — the boundaries are a physical-model fact, not per-key policy, so
/// they belong here alongside [`MISTUNING_SHARE_BICHORD`]/
/// [`MISTUNING_SHARE_TRICHORD`], even though *which key gets which count* is
/// looked up by the voicing layer.
#[must_use]
pub fn unison_count_for_key_index(key_index: u8) -> usize {
    const MONOCHORD_KEYS: u8 = 12;
    const BICHORD_KEYS: u8 = 18;
    if key_index < MONOCHORD_KEYS {
        1
    } else if key_index < MONOCHORD_KEYS + BICHORD_KEYS {
        2
    } else {
        3
    }
}

// Split out to keep this file near the project's size limit; both still
// compile as children of this module.
#[cfg(test)]
#[path = "unison_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "unison_bridge_tests.rs"]
mod bridge_tests;
