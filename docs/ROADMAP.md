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

## M2 — Real-time playback

Live audio output on macOS through `cpal`, driven by a lock-free command queue.
This is where the real-time rules stop being theory.

**Includes**: the SPSC ring buffer, the audio-thread contract, hardware denormal
control (`PERF-002`), and the first latency measurements.

**Done when**: `piano play --note A4` makes sound with no allocation, no locking
and no missed buffers over a ten-minute run.

---

## M3 — Web playground

The core compiled to WASM and driven from an `AudioWorklet`, with a page that has
a button and a frequency slider. Ugly is fine. The point is a fast feedback loop
for anyone, on any machine, with no toolchain.

**Includes**: `simd128` build flags (`PERF-011`) and a 128-sample block size
matching the worklet quantum.

**Done when**: clicking a button in a browser plays the same string model.

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

88 keys, real polyphony, MIDI input, and the voice management that makes it
survivable.

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
