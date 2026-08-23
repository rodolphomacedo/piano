// The AudioWorklet side of piano-wasm's browser demo.
//
// This file runs in `AudioWorkletGlobalScope`, on the browser's dedicated
// audio-rendering thread — the WASM equivalent of the native `cpal` audio
// callback described in docs/REALTIME-AUDIO-RULES.md. `process()` is called
// once per 128-sample render quantum and must never allocate, block or
// throw; `PianoVoice::render` (see crates/piano-wasm/src/lib.rs) is written
// to exactly that contract.
//
// `AudioWorkletGlobalScope` has no `fetch`, so the wasm bytes are fetched on
// the main thread (index.html) and handed over through `processorOptions`.
// `initSync` — added to wasm-bindgen specifically to support this pattern —
// instantiates the module synchronously from those already-fetched bytes,
// with no network access needed here.
import { PianoVoice, wasmMemory, blockSize, initSync } from "./pkg/piano_wasm.js";

class PianoProcessor extends AudioWorkletProcessor {
  constructor(options) {
    super();

    const { wasmBytes } = options.processorOptions;
    initSync({ module: wasmBytes });

    // `sampleRate` is a global provided by AudioWorkletGlobalScope: the
    // audio device's real rate, not necessarily 48 kHz.
    this.voice = new PianoVoice(sampleRate);
    this.memory = wasmMemory();
    this.quantum = blockSize();

    // Runs on this same audio-rendering thread, but dispatched between
    // render quanta rather than nested inside one — never called from
    // `process()` below. `PianoVoice.strike` allocates (it builds a new
    // delay line for the new frequency), which is exactly why it must stay
    // out of the per-quantum path; see the crate docs for the reasoning.
    this.port.onmessage = (event) => {
      const { frequency, velocity } = event.data;
      try {
        this.voice.strike(frequency, velocity);
      } catch (error) {
        // A rejected strike (e.g. a frequency too high to represent) is not
        // a reason to kill the audio graph — log it and keep rendering
        // silence from whatever the voice was already doing.
        console.error("piano-wasm: strike rejected:", error);
      }
    };
  }

  process(_inputs, outputs) {
    this.voice.render();

    // Re-read `memory.buffer` on every quantum rather than caching the
    // view: `strike()` can grow the Wasm heap between calls, which detaches
    // any `ArrayBuffer`/`Float32Array` built over the previous memory
    // instance. Constructing a new typed-array view here is a lightweight
    // JS object, not a copy of the underlying bytes.
    const view = new Float32Array(this.memory.buffer, this.voice.outputPtr(), this.quantum);
    for (const channel of outputs[0]) {
      channel.set(view);
    }
    return true;
  }
}

registerProcessor("piano-processor", PianoProcessor);
