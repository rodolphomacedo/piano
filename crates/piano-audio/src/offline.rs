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

    /// Renders `seconds` (clamped to `(0, MAX_RENDER_SECONDS]`, and to `0`
    /// for a non-finite request) into a freshly allocated mono buffer.
    #[must_use]
    pub fn render(&mut self, seconds: f32) -> Vec<f32> {
        let seconds = if seconds.is_finite() {
            seconds.clamp(0.0, MAX_RENDER_SECONDS)
        } else {
            0.0
        };
        let count = (seconds * self.sample_rate.hertz()) as usize;
        let mut output = vec![0.0; count];
        self.render_into(&mut output);
        output
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

impl core::fmt::Debug for OfflineEngine {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("OfflineEngine")
            .field("sample_rate", &self.sample_rate)
            .finish_non_exhaustive()
    }
}
