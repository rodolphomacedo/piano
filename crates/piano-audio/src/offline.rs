//! Headless offline rendering through the real [`crate::engine::Engine`].
//!
//! Every timbre diagnostic this project had before issue #87 rendered a bare
//! [`piano_core::PluckedString`], or a `PluckedString` + [`piano_core::Soundboard`]
//! pair. **None rendered the [`Engine`](crate::engine::Engine)** — the only
//! path a player ever hears, and the one unison coupling, the shared bridge
//! bus, the soundboard mix and the output limiter all live on. That is
//! exactly how the A5 trichord collapse (`docs/TIMBRE-PLAN.md` D10) passed
//! every test: the string measured fine in isolation and the collapse only
//! existed once three detuned strings were blended through the engine.
//!
//! [`OfflineEngine`] is that missing path, minus the `cpal` stream and the
//! lock-free command ring — commands are applied straight to the engine
//! because there is no audio thread to protect. It is **not** for realtime
//! use: [`OfflineEngine::render`] allocates its output buffer, and the whole
//! point is to run slower than realtime with a measurement attached
//! (`crates/piano-audio/tests/engine_timbre.rs`).

use piano_core::SampleRate;
use piano_params::Tuning;

use crate::Command;
use crate::engine::Engine;

/// How many samples [`OfflineEngine::render_into`] hands
/// [`Engine::process_block`] at a time. Larger than the engine's own
/// 128-sample bridge block (`process_block` re-chunks internally anyway) so
/// an offline render of tens of seconds is not a million tiny calls; small
/// enough to stay a modest stack buffer's worth if a caller ever wants one.
const RENDER_BLOCK: usize = 1_024;

/// Longest single [`OfflineEngine::render`] call, in seconds — a bound so a
/// typo in a test cannot ask for an hour of 88-voice synthesis. `render`
/// clamps to this rather than erroring; a measurement harness has no error
/// path worth the ceremony.
const MAX_RENDER_SECONDS: f32 = 120.0;

/// A full [`Engine`] wired for offline measurement: strike notes, then
/// [`render`](OfflineEngine::render) the mixed, soundboard-coloured,
/// limiter-capped output the same way [`crate::AudioSession`] would produce
/// it live.
pub struct OfflineEngine {
    engine: Engine,
    sample_rate: SampleRate,
}

impl OfflineEngine {
    /// A production-voiced engine: every key gets
    /// [`voicing::unison_count_for_key`](crate::voicing::unison_count_for_key)
    /// strings, exactly as [`crate::AudioSession`] builds it.
    #[must_use]
    pub fn new(sample_rate: SampleRate, tuning: Tuning) -> Self {
        Self {
            engine: Engine::new(sample_rate, tuning),
            sample_rate,
        }
    }

    /// [`OfflineEngine::new`], but every voice is forced to `unison_strings`
    /// strings (clamped to 1-3). `Some(1)` renders every key as a monochord;
    /// pairing that with the natural [`OfflineEngine::new`] rendering is the
    /// monochord-vs-trichord control issue #87 asks for — a key whose
    /// trichord decays very differently from its own monochord is the
    /// `docs/TIMBRE-PLAN.md` D10 collapse signature.
    #[must_use]
    pub fn with_unison_strings(
        sample_rate: SampleRate,
        tuning: Tuning,
        unison_strings: usize,
    ) -> Self {
        Self {
            engine: Engine::with_unison_override(sample_rate, tuning, Some(unison_strings)),
            sample_rate,
        }
    }

    /// The sample rate every render runs at.
    #[must_use]
    pub fn sample_rate(&self) -> SampleRate {
        self.sample_rate
    }

    /// Strikes `midi` at `velocity` (`[0, 1]`), immediately — no queue.
    pub fn note_on(&mut self, midi: u8, velocity: f32) {
        self.engine.note_on(midi, velocity);
    }

    /// Releases `midi` — engages its damper (or defers to the sustain pedal
    /// coming up, exactly as the live engine does).
    pub fn note_off(&mut self, midi: u8) {
        self.engine.note_off(midi);
    }

    /// Sets the CC64 sustain-pedal hold state.
    pub fn set_sustain_pedal(&mut self, down: bool) {
        self.engine.set_sustain_pedal(down);
    }

    /// Sets the sustain pedal's continuous position, `0` (up) to `1`.
    pub fn set_sustain_pedal_position(&mut self, position: f32) {
        self.engine.set_sustain_pedal_position(position);
    }

    /// Sets the sostenuto (middle) pedal.
    pub fn set_sostenuto_pedal(&mut self, down: bool) {
        self.engine.set_sostenuto_pedal(down);
    }

    /// Sets the room's wet level. Offline renders start dry (`0`), so
    /// measurements see the instrument alone.
    pub fn set_room_mix(&mut self, mix: f32) {
        self.engine.set_room_mix(mix);
    }

    /// Sets the treble's duplex-segment gain; `0` mutes them.
    pub fn set_duplex_gain(&mut self, gain: f32) {
        self.engine.set_duplex_gain(gain);
    }

    /// Scales the room: how far apart its first reflections are.
    pub fn set_room_size(&mut self, size: f32) {
        self.engine.apply(Command::SetRoomSize { size });
    }

    /// Sets how long the room rings in the bass.
    pub fn set_room_reverb_seconds(&mut self, seconds: f32) {
        self.engine.apply(Command::SetRoomReverbSeconds { seconds });
    }

    /// Sets how long the room rings at the top of the spectrum.
    pub fn set_room_treble_reverb_seconds(&mut self, seconds: f32) {
        self.engine
            .apply(Command::SetRoomTrebleReverbSeconds { seconds });
    }

    /// Sets the silence before the room answers.
    pub fn set_room_predelay(&mut self, milliseconds: f32) {
        self.engine.apply(Command::SetRoomPredelay { milliseconds });
    }

    /// Sets how strongly the soundboard loads the strings.
    pub fn set_board_load_gain(&mut self, gain: f32) {
        self.engine.apply(Command::SetBoardLoadGain { gain });
    }

    /// Sets where the output limiter starts compressing.
    pub fn set_limiter_threshold(&mut self, threshold: f32) {
        self.engine
            .apply(Command::SetLimiterThreshold { threshold });
    }

    /// Sets how hard every damper grips its string.
    pub fn set_damper_strength(&mut self, strength: f32) {
        self.engine.set_damper_strength(strength);
    }

    /// Sets the bass's phantom-partial gain; `0` turns phantoms off.
    pub fn set_phantom_gain(&mut self, gain: f32) {
        self.engine.set_phantom_gain(gain);
    }

    /// Sets the keybed thump's level; `0` silences the action noise.
    pub fn set_action_noise_gain(&mut self, gain: f32) {
        self.engine.set_action_noise_gain(gain);
    }

    /// Sets the soft (una corda) pedal.
    pub fn set_soft_pedal(&mut self, down: bool) {
        self.engine.set_soft_pedal(down);
    }

    /// Renders `seconds` (clamped to `(0, MAX_RENDER_SECONDS]`, and to `0`
    /// for a non-finite request) into a freshly allocated mono buffer.
    #[must_use]
    pub fn render(&mut self, seconds: f32) -> Vec<f32> {
        let count = frames_for(seconds, self.sample_rate.hertz());
        let mut output = vec![0.0; count];
        self.render_into(&mut output);
        output
    }

    /// Renders `seconds` (clamped like [`OfflineEngine::render`]) into a
    /// freshly allocated stereo pair, `(left, right)`, heard from the
    /// player's bench: bass to the left, treble to the right. The mean of
    /// the two is exactly what [`OfflineEngine::render`] returns.
    #[must_use]
    pub fn render_stereo(&mut self, seconds: f32) -> (Vec<f32>, Vec<f32>) {
        let count = frames_for(seconds, self.sample_rate.hertz());
        let mut left = vec![0.0; count];
        let mut right = vec![0.0; count];
        for (left, right) in left
            .chunks_mut(RENDER_BLOCK)
            .zip(right.chunks_mut(RENDER_BLOCK))
        {
            self.engine.process_block_stereo(left, right);
        }
        (left, right)
    }

    /// Renders `output.len()` samples into `output`, in
    /// [`RENDER_BLOCK`]-sized pieces. Allocation-free, so a caller measuring
    /// many keys can reuse one buffer.
    pub fn render_into(&mut self, output: &mut [f32]) {
        for block in output.chunks_mut(RENDER_BLOCK) {
            self.engine.process_block(block);
        }
    }
}

/// Frames in `seconds`, clamped to `(0, MAX_RENDER_SECONDS]` and to `0`
/// for a non-finite request.
fn frames_for(seconds: f32, sample_rate_hz: f32) -> usize {
    let seconds = if seconds.is_finite() {
        seconds.clamp(0.0, MAX_RENDER_SECONDS)
    } else {
        0.0
    };
    (seconds * sample_rate_hz) as usize
}

impl core::fmt::Debug for OfflineEngine {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("OfflineEngine")
            .field("sample_rate", &self.sample_rate)
            .finish_non_exhaustive()
    }
}
