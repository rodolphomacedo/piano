//! The three pedals and the key releases they interact with.
//!
//! Every key's damper is derived from the same state, each time anything
//! that could move it changes: a key held by a finger, or caught by the
//! sostenuto, has its damper lifted; any other key's damper rests with the
//! pressure the sustain pedal's position leaves it
//! ([`damper_pressure_for_sustain_position`]). That one rule covers a plain
//! release, a pedal-held release, sympathetic resonance with the pedal down
//! (every free damper lifts, `PERF-008`), half-pedalling and the sostenuto,
//! without a separate code path for each.

use piano_core::math;

use super::{Engine, Voice};

/// Pedal travel below which every damper still rests fully on its string
/// — the free play at the top of a real sustain pedal's stroke.
const SUSTAIN_DAMPERS_START_LIFTING: f32 = 0.25;

/// Pedal travel at which every damper has fully cleared its string.
///
/// The span between the two is the half-pedal zone, where the felt grazes
/// the string and shortens its ring without stopping it — the graded
/// sustain pianists use constantly (H.-M. Lehtonen, H. Penttinen,
/// J. Rauhala & V. Välimäki, "Analysis and modeling of piano sustain-pedal
/// effects", JASA 122(3), 2007, describe the pedal's partial-damping
/// region). The two bounds are this project's own reading of a typical
/// grand's stroke, not a measured curve.
const SUSTAIN_DAMPERS_FULLY_LIFTED: f32 = 0.7;

/// Where the pedals are.
#[derive(Debug, Clone, Copy, Default)]
pub(super) struct PedalState {
    sustain_position: f32,
    sostenuto_down: bool,
    pub(super) soft_down: bool,
}

/// How hard a free key's damper presses at sustain pedal `position`:
/// `1` at rest, `0` once lifted, a smoothstep through the half-pedal zone
/// so the decay rate has no corner anywhere along the stroke.
fn damper_pressure_for_sustain_position(position: f32) -> f32 {
    let span = SUSTAIN_DAMPERS_FULLY_LIFTED - SUSTAIN_DAMPERS_START_LIFTING;
    let lift = math::clamp_or_low((position - SUSTAIN_DAMPERS_START_LIFTING) / span, 0.0, 1.0);
    1.0 - lift * lift * (3.0 - 2.0 * lift)
}

impl Engine {
    /// Releases `midi`'s voice — a MIDI note-off or a computer-keyboard
    /// key-up. Its damper then rests with whatever pressure the pedals
    /// leave it: none while the sustain pedal is down or the sostenuto
    /// caught this key, full otherwise.
    pub(crate) fn note_off(&mut self, midi: u8) {
        let pressure = damper_pressure_for_sustain_position(self.pedals.sustain_position);
        let Some(voice) = self.voice_for_midi(midi) else {
            return;
        };
        voice.held = false;
        settle_damper(voice, pressure);
    }

    /// The two-position sustain pedal: fully down or fully up.
    pub(crate) fn set_sustain_pedal(&mut self, down: bool) {
        self.set_sustain_pedal_position(if down { 1.0 } else { 0.0 });
    }

    /// Moves the sustain pedal to `position` (`0` up, `1` down; `NaN`
    /// counts as up) and settles every key's damper accordingly. Bounded by
    /// the 88 voices.
    pub(crate) fn set_sustain_pedal_position(&mut self, position: f32) {
        self.pedals.sustain_position = math::clamp_or_low(position, 0.0, 1.0);
        self.settle_every_damper();
    }

    /// Presses or releases the sostenuto. Pressing catches exactly the
    /// keys held at that instant; releasing lets every caught key's damper
    /// fall back to what the sustain pedal and its finger say. A press
    /// while already down catches nothing new, as on a real piano.
    pub(crate) fn set_sostenuto_pedal(&mut self, down: bool) {
        if down == self.pedals.sostenuto_down {
            return;
        }
        self.pedals.sostenuto_down = down;
        for voice in &mut self.voices {
            voice.sostenuto_latched = down && voice.held;
        }
        self.settle_every_damper();
    }

    /// Presses or releases the soft pedal; it shapes the *next* strikes
    /// only, as the shifted action does.
    pub(crate) fn set_soft_pedal(&mut self, down: bool) {
        self.pedals.soft_down = down;
    }

    fn settle_every_damper(&mut self) {
        let pressure = damper_pressure_for_sustain_position(self.pedals.sustain_position);
        for voice in &mut self.voices {
            settle_damper(voice, pressure);
        }
    }
}

/// Rests `voice`'s damper with `free_pressure` unless a finger or the
/// sostenuto is holding it off the string.
fn settle_damper(voice: &mut Voice, free_pressure: f32) {
    let pressure = if voice.held || voice.sostenuto_latched {
        0.0
    } else {
        free_pressure
    };
    if let Some(strings) = voice.strings.as_mut() {
        strings.set_damper_pressure(pressure);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_damper_rests_at_the_top_of_the_stroke_and_clears_at_the_bottom() {
        assert!((damper_pressure_for_sustain_position(0.0) - 1.0).abs() < 1e-6);
        assert!((damper_pressure_for_sustain_position(0.2) - 1.0).abs() < 1e-6);
        assert!(damper_pressure_for_sustain_position(0.7).abs() < 1e-6);
        assert!(damper_pressure_for_sustain_position(1.0).abs() < 1e-6);
        assert!((damper_pressure_for_sustain_position(f32::NAN) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn pressure_falls_monotonically_through_the_half_pedal_zone() {
        let mut previous = 1.0f32;
        for step in 0..=100u8 {
            let pressure = damper_pressure_for_sustain_position(f32::from(step) / 100.0);
            assert!(
                pressure <= previous + 1e-6,
                "{step}: {pressure} > {previous}"
            );
            previous = pressure;
        }
    }
}
