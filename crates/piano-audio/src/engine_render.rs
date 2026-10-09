//! How [`Engine`] turns its voices into output samples: every voice mixed
//! into a stereo pair at its place on the keyboard, the soundboard heard
//! from two microphone positions, then the master gain and limiter.
//!
//! A piano heard from the bench is wide: the bass strings run to the
//! player's left, the treble to the right, and the soundboard's modes
//! reach the two ears at different strengths. A mono render collapses all
//! of that into one point. Every stereo weight here is `1 ± spread`, so the
//! mean of the two channels is exactly the mono instrument: the mono path
//! ([`Engine::process_block`]) and every level calibrated against it are
//! unchanged by the image.

use piano_params::{HIGHEST_PIANO_KEY, LOWEST_PIANO_KEY, PianoKey};

use super::{BRIDGE_BLOCK_SAMPLES, Engine};
use crate::limiter::{OUTPUT_LIMITER_THRESHOLD, soft_limit};

/// How far the keyboard's ends sit off centre: A0's strings reach the left
/// channel `(1 + w) / (1 - w)` times as strongly as the right — about
/// 9.5 dB at 0.5, a player's-eye width that stays well inside the pair
/// rather than hard-panned.
const KEYBOARD_STEREO_WIDTH: f32 = 0.5;

/// Where `key` sits in the stereo pair: `+KEYBOARD_STEREO_WIDTH` (left) at
/// A0 to `-KEYBOARD_STEREO_WIDTH` (right) at C8, linear in key number the
/// way the strings fan out across the frame.
#[must_use]
pub(super) fn pan_for_key(key: PianoKey) -> f32 {
    let span = f32::from(HIGHEST_PIANO_KEY - LOWEST_PIANO_KEY);
    let position = f32::from(key.midi_number() - LOWEST_PIANO_KEY) / span;
    KEYBOARD_STEREO_WIDTH * (1.0 - 2.0 * position)
}

impl Engine {
    /// Renders `output.len()` mono samples: the mean of the stereo pair
    /// [`Engine::process_block_stereo`] would produce. Chunked into
    /// [`BRIDGE_BLOCK_SAMPLES`]-sized pieces so the bridge bus always sees a
    /// block no longer than it was sized for.
    pub(crate) fn process_block(&mut self, output: &mut [f32]) {
        let mut right = [0.0f32; BRIDGE_BLOCK_SAMPLES];
        for chunk in output.chunks_mut(BRIDGE_BLOCK_SAMPLES) {
            let Some(right) = right.get_mut(..chunk.len()) else {
                continue;
            };
            self.process_stereo_chunk(chunk, right);
            for (centre, &right) in chunk.iter_mut().zip(right.iter()) {
                *centre = f32::midpoint(*centre, right);
            }
        }
    }

    /// Renders a stereo pair into `left` and `right`, as many frames as
    /// the shorter of the two holds.
    pub(crate) fn process_block_stereo(&mut self, left: &mut [f32], right: &mut [f32]) {
        let chunks = left
            .chunks_mut(BRIDGE_BLOCK_SAMPLES)
            .zip(right.chunks_mut(BRIDGE_BLOCK_SAMPLES));
        for (left, right) in chunks {
            self.process_stereo_chunk(left, right);
        }
    }

    /// One bridge-bus block, overwriting `left` and `right`.
    fn process_stereo_chunk(&mut self, left: &mut [f32], right: &mut [f32]) {
        left.fill(0.0);
        right.fill(0.0);
        self.bridge.begin_block();
        self.mix_voices(left, right);
        self.radiate_and_limit(left, right);
    }

    /// Adds every audible voice into the pair at its own pan.
    ///
    /// A voice is skipped only when it is *both* silent and fully damped
    /// (`!is_receptive`): a silent voice whose damper the pedal has lifted
    /// must still run so it can pick up sympathetic energy from the bridge
    /// bus and wake up (`PERF-006`, `PERF-008`).
    fn mix_voices(&mut self, left: &mut [f32], right: &mut [f32]) {
        for voice in &mut self.voices {
            let Some(strings) = voice.strings.as_mut() else {
                continue;
            };
            if strings.is_silent() && !strings.is_receptive() {
                continue;
            }
            let frames = left.iter_mut().zip(right.iter_mut()).enumerate();
            for (index, (left, right)) in frames {
                let transverse = voice.level * strings.process_with_bridge(&mut self.bridge, index);
                let radiated = voice.phantom.process(transverse);
                *left += radiated * (1.0 + voice.pan);
                *right += radiated * (1.0 - voice.pan);
            }
        }
    }

    /// Drives the soundboard with the strings' mono sum plus the action's
    /// knocks, adds its two microphone signals, and limits each channel.
    fn radiate_and_limit(&mut self, left: &mut [f32], right: &mut [f32]) {
        for (left, right) in left.iter_mut().zip(right.iter_mut()) {
            let board_drive = f32::midpoint(*left, *right) + self.action.next_sample();
            let (board_left, board_right) = self.soundboard.process_stereo(board_drive);
            let mixed_left = *left + self.soundboard_mix_gain * board_left;
            let mixed_right = *right + self.soundboard_mix_gain * board_right;
            *left = soft_limit(self.master_gain * mixed_left, OUTPUT_LIMITER_THRESHOLD);
            *right = soft_limit(self.master_gain * mixed_right, OUTPUT_LIMITER_THRESHOLD);
        }
    }
}
