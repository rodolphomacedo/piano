//! Browser bindings for [`piano_core`]: a single plucked-string voice driven
//! from an `AudioWorklet`.
//!
//! # What this crate is, and is not
//!
//! This is a thin `wasm-bindgen` wrapper, the same shape as `piano-audio`'s
//! wrap of `cpal` and `piano-midi`'s wrap of `midir` (see
//! `docs/ARCHITECTURE.md`). It adds no physics: [`PianoVoice`] owns exactly
//! one [`PluckedString`], built the same way the native engine builds each of
//! its 88 permanent voices. Polyphony, MIDI-in-the-browser and note names are
//! explicitly out of scope for milestone M3 — see `docs/ROADMAP.md`.
//!
//! # The realtime contract, translated to a browser
//!
//! `AudioWorkletProcessor.process()` is this crate's equivalent of the audio
//! callback described in `docs/REALTIME-AUDIO-RULES.md`: it runs on the
//! browser's dedicated audio rendering thread, once per 128-sample render
//! quantum, and has no more slack to miss a deadline than a native callback
//! does. [`PianoVoice::render`] is written to the same standard as
//! `piano-audio::Engine::process_block`: it allocates nothing, because
//! [`PianoVoice::new`] is the one call that allocates (building the voice's
//! delay line), and [`PianoVoice::strike`] — which also allocates, to retune
//! the voice to a new frequency — is meant to be called only from the
//! worklet's message handler, which runs on the same thread as `process()`
//! but *between* render quanta, never nested inside one. See the crate-level
//! honesty note in `docs/PERFORMANCE.md` (`PERF-014`) for what still crosses
//! the JS↔Wasm boundary on every call and why that is accepted for M3.
//!
//! # Zero-copy output
//!
//! [`PianoVoice::render`] writes into a buffer owned by the struct itself,
//! sized once at construction. JS reads the result by viewing Wasm linear
//! memory directly at [`PianoVoice::output_ptr`] through [`wasm_memory`],
//! rather than by receiving a return value — a `&mut [f32]` return or
//! parameter would make `wasm-bindgen`'s glue copy the buffer on every call,
//! which is exactly the per-quantum allocation this design avoids.

#![forbid(unsafe_code)]

use piano_core::string::StringConfig;
use piano_core::{Hz, ParamError, PluckedString, SampleRate};
use wasm_bindgen::prelude::*;

/// Samples rendered per call to [`PianoVoice::render`].
///
/// Fixed at 128 to match the `AudioWorklet` render quantum exactly, per
/// `PERF-011` — the browser never calls `process()` with any other block
/// size, so there is no reason for this to be a runtime parameter.
const BLOCK_SIZE: usize = 128;

/// A single struck string, ready to be driven from an `AudioWorklet`.
///
/// Mirrors one voice of `piano-audio::Engine`, minus the 88-voice pool and
/// the command queue — M3 asks for one button and one slider, not
/// polyphony, so there is exactly one [`PluckedString`] here.
#[wasm_bindgen]
#[derive(Debug)]
pub struct PianoVoice {
    string: Option<PluckedString>,
    sample_rate: SampleRate,
    output: [f32; BLOCK_SIZE],
}

#[wasm_bindgen]
impl PianoVoice {
    /// Builds an unstruck voice tuned for `sample_rate_hz`.
    ///
    /// Allocates nothing yet — no [`PluckedString`] exists until the first
    /// [`PianoVoice::strike`] — but validates the sample rate up front so a
    /// bad value from the host's `AudioContext` fails loudly here rather
    /// than silently later.
    ///
    /// # Errors
    ///
    /// Throws (as a `JsError`) if `sample_rate_hz` is not finite and
    /// positive.
    #[wasm_bindgen(constructor)]
    pub fn new(sample_rate_hz: f32) -> Result<PianoVoice, JsError> {
        Self::try_new(sample_rate_hz).map_err(param_error_to_js)
    }

    /// The fallible core of [`PianoVoice::new`], kept separate so tests can
    /// exercise the rejection path with a plain [`ParamError`]. Constructing
    /// a `JsError` calls into a real JS engine internally, which is not
    /// available under `cargo test` on a native target — see
    /// `param_error_to_js`'s callers for the only place that conversion
    /// happens.
    fn try_new(sample_rate_hz: f32) -> Result<Self, ParamError> {
        let sample_rate = SampleRate::new(sample_rate_hz)?;
        Ok(Self {
            string: None,
            sample_rate,
            output: [0.0; BLOCK_SIZE],
        })
    }

    /// (Re)tunes the voice to `frequency_hz` and strikes it at `velocity`.
    ///
    /// **Allocates.** Building a [`PluckedString`] for a new frequency means
    /// sizing a new delay line — `piano-core`'s one allocating call, exactly
    /// as it is for every native voice at construction. Call this only from
    /// the `AudioWorklet`'s `port.onmessage` handler, never from inside
    /// `process()`/[`PianoVoice::render`]; seeing `docs/REALTIME-AUDIO-RULES.md`
    /// explains why the two run on the same thread but must never nest.
    ///
    /// `velocity` is clamped into `[0, 1]` by [`PluckedString::pluck`]; an
    /// out-of-range value is therefore never an error, only a clamp.
    ///
    /// # Errors
    ///
    /// Throws if `frequency_hz` is not finite and positive, or is too high
    /// to be represented at this voice's sample rate (see
    /// [`piano_core::ParamError::FrequencyOutOfRange`]).
    pub fn strike(&mut self, frequency_hz: f32, velocity: f32) -> Result<(), JsError> {
        self.try_strike(frequency_hz, velocity)
            .map_err(param_error_to_js)
    }

    /// The fallible core of [`PianoVoice::strike`]. See
    /// [`PianoVoice::try_new`] for why this stays separate from the
    /// `JsError`-returning wrapper.
    fn try_strike(&mut self, frequency_hz: f32, velocity: f32) -> Result<(), ParamError> {
        let frequency = Hz::new(frequency_hz)?;
        let config = StringConfig::new(frequency);
        let mut string = PluckedString::new(config, self.sample_rate)?;
        string.pluck(velocity);
        self.string = Some(string);
        Ok(())
    }

    /// Renders one render quantum into the voice's own buffer.
    ///
    /// Allocation-free: `output` was sized once, at construction, and never
    /// resized. This is the method the `AudioWorklet`'s `process()` calls
    /// once per quantum; nothing it calls transitively allocates, locks or
    /// panics. An unstruck or fully decayed voice fills the block with
    /// silence rather than doing string processing for nothing, the same
    /// energy-gating `piano-audio::Engine` already does per voice.
    pub fn render(&mut self) {
        self.output.fill(0.0);
        let Some(string) = self.string.as_mut() else {
            return;
        };
        if string.is_silent() {
            return;
        }
        string.process_block_add(&mut self.output);
    }

    /// Address of the voice's output buffer in Wasm linear memory.
    ///
    /// Combined with [`wasm_memory`] and [`block_size`], lets JS construct a
    /// `Float32Array` view directly over `output` and read the result of
    /// [`PianoVoice::render`] with no copy — see the crate-level docs.
    #[wasm_bindgen(js_name = outputPtr)]
    #[must_use]
    pub fn output_ptr(&self) -> *const f32 {
        self.output.as_ptr()
    }

    /// Whether the voice is unstruck or has decayed below audibility.
    #[wasm_bindgen(js_name = isSilent)]
    #[must_use]
    pub fn is_silent(&self) -> bool {
        self.string.as_ref().is_none_or(PluckedString::is_silent)
    }
}

/// Samples per render quantum — the JS side needs this to size its
/// `Float32Array` view over [`PianoVoice::output_ptr`]. A free function
/// because `wasm-bindgen` cannot export a plain `const`.
#[wasm_bindgen(js_name = blockSize)]
#[must_use]
pub fn block_size() -> usize {
    BLOCK_SIZE
}

/// The instance's `WebAssembly.Memory`, for viewing [`PianoVoice::output_ptr`]
/// without copying. Thin rename of [`wasm_bindgen::memory`] so the JS side
/// only imports names from this crate's own glue.
#[wasm_bindgen(js_name = wasmMemory)]
#[must_use]
pub fn wasm_memory() -> JsValue {
    wasm_bindgen::memory()
}

/// Converts a rejected construction parameter into a `JsError`, carrying the
/// offending value through [`ParamError`]'s `Display` message rather than
/// losing it at the FFI boundary.
fn param_error_to_js(error: ParamError) -> JsError {
    JsError::new(&error.to_string())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn a_freshly_built_voice_is_silent() {
        let voice = PianoVoice::new(48_000.0).expect("48 kHz is a valid sample rate");
        assert!(voice.is_silent());
    }

    #[test]
    fn rejects_a_non_positive_sample_rate() {
        // Goes through `try_new`, not the public `new`: constructing the
        // `JsError` that `new` would return calls into a JS engine that does
        // not exist under `cargo test` on a native target (see
        // `PianoVoice::try_new`'s doc comment).
        assert!(PianoVoice::try_new(0.0).is_err());
        assert!(PianoVoice::try_new(-48_000.0).is_err());
    }

    #[test]
    fn striking_produces_an_audible_block() {
        let mut voice = PianoVoice::new(48_000.0).expect("48 kHz is a valid sample rate");
        voice.strike(440.0, 1.0).expect("440 Hz strikes cleanly");
        assert!(!voice.is_silent());
        voice.render();
        assert!(voice.output.iter().any(|sample| sample.abs() > 1e-3));
    }

    #[test]
    fn rendering_an_unstruck_voice_stays_silent() {
        let mut voice = PianoVoice::new(48_000.0).expect("48 kHz is a valid sample rate");
        voice.render();
        assert!(voice.output.iter().all(|sample| *sample == 0.0));
    }

    #[test]
    fn re_striking_retunes_to_the_new_frequency() {
        let mut voice = PianoVoice::new(48_000.0).expect("48 kHz is a valid sample rate");
        voice.strike(220.0, 1.0).expect("220 Hz strikes cleanly");
        let low = voice.string.as_ref().expect("just struck").loop_delay();
        voice.strike(880.0, 1.0).expect("880 Hz strikes cleanly");
        let high = voice.string.as_ref().expect("just struck").loop_delay();
        assert!(
            high < low,
            "880 Hz loop {high} should be shorter than 220 Hz loop {low}"
        );
    }

    #[test]
    fn rejects_a_frequency_too_high_to_represent() {
        // Same reasoning as `rejects_a_non_positive_sample_rate`: exercises
        // `try_strike` so no `JsError` is constructed on a native target.
        let mut voice = PianoVoice::new(48_000.0).expect("48 kHz is a valid sample rate");
        assert!(voice.try_strike(30_000.0, 1.0).is_err());
    }

    #[test]
    fn block_size_matches_the_worklet_quantum() {
        assert_eq!(block_size(), 128);
    }

    #[test]
    fn output_ptr_points_at_a_full_block() {
        let voice = PianoVoice::new(48_000.0).expect("48 kHz is a valid sample rate");
        let ptr = voice.output_ptr();
        assert!(!ptr.is_null());
    }
}
