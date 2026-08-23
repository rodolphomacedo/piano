//! The realtime voice engine: one [`PluckedString`] pre-allocated for every
//! key on the keyboard, driven by commands drained from the audio thread's
//! [`Consumer`].
//!
//! Every method here runs on the audio thread. `Engine::new` is the only
//! allocating call — it builds all 88 voices up front, on the control
//! thread, before the stream ever starts. Everything after it is
//! allocation-free, lock-free and panic-free: `note_on` never constructs a
//! new [`PluckedString`], only retunes-in-place by calling the already
//! allocation-free [`PluckedString::pluck`] on a voice that has existed
//! since construction. An earlier version of this engine built a fresh
//! voice per strike from a small dynamic pool — that allocated on the audio
//! thread on every note-on, which is exactly the bug the no-allocation test
//! in `tests/no_allocation.rs` exists to catch.

use piano_core::{PluckedString, SampleRate};
use piano_params::{HIGHEST_PIANO_KEY, LOWEST_PIANO_KEY, PianoKey, Tuning};
use rtrb::Consumer;

use crate::commands::Command;
use crate::voicing;

/// Every key on a standard 88-key piano gets its own permanent voice.
const KEY_COUNT: usize = (HIGHEST_PIANO_KEY - LOWEST_PIANO_KEY + 1) as usize;

/// Commands drained per callback: an upper bound so the drain loop is bounded
/// even if the producer floods the ring faster than the audio thread reads it.
const MAX_COMMANDS_PER_CALLBACK: usize = 64;

/// One key's permanent voice. `None` only when `sample_rate` could not
/// represent that key's frequency at construction (see
/// [`PluckedString::new`]) — a real but rare degradation, not a bug.
struct Voice {
    string: Option<PluckedString>,
    /// Set by [`Engine::note_off`] when this key's `NoteOff` arrives while
    /// the sustain pedal is down: the voice keeps ringing, but is released
    /// for real the moment the pedal comes back up
    /// ([`Engine::release_pedal_held_voices`]). Cleared by a fresh
    /// [`Engine::note_on`] on the same key, since a re-strike is held by
    /// the finger again, not by the pedal.
    pending_pedal_release: bool,
}

/// Owns one voice per key, all tuned for the sample rate given at
/// construction.
pub(crate) struct Engine {
    voices: [Voice; KEY_COUNT],
    /// CC64 hold state. See [`Command::SustainPedal`].
    pedal_down: bool,
}

impl Engine {
    /// Builds every voice for `sample_rate` and `tuning`. The only
    /// allocating call in this type; always called on the control thread,
    /// before the audio stream starts.
    #[must_use]
    pub(crate) fn new(sample_rate: SampleRate, tuning: Tuning) -> Self {
        Self {
            voices: core::array::from_fn(|index| voice_for_key(index, sample_rate, tuning)),
            pedal_down: false,
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

    /// Renders `output.len()` mixed samples from every ringing voice, adding
    /// into `output` after zeroing it. Voices that have not been struck, or
    /// have decayed to silence, are skipped rather than processed for
    /// nothing.
    pub(crate) fn process_block(&mut self, output: &mut [f32]) {
        output.fill(0.0);
        for voice in &mut self.voices {
            let Some(string) = voice.string.as_mut() else {
                continue;
            };
            if string.is_silent() {
                continue;
            }
            string.process_block_add(output);
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
        // Struck again by the finger, not held by the pedal any more —
        // even if this key's previous ringing was pedal-held when it was
        // re-struck.
        voice.pending_pedal_release = false;
        let Some(string) = voice.string.as_mut() else {
            return;
        };
        string.pluck(velocity);
    }

    /// Releases `midi`'s voice — a MIDI note-off or a computer-keyboard
    /// key-up. While the sustain pedal is down, the voice is marked
    /// [`Voice::pending_pedal_release`] instead of released immediately;
    /// [`Engine::release_pedal_held_voices`] finishes the job once the
    /// pedal comes back up.
    fn note_off(&mut self, midi: u8) {
        let pedal_down = self.pedal_down;
        let Some(voice) = self.voice_for_midi(midi) else {
            return;
        };
        if pedal_down {
            voice.pending_pedal_release = true;
            return;
        }
        if let Some(string) = voice.string.as_mut() {
            string.release();
        }
    }

    /// Sets the CC64 hold state. Pressing the pedal changes nothing by
    /// itself; releasing it releases every voice
    /// [`Voice::pending_pedal_release`] marked while it was down.
    fn set_sustain_pedal(&mut self, down: bool) {
        let was_down = self.pedal_down;
        self.pedal_down = down;
        if was_down && !down {
            self.release_pedal_held_voices();
        }
    }

    /// Releases every voice the pedal was holding. Bounded by
    /// [`KEY_COUNT`] — a compile-time maximum, not an unbounded scan —
    /// same as [`Engine::drain_commands`]'s own bounded loop.
    fn release_pedal_held_voices(&mut self) {
        for voice in &mut self.voices {
            if !voice.pending_pedal_release {
                continue;
            }
            voice.pending_pedal_release = false;
            if let Some(string) = voice.string.as_mut() {
                string.release();
            }
        }
    }

    /// Looks up the voice for a MIDI note number, if it names a real piano
    /// key. Shared by every per-key command handler.
    fn voice_for_midi(&mut self, midi: u8) -> Option<&mut Voice> {
        let key = PianoKey::from_midi(midi).ok()?;
        self.voices.get_mut(usize::from(key.key_index()))
    }

    /// Zero-velocity pluck silences a voice using the same allocation-free
    /// path as a normal strike: it clears the delay line, resets the loss
    /// filter and DC blocker, and sets the envelope to zero.
    fn silence_all(&mut self) {
        for voice in &mut self.voices {
            voice.pending_pedal_release = false;
            if let Some(string) = voice.string.as_mut() {
                string.pluck(0.0);
            }
        }
    }

    /// Applies a new damping to every voice, live — including voices
    /// currently ringing, per
    /// [`PluckedString::set_damping`](piano_core::PluckedString::set_damping).
    /// A global "brightness" knob, not per-key: a hardware controller has
    /// one physical knob, not eighty-eight.
    fn set_damping(&mut self, damping: f32) {
        for voice in &mut self.voices {
            if let Some(string) = voice.string.as_mut() {
                string.set_damping(damping);
            }
        }
    }

    /// Applies a new sustain to every voice, live. Same "one global knob"
    /// reasoning as [`Engine::set_damping`].
    fn set_sustain(&mut self, sustain: f32) {
        for voice in &mut self.voices {
            if let Some(string) = voice.string.as_mut() {
                string.set_sustain(sustain);
            }
        }
    }
}

/// Builds `key_index`'s permanent voice, its baseline damping, sustain and
/// inharmonicity computed per key by [`voicing::config_for_key`] rather
/// than left at one global default across all 88 keys — see the module
/// docs of [`crate::voicing`].
fn voice_for_key(key_index: usize, sample_rate: SampleRate, tuning: Tuning) -> Voice {
    let midi = LOWEST_PIANO_KEY.saturating_add(key_index as u8);
    let Ok(key) = PianoKey::from_midi(midi) else {
        return Voice {
            string: None,
            pending_pedal_release: false,
        };
    };
    let config = voicing::config_for_key(key, tuning);
    Voice {
        string: PluckedString::new(config, sample_rate).ok(),
        pending_pedal_release: false,
    }
}

// Split into `engine_tests.rs` to keep this file under the project's
// 500-line limit (`CONTRIBUTING.md`) — still compiles as `engine::tests`.
#[cfg(test)]
#[path = "engine_tests.rs"]
mod tests;
