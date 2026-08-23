# Roadmap

Milestones are **demonstrable states of the instrument**, not lists of tasks. Each
one ends with something you can hear or measure. The order is chosen so that the
project is audible from day one and stays audible.

Every milestone here maps to a GitHub milestone of the same name.

---

## M0 — Foundations ✅

Cargo workspace, lint gate, CI, and the documents that define how the project
works. No sound yet, but the rules that make the rest possible.

**Done when**: `cargo clippy --workspace --all-targets -- -D warnings` is clean and
CI runs on every push.

---

## M1 — First sound ✅

Karplus–Strong plucked string, rendered offline to a WAV file through a CLI.

**Done when**: `piano render --note A4 --seconds 2` produces an audible file whose
measured fundamental is within a few cents of 440 Hz.

*Achieved: 1.4 cents.*

---

## M2 — Real-time playback ✅

Live audio output on macOS through `cpal`, driven by a lock-free command queue.
This is where the real-time rules stopped being theory.

**Includes**: the SPSC ring buffer (`rtrb`), the audio-thread contract, hardware
denormal control (`PERF-002`), a callback timing histogram (p50–p99.9), and a
no-allocation proof test. `piano play --note A4` plays one note through the
default output device; `piano keyboard` plays live from the computer keyboard;
`piano midi` plays live from a hardware MIDI controller via `midir`, nothing
written to disk in any case. Damping, sustain and the excitation seed are live
setters reachable from any of the three (`--damping`/`--sustain` flags,
`[`/`]`/`-`/`=` on the keyboard, CC74/CC1 over MIDI), not build-time constants
— the instrument is fully parametrised, ready for both manual voicing and a
future automated one (see "Ideas beyond M8" below).

**Done when**: `piano play --note A4` makes sound with no allocation, no locking
and no missed buffers over a ten-minute run.

*Achieved, with one open item*: the audio path is proven allocation-free by a
custom-allocator test (`piano-audio/src/tests_no_allocation.rs`, `PERF-012`
closed), and callback timing is measured and reported. Not yet done: a
ten-minute unattended soak run, and full device-hot-swap recovery (issue #20
covers panic-free stream error reporting; retuning voices after a live
sample-rate change is deferred).

---

## M3 — Web playground ✅

The core compiled to WASM and driven from an `AudioWorklet`, with a page that has
a button and a frequency slider. Ugly is fine. The point is a fast feedback loop
for anyone, on any machine, with no toolchain.

**Includes**: `simd128` build flags (`PERF-011`) and a 128-sample block size
matching the worklet quantum.

**Done when**: clicking a button in a browser plays the same string model.

*Achieved, with one open item*: `piano-wasm` wraps a single `PluckedString` in
a `wasm-bindgen` `PianoVoice`, built and reviewed to the same no-allocation
standard as `piano-audio::Engine` (`PianoVoice::render` is the per-quantum
entry point; `PianoVoice::strike`, the one allocating call, is documented to
run only between quanta — see `PERF-014`). The plain HTML/JS page and
`AudioWorkletProcessor` in `crates/piano-wasm/www/` were written against, and
verified against, the actual `wasm-bindgen 0.2.100 --target web` glue —
generating that glue and reading it caught a real bug before it could reach
a browser (`initSync` is a named export, not the default one, in this
`wasm-bindgen` version; the worklet's import statement had it backwards).
`cargo build -p piano-wasm --release --target wasm32-unknown-unknown`
succeeds with `simd128` enabled, and `wasm-bindgen` turns that binary into
loadable JS/Wasm without errors. **Not yet done**: no human has opened a
browser tab and clicked the button — the one verification this milestone's
own "done when" line names cannot be claimed from a terminal, and was not
available in the environment this milestone was built in. Whoever confirms
that should also delete this sentence.

---

## M4 — Real piano physics

The milestone that turns a plucked string into a struck one. The largest
single jump in sound quality in the whole roadmap.

**Includes**: a proper digital waveguide, a frequency-dependent loss filter, an
allpass dispersion cascade for inharmonicity (`PERF-005`), a better fractional
delay (`PERF-004`), and a nonlinear felt hammer model (`PERF-007`).

**Done when**: a struck note is audibly a piano rather than a guitar, its partials
are measurably sharp in the way a real piano's are, and hitting harder makes it
brighter rather than merely louder.

---

## M5 — The whole keyboard

88 keys, real polyphony, and the voice management that makes it survivable.
Basic MIDI note input already landed in M2 (`piano midi`, one permanent voice
per key means no stealing is needed yet); what M5 still owes is the sustain
pedal and a release/damping model — the current instrument has no way to stop
a note early, MIDI or otherwise.

**Includes**: a pre-allocated voice pool (`PERF-012`), energy-gated voice skipping
and voice stealing (`PERF-006`), block-based processing (`PERF-003`, `PERF-010`),
and per-key parameter tables.

**Done when**: a MIDI keyboard plays chords, the sustain pedal works, and the
callback holds its deadline at the documented voice count.

---

## M6 — Instrument realism

The things that separate "a piano model" from "an instrument": unison string
groups and their beating, sympathetic resonance through a shared bridge bus
(`PERF-008`), and a soundboard (`PERF-009`).

**Done when**: holding the sustain pedal and playing a note makes the rest of the
instrument ring, and a note has the two-stage decay real pianos have.

---

## M7 — Performance engineering

A dedicated pass with the benchmark harness in place: profile, measure, and close
`PERF` register entries with numbers.

**Includes**: criterion benchmarks per component, a callback-timing harness that
reports p99.9 rather than mean, SIMD where measurement justifies it, and
resolution of `PERF-001`, `PERF-010` and `PERF-013`.

**Done when**: every open `PERF` entry has either a measurement showing it does not
matter or a fix with a before/after number.

---

## M8 — Release and packaging

Reproducible builds for macOS Intel, macOS Apple Silicon and Linux; published
crates; a documented plugin path if it is worth taking.

**Done when**: someone who is not the author can install it and play it.

---

## Backlog — future optimisations

Items that are real but not scheduled. The `PERF` register in
`docs/PERFORMANCE.md` is the authoritative list; anything there without a
milestone lives here.

---

## Ideas beyond M8

Not scheduled, not sized, and not started — recorded so the reasoning behind
them is not lost between conversations.

**Bayesian parameter estimation from a recorded instrument.** Every parameter
this model exposes (damping, sustain, dispersion once M4 lands, hammer
hardness) is a live, adjustable control as of M2 — the prerequisite for
fitting them automatically rather than only by ear. The idea: record a real
instrument's note, extract features (partial decay rates, inharmonicity
coefficients, attack shape), and use Bayesian inference (Stan, or a
Rust-native sampler such as `nuts-rs`/`bridgestan` bindings to avoid a
non-Rust runtime dependency) to fit the physical model's parameters against
that recording — a calibration problem, not a real-time one, so none of the
audio-thread rules apply to it. Plausible in principle for a Karplus-Strong
model's handful of scalar parameters; substantially harder once M4's full
waveguide and M6's coupled strings add dozens of interacting ones. The
recordings this needs are training data for an *offline* fitting tool, never
compiled into the instrument itself, so they cannot live in this repository —
its "no samples, ever" rule (see the project's `CLAUDE.md`) is about what
ships, not about what a separate calibration tool may read from disk. This
belongs in its own repository that depends on `piano-core` for the model to
fit, not the other way around.
