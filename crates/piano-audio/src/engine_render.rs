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
use crate::limiter::soft_limit;

/// How far the keyboard's ends sit off centre: A0's strings reach the left
/// channel `(1 + w) / (1 - w)` times as strongly as the right — about
/// 9.5 dB at 0.5, a player's-eye width that stays well inside the pair
/// rather than hard-panned.
const KEYBOARD_STEREO_WIDTH: f32 = 0.5;

/// How much of the bridge force is heard undelayed, against the diffuse
/// field at unity: the board next to the bridge radiates at once, before
/// the bending waves have spread. Fitted together with the field's decay
/// to the reference's attack (`docs/CALIBRATION.md`): more of it and the
/// attack turns back into a plucked string's, less and it loses its edge.
const DIRECT_RADIATION_GAIN: f32 = 0.3;

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
                // The level comes last: phantoms grow with the square of the
                // string's own motion, which regulation must not rescale.
                let transverse = strings.process_with_bridge(&mut self.bridge, index);
                let coloured = voice.duplex.process(voice.phantom.process(transverse));
                let radiated = voice.level * coloured;
                *left += radiated * (1.0 + voice.pan);
                *right += radiated * (1.0 - voice.pan);
            }
        }
    }

    /// Lets the soundboard radiate each frame of bridge force, lets the
    /// room answer the result, and limits each channel.
    fn radiate_and_limit(&mut self, left: &mut [f32], right: &mut [f32]) {
        for (left, right) in left.iter_mut().zip(right.iter_mut()) {
            let (heard_left, heard_right) = self.radiate(*left, *right);
            let (room_left, room_right) = self.room.process(f32::midpoint(heard_left, heard_right));
            let wet_left = self.master_gain * (heard_left + room_left);
            let wet_right = self.master_gain * (heard_right + room_right);
            *left = soft_limit(wet_left, self.limiter_threshold);
            *right = soft_limit(wet_right, self.limiter_threshold);
        }
    }

    /// What the soundboard makes of one frame of bridge force: the part
    /// heard at once, the diffuse field that follows it and the board's
    /// separate low resonances, summed and radiated. The action's knock
    /// reaches the resonances only, through the frame, as before.
    fn radiate(&mut self, left: f32, right: f32) -> (f32, f32) {
        let board_drive = f32::midpoint(left, right) + self.action.next_sample();
        let (modes_left, modes_right) = self.soundboard.process_stereo(board_drive);
        let (field_left, field_right) = self.body.process(left, right);
        let mix = self.soundboard_mix_gain;
        let moved_left = DIRECT_RADIATION_GAIN * left + field_left + mix * modes_left;
        let moved_right = DIRECT_RADIATION_GAIN * right + field_right + mix * modes_right;
        self.radiation.process(moved_left, moved_right)
    }
}
