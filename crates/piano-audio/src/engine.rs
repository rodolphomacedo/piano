//! The realtime voice engine: one [`UnisonGroup`] pre-allocated for every
//! key on the keyboard, driven by commands drained from the audio thread's
//! [`Consumer`].
//!
//! Every method here runs on the audio thread. `Engine::new` is the only
//! allocating call — it builds all 88 voices, the shared bridge bus and the
//! soundboard up front, on the control thread, before the stream ever
//! starts. Everything after it is allocation-free, lock-free and
//! panic-free: `note_on` never constructs a new [`UnisonGroup`], only
//! retunes-in-place by calling the already allocation-free
//! [`UnisonGroup::pluck`] on a voice that has existed since construction.
//! An earlier version of this engine built a fresh voice per strike from a
//! small dynamic pool — that allocated on the audio thread on every
//! note-on, which is exactly the bug the no-allocation test in
//! `tests/no_allocation.rs` exists to catch.
//!
//! # M6: unison strings, the shared bridge bus, and the soundboard
//!
//! Three pieces landed together because they interact:
//!
//! - Each voice is now a [`UnisonGroup`] of 1-3 strings
//!   (`piano_audio::voicing::unison_count_for_key`), not a lone
//!   [`piano_core::PluckedString`] — real piano notes beat and have a
//!   two-stage decay because they *are* more than one string
//!   (`docs/ROADMAP.md`, M6).
//! - [`BridgeBus`] carries cross-key sympathetic resonance (`PERF-008`):
//!   [`Engine::process_block`] chunks its output into `bridge.capacity()`
//!   pieces and every voice reads/writes the bus once per sample within
//!   each chunk — see `piano_core::bridge` for why this is one block of
//!   latency rather than sample-accurate.
//! - [`Soundboard`] is a post-mix modal-synthesis stage (`PERF-009`): once
//!   a chunk's direct signal is fully mixed, it drives the soundboard and
//!   the radiated result is added back in.
//!
//! Sympathetic resonance also changes which voices energy gating
//! (`PERF-006`) can safely skip: a voice can be silent yet still need
//! processing, if the sustain pedal (or a held key) has its damper
//! lifted — see [`UnisonGroup::is_receptive`] and
//! [`Engine::process_chunk`]'s skip condition.

use piano_core::soundboard::SoundboardMode;
use piano_core::{BridgeBus, SampleRate, Soundboard, UnisonGroup, hammer};
use piano_params::{HIGHEST_PIANO_KEY, LOWEST_PIANO_KEY, PianoKey, Tuning};
use rtrb::Consumer;

use crate::commands::Command;
use crate::voicing;

/// Every key on a standard 88-key piano gets its own permanent voice.
const KEY_COUNT: usize = (HIGHEST_PIANO_KEY - LOWEST_PIANO_KEY + 1) as usize;

/// Commands drained per callback: an upper bound so the drain loop is bounded
/// even if the producer floods the ring faster than the audio thread reads it.
const MAX_COMMANDS_PER_CALLBACK: usize = 64;

/// How many samples of one block [`Engine`]'s [`BridgeBus`] accumulates
/// before every voice reads it back — see `piano_core::bridge`'s module
/// docs for why a block-delayed bus, not a sample-accurate one. 128 matches
/// the block size this project has used as its realistic audio-callback
/// quantum since M2/M3 (`docs/PERFORMANCE.md`, `PERF-010`, `PERF-011`), so
/// the cross-key coupling latency this introduces is about 2.7 ms at 48 kHz
/// — see the module docs on why that is negligible for this effect.
const BRIDGE_BLOCK_SAMPLES: usize = 128;

/// How strongly the soundboard's radiated signal is mixed back into the
/// direct one at [`Engine::process_chunk`]'s post-mix stage.
///
/// A parallel mix, not a replacement — `docs/PHYSICS.md` explains the
/// choice: replacing the direct signal entirely would be more faithful to
/// how a real piano is *only* heard through its soundboard, but would also
/// change the level and shape of the fundamental partial in every already-
/// measured tuning/inharmonicity test this project has (M1, M4). This
/// value was chosen by ear, not measured or fit, so the soundboard's
/// resonant colour is clearly present without dominating — an honest,
/// reasoned choice in the same spirit as `voicing`'s `BASS_DAMPING`.
const SOUNDBOARD_MIX_GAIN: f32 = 0.5;

/// One key's permanent voice. `None` only when `sample_rate` could not
/// represent that key's frequency at construction (see
/// [`UnisonGroup::new`]) — a real but rare degradation, not a bug.
struct Voice {
    strings: Option<UnisonGroup>,
    /// `true` from [`Engine::note_on`] until the matching
    /// [`Engine::note_off`], regardless of pedal state. Distinct from
    /// `pending_pedal_release`: this tracks whether a *finger* is
    /// currently holding the key, which is what decides whether the
    /// sustain pedal needs to lift or re-engage this voice's damper for
    /// sympathetic-resonance purposes ([`Engine::set_sustain_pedal`]).
    held: bool,
    /// Set by [`Engine::note_off`] when this key's `NoteOff` arrives while
    /// the sustain pedal is down: the voice keeps ringing, but is released
    /// for real the moment the pedal comes back up
    /// ([`Engine::release_pedal_held_voices`]). Cleared by a fresh
    /// [`Engine::note_on`] on the same key, since a re-strike is held by
    /// the finger again, not by the pedal.
    pending_pedal_release: bool,
}

/// Owns one voice per key, the shared bridge bus and the soundboard, all
/// tuned for the sample rate given at construction.
pub(crate) struct Engine {
    voices: [Voice; KEY_COUNT],
    /// CC64 hold state. See [`Command::SustainPedal`].
    pedal_down: bool,
    /// Cross-key sympathetic resonance (`PERF-008`). See the module docs.
    bridge: BridgeBus,
    /// Post-mix modal-synthesis soundboard (`PERF-009`). See the module
    /// docs.
    soundboard: Soundboard,
}

impl Engine {
    /// Builds every voice for `sample_rate` and `tuning`, plus the shared
    /// bridge bus and soundboard. The only allocating call in this type;
    /// always called on the control thread, before the audio stream
    /// starts.
    #[must_use]
    pub(crate) fn new(sample_rate: SampleRate, tuning: Tuning) -> Self {
        Self {
            voices: core::array::from_fn(|index| voice_for_key(index, sample_rate, tuning)),
            pedal_down: false,
            bridge: BridgeBus::with_capacity(BRIDGE_BLOCK_SAMPLES),
            soundboard: Soundboard::new(sample_rate),
        }
    }

    /// Drains up to [`MAX_COMMANDS_PER_CALLBACK`] pending commands from
    /// `consumer`, applying each to the voice pool.
    pub(crate) fn drain_commands(&mut self, consumer: &mut Consumer<Command>) {
        for _ in 0..MAX_COMMANDS_PER_CALLBACK {
            let Ok(command) = consumer.pop() else {
                break;
            };
            self.apply(command);
        }
    }

    /// Renders `output.len()` mixed, soundboard-coloured samples, adding
    /// into `output` after zeroing it. Chunked into
    /// [`BRIDGE_BLOCK_SAMPLES`]-sized pieces so [`BridgeBus`] always sees a
    /// block no longer than it was sized for — a bounded loop over a
    /// caller-supplied length, not a recursive or unbounded one.
    pub(crate) fn process_block(&mut self, output: &mut [f32]) {
        output.fill(0.0);
        for chunk in output.chunks_mut(BRIDGE_BLOCK_SAMPLES) {
            self.process_chunk(chunk);
        }
    }

    /// Processes one bridge-bus block: every voice mixes into `chunk`
    /// (skipped only when genuinely inert, see below), then the
    /// soundboard's radiated contribution is mixed back in.
    ///
    /// A voice is skipped only when it is *both* silent and fully damped
    /// (`!is_receptive`) — energy gating's original condition
    /// (`is_silent`, `PERF-006`) alone would also skip a silent voice
    /// whose damper the pedal has lifted, which must still be processed so
    /// it can pick up sympathetic energy from [`Engine::bridge`] and wake
    /// up; a fully damped voice genuinely cannot, so skipping it is exact.
    fn process_chunk(&mut self, chunk: &mut [f32]) {
        self.bridge.begin_block();
        for voice in &mut self.voices {
            let Some(strings) = voice.strings.as_mut() else {
                continue;
            };
            if strings.is_silent() && !strings.is_receptive() {
                continue;
            }
            for (index, sample) in chunk.iter_mut().enumerate() {
                *sample += strings.process_with_bridge(&mut self.bridge, index);
            }
        }
        for sample in chunk.iter_mut() {
            *sample += SOUNDBOARD_MIX_GAIN * self.soundboard.process(*sample);
        }
    }

    fn apply(&mut self, command: Command) {
        match command {
            Command::NoteOn { midi, velocity } => self.note_on(midi, velocity),
            Command::AllNotesOff => self.silence_all(),
            Command::SetDamping { damping } => self.set_damping(damping),
            Command::SetSustain { sustain } => self.set_sustain(sustain),
            Command::NoteOff { midi } => self.note_off(midi),
            Command::SustainPedal { down } => self.set_sustain_pedal(down),
            Command::SetSoundboardMode { index, mode } => self.set_soundboard_mode(index, mode),
            Command::SetLocalCouplingGain { gain } => self.set_local_coupling_gain(gain),
            Command::SetGlobalCouplingGain { gain } => self.set_global_coupling_gain(gain),
            Command::SetStringDamping {
                midi,
                string_index,
                damping,
            } => self.set_string_damping(midi, string_index, damping),
            Command::SetStringSustain {
                midi,
                string_index,
                sustain,
            } => self.set_string_sustain(midi, string_index, sustain),
            Command::SetStringInharmonicity {
                midi,
                string_index,
                inharmonicity,
            } => self.set_string_inharmonicity(midi, string_index, inharmonicity),
            Command::SetStringDetune {
                midi,
                string_index,
                cents,
            } => self.set_string_detune(midi, string_index, cents),
            Command::SetStringSeed {
                midi,
                string_index,
                seed,
            } => self.set_string_seed(midi, string_index, seed),
            Command::SetStringHammer {
                midi,
                string_index,
                hammer,
            } => self.set_string_hammer(midi, string_index, hammer),
        }
    }

    /// Re-strikes the voice already allocated for `midi`. Silently ignores
    /// notes off the keyboard and keys whose voice could not be tuned at
    /// construction — the same "invalid input is dropped" pattern used
    /// throughout the audio path, since the audio thread cannot report an
    /// error to anyone.
    fn note_on(&mut self, midi: u8, velocity: f32) {
        let Some(voice) = self.voice_for_midi(midi) else {
            return;
        };
        voice.held = true;
        // Struck again by the finger, not held by the pedal any more —
        // even if this key's previous ringing was pedal-held when it was
        // re-struck.
        voice.pending_pedal_release = false;
        let Some(strings) = voice.strings.as_mut() else {
            return;
        };
        strings.pluck(velocity);
    }

    /// Releases `midi`'s voice — a MIDI note-off or a computer-keyboard
    /// key-up. While the sustain pedal is down, the voice is marked
    /// [`Voice::pending_pedal_release`] instead of released immediately;
    /// [`Engine::release_pedal_held_voices`] finishes the job once the
    /// pedal comes back up. `held` clears unconditionally, regardless of
    /// pedal state — it tracks the finger, not the sound.
    fn note_off(&mut self, midi: u8) {
        let pedal_down = self.pedal_down;
        let Some(voice) = self.voice_for_midi(midi) else {
            return;
        };
        voice.held = false;
        if pedal_down {
            voice.pending_pedal_release = true;
            return;
        }
        if let Some(strings) = voice.strings.as_mut() {
            strings.release();
        }
    }

    /// Sets the CC64 hold state. Pressing the pedal also lifts the damper
    /// on every currently-idle voice, so it becomes receptive to
    /// sympathetic resonance (`PERF-008`, M6) exactly like a real piano's
    /// pedal lifting every damper regardless of which keys are held;
    /// releasing it releases every non-held voice, both the ones
    /// `pending_pedal_release` marked and the ones that were merely made
    /// receptive.
    fn set_sustain_pedal(&mut self, down: bool) {
        let was_down = self.pedal_down;
        self.pedal_down = down;
        if !was_down && down {
            self.lift_dampers_for_sympathetic_resonance();
        }
        if was_down && !down {
            self.release_pedal_held_voices();
        }
    }

    /// Lifts the damper on every voice not currently held by a finger —
    /// see [`Engine::set_sustain_pedal`]. Held voices need no change:
    /// [`UnisonGroup::pluck`] already lifted their damper.
    fn lift_dampers_for_sympathetic_resonance(&mut self) {
        for voice in &mut self.voices {
            if voice.held {
                continue;
            }
            if let Some(strings) = voice.strings.as_mut() {
                strings.lift_damper();
            }
        }
    }

    /// Re-engages the damper on every voice not currently held by a
    /// finger, once the pedal comes back up — covers both voices with a
    /// pending release and idle voices [`Engine::lift_dampers_for_
    /// sympathetic_resonance`] made receptive; [`UnisonGroup::release`] is
    /// idempotent, so calling it on an already-silent, already-damped
    /// voice is harmless. Bounded by [`KEY_COUNT`] — a compile-time
    /// maximum, not an unbounded scan — same as
    /// [`Engine::drain_commands`]'s own bounded loop.
    fn release_pedal_held_voices(&mut self) {
        for voice in &mut self.voices {
            if voice.held {
                continue;
            }
            voice.pending_pedal_release = false;
            if let Some(strings) = voice.strings.as_mut() {
                strings.release();
            }
        }
    }

    /// Looks up the voice for a MIDI note number, if it names a real piano
    /// key. Shared by every per-key command handler.
    fn voice_for_midi(&mut self, midi: u8) -> Option<&mut Voice> {
        let key = PianoKey::from_midi(midi).ok()?;
        self.voices.get_mut(usize::from(key.key_index()))
    }

    /// Looks up `midi`'s [`UnisonGroup`], if the note names a real piano key
    /// and that key was tunable at construction. Shared by every per-string
    /// command handler, which all need exactly this same
    /// `voice_for_midi` + `strings.as_mut()` chain.
    fn unison_for_midi(&mut self, midi: u8) -> Option<&mut UnisonGroup> {
        self.voice_for_midi(midi)?.strings.as_mut()
    }

    /// Zero-velocity pluck silences a voice using the same allocation-free
    /// path as a normal strike, then immediately re-engages the damper —
    /// "all notes off" should leave every voice damped, not sympathetically
    /// receptive, regardless of pedal state.
    fn silence_all(&mut self) {
        for voice in &mut self.voices {
            voice.held = false;
            voice.pending_pedal_release = false;
            if let Some(strings) = voice.strings.as_mut() {
                strings.pluck(0.0);
                strings.release();
            }
        }
    }

    /// Applies a new damping to every voice, live — including voices
    /// currently ringing, per
    /// [`UnisonGroup::set_damping`](piano_core::UnisonGroup::set_damping).
    /// A global "brightness" knob, not per-key: a hardware controller has
    /// one physical knob, not eighty-eight.
    fn set_damping(&mut self, damping: f32) {
        for voice in &mut self.voices {
            if let Some(strings) = voice.strings.as_mut() {
                strings.set_damping(damping);
            }
        }
    }

    /// Applies a new sustain to every voice, live. Same "one global knob"
    /// reasoning as [`Engine::set_damping`].
    fn set_sustain(&mut self, sustain: f32) {
        for voice in &mut self.voices {
            if let Some(strings) = voice.strings.as_mut() {
                strings.set_sustain(sustain);
            }
        }
    }

    /// Rebuilds soundboard mode `index` live. There is only ever one
    /// [`Soundboard`], shared by every voice's post-mix output, so — unlike
    /// [`Engine::set_damping`] — this reaches straight into
    /// [`Engine::soundboard`] rather than broadcasting across
    /// [`Engine::voices`].
    fn set_soundboard_mode(&mut self, index: usize, mode: SoundboardMode) {
        self.soundboard.set_mode(index, mode);
    }

    /// Applies a new local (within-group) coupling gain to every voice,
    /// live. Same "one global knob" broadcast as [`Engine::set_damping`].
    fn set_local_coupling_gain(&mut self, gain: f32) {
        for voice in &mut self.voices {
            if let Some(strings) = voice.strings.as_mut() {
                strings.set_local_coupling_gain(gain);
            }
        }
    }

    /// Applies a new global (cross-key, [`BridgeBus`]) coupling gain to
    /// every voice, live. Same "one global knob" broadcast as
    /// [`Engine::set_damping`].
    fn set_global_coupling_gain(&mut self, gain: f32) {
        for voice in &mut self.voices {
            if let Some(strings) = voice.strings.as_mut() {
                strings.set_global_coupling_gain(gain);
            }
        }
    }

    /// Sets one string's damping, live. See
    /// [`UnisonGroup::set_string_damping`] for the out-of-range
    /// `string_index` contract; an unrecognised `midi` is silently ignored
    /// the same way [`Engine::note_on`] already ignores one.
    fn set_string_damping(&mut self, midi: u8, string_index: u8, damping: f32) {
        if let Some(strings) = self.unison_for_midi(midi) {
            strings.set_string_damping(usize::from(string_index), damping);
        }
    }

    /// Sets one string's sustain, live. See [`Engine::set_string_damping`]
    /// for the out-of-range contract.
    fn set_string_sustain(&mut self, midi: u8, string_index: u8, sustain: f32) {
        if let Some(strings) = self.unison_for_midi(midi) {
            strings.set_string_sustain(usize::from(string_index), sustain);
        }
    }

    /// Sets one string's inharmonicity coefficient, live. See
    /// [`Engine::set_string_damping`] for the out-of-range contract.
    fn set_string_inharmonicity(&mut self, midi: u8, string_index: u8, inharmonicity: f32) {
        if let Some(strings) = self.unison_for_midi(midi) {
            strings.set_string_inharmonicity(usize::from(string_index), inharmonicity);
        }
    }

    /// Retunes one string, live. See [`Engine::set_string_damping`] for the
    /// out-of-range contract.
    fn set_string_detune(&mut self, midi: u8, string_index: u8, cents: f32) {
        if let Some(strings) = self.unison_for_midi(midi) {
            strings.set_string_detune(usize::from(string_index), cents);
        }
    }

    /// Reseeds one string's excitation noise for its next strike. See
    /// [`Engine::set_string_damping`] for the out-of-range contract.
    fn set_string_seed(&mut self, midi: u8, string_index: u8, seed: u32) {
        if let Some(strings) = self.unison_for_midi(midi) {
            strings.set_string_seed(usize::from(string_index), seed);
        }
    }

    /// Changes one string's felt-contact physics for its next strike. See
    /// [`Engine::set_string_damping`] for the out-of-range contract.
    fn set_string_hammer(&mut self, midi: u8, string_index: u8, hammer: hammer::HammerConfig) {
        if let Some(strings) = self.unison_for_midi(midi) {
            strings.set_string_hammer(usize::from(string_index), hammer);
        }
    }
}

/// Builds `key_index`'s permanent voice: a [`UnisonGroup`] whose string
/// count and baseline damping/sustain/inharmonicity are computed per key
/// by [`voicing::unison_count_for_key`] and [`voicing::config_for_key`]
/// rather than left at one global default across all 88 keys — see the
/// module docs of [`crate::voicing`].
fn voice_for_key(key_index: usize, sample_rate: SampleRate, tuning: Tuning) -> Voice {
    let midi = LOWEST_PIANO_KEY.saturating_add(key_index as u8);
    let Ok(key) = PianoKey::from_midi(midi) else {
        return Voice {
            strings: None,
            held: false,
            pending_pedal_release: false,
        };
    };
    let config = voicing::config_for_key(key, tuning);
    let unison_count = voicing::unison_count_for_key(key);
    Voice {
        strings: UnisonGroup::new(config, unison_count, sample_rate).ok(),
        held: false,
        pending_pedal_release: false,
    }
}

// Split into `engine_tests.rs` to keep this file under the project's
// 500-line limit (`CONTRIBUTING.md`) — still compiles as `engine::tests`.
#[cfg(test)]
#[path = "engine_tests.rs"]
mod tests;
