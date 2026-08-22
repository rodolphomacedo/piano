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

use piano_core::string::StringConfig;
use piano_core::{PluckedString, SampleRate};
use piano_params::{HIGHEST_PIANO_KEY, LOWEST_PIANO_KEY, PianoKey, Tuning};
use rtrb::Consumer;

use crate::commands::Command;

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
}

/// Owns one voice per key, all tuned for the sample rate given at
/// construction.
pub(crate) struct Engine {
    voices: [Voice; KEY_COUNT],
}

impl Engine {
    /// Builds every voice for `sample_rate` and `tuning`. The only
    /// allocating call in this type; always called on the control thread,
    /// before the audio stream starts.
    #[must_use]
    pub(crate) fn new(sample_rate: SampleRate, tuning: Tuning) -> Self {
        Self {
            voices: core::array::from_fn(|index| voice_for_key(index, sample_rate, tuning)),
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
        }
    }

    /// Re-strikes the voice already allocated for `midi`. Silently ignores
    /// notes off the keyboard and keys whose voice could not be tuned at
    /// construction — the same "invalid input is dropped" pattern used
    /// throughout the audio path, since the audio thread cannot report an
    /// error to anyone.
    fn note_on(&mut self, midi: u8, velocity: f32) {
        let Ok(key) = PianoKey::from_midi(midi) else {
            return;
        };
        let Some(voice) = self.voices.get_mut(usize::from(key.key_index())) else {
            return;
        };
        let Some(string) = voice.string.as_mut() else {
            return;
        };
        string.pluck(velocity);
    }

    /// Zero-velocity pluck silences a voice using the same allocation-free
    /// path as a normal strike: it clears the delay line, resets the loss
    /// filter and DC blocker, and sets the envelope to zero.
    fn silence_all(&mut self) {
        for voice in &mut self.voices {
            if let Some(string) = voice.string.as_mut() {
                string.pluck(0.0);
            }
        }
    }
}

fn voice_for_key(key_index: usize, sample_rate: SampleRate, tuning: Tuning) -> Voice {
    let midi = LOWEST_PIANO_KEY.saturating_add(key_index as u8);
    let Ok(key) = PianoKey::from_midi(midi) else {
        return Voice { string: None };
    };
    let config = StringConfig::new(key.frequency(tuning));
    Voice {
        string: PluckedString::new(config, sample_rate).ok(),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    fn engine() -> Engine {
        let rate = SampleRate::new(48_000.0).expect("48 kHz is valid");
        Engine::new(rate, Tuning::default())
    }

    fn ring_buffer() -> (rtrb::Producer<Command>, Consumer<Command>) {
        ring_buffer_with_capacity(16)
    }

    fn ring_buffer_with_capacity(capacity: usize) -> (rtrb::Producer<Command>, Consumer<Command>) {
        rtrb::RingBuffer::new(capacity)
    }

    #[test]
    fn a_note_on_command_produces_sound() {
        let mut engine = engine();
        let (mut producer, mut consumer) = ring_buffer();
        producer
            .push(Command::NoteOn {
                midi: 69,
                velocity: 1.0,
            })
            .expect("queue has room");
        engine.drain_commands(&mut consumer);

        let mut buffer = [0.0f32; 512];
        engine.process_block(&mut buffer);
        assert!(buffer.iter().any(|sample| sample.abs() > 1e-3));
    }

    #[test]
    fn an_out_of_range_note_is_ignored_without_panicking() {
        let mut engine = engine();
        let (mut producer, mut consumer) = ring_buffer();
        producer
            .push(Command::NoteOn {
                midi: 200,
                velocity: 1.0,
            })
            .expect("queue has room");
        engine.drain_commands(&mut consumer);

        let mut buffer = [0.0f32; 64];
        engine.process_block(&mut buffer);
        assert!(buffer.iter().all(|sample| *sample == 0.0));
    }

    #[test]
    fn all_notes_off_silences_every_voice() {
        let mut engine = engine();
        let (mut producer, mut consumer) = ring_buffer();
        producer
            .push(Command::NoteOn {
                midi: 69,
                velocity: 1.0,
            })
            .expect("queue has room");
        engine.drain_commands(&mut consumer);
        producer.push(Command::AllNotesOff).expect("queue has room");
        engine.drain_commands(&mut consumer);

        let mut buffer = [0.0f32; 64];
        engine.process_block(&mut buffer);
        assert!(buffer.iter().all(|sample| *sample == 0.0));
    }

    #[test]
    fn striking_every_key_at_once_never_panics() {
        let mut engine = engine();
        let (mut producer, mut consumer) = ring_buffer_with_capacity(KEY_COUNT);
        for midi in LOWEST_PIANO_KEY..=HIGHEST_PIANO_KEY {
            producer
                .push(Command::NoteOn {
                    midi,
                    velocity: 1.0,
                })
                .expect("queue has room");
        }
        engine.drain_commands(&mut consumer);

        let mut buffer = [0.0f32; 64];
        engine.process_block(&mut buffer);
        assert!(buffer.iter().any(|sample| sample.is_finite()));
    }

    #[test]
    fn re_striking_the_same_key_never_panics() {
        let mut engine = engine();
        let (mut producer, mut consumer) = ring_buffer_with_capacity(64);
        for _ in 0..64 {
            producer
                .push(Command::NoteOn {
                    midi: 69,
                    velocity: 0.9,
                })
                .expect("queue has room");
        }
        engine.drain_commands(&mut consumer);
        let mut buffer = [0.0f32; 64];
        engine.process_block(&mut buffer);
    }

    #[test]
    fn flooding_the_queue_beyond_the_drain_cap_never_panics() {
        let mut engine = engine();
        let (mut producer, mut consumer) = ring_buffer();
        for _ in 0..(MAX_COMMANDS_PER_CALLBACK + 8) {
            let _ = producer.push(Command::NoteOn {
                midi: 69,
                velocity: 0.5,
            });
        }
        engine.drain_commands(&mut consumer);
        let mut buffer = [0.0f32; 32];
        engine.process_block(&mut buffer);
    }

    #[test]
    fn a_note_the_engine_cannot_tune_is_ignored_without_panicking() {
        let rate = SampleRate::new(8_000.0).expect("8 kHz is valid");
        let mut engine = Engine::new(rate, Tuning::default());
        let (mut producer, mut consumer) = ring_buffer();
        let key = PianoKey::from_midi(HIGHEST_PIANO_KEY).expect("C8 is on the keyboard");
        producer
            .push(Command::NoteOn {
                midi: key.midi_number(),
                velocity: 1.0,
            })
            .expect("queue has room");
        engine.drain_commands(&mut consumer);
        let mut buffer = [0.0f32; 16];
        engine.process_block(&mut buffer);
    }
}
