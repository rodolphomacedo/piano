# Changelog

All notable changes to this project are documented here, newest first. The
version applies to the whole workspace — every `piano-*` crate is versioned
together (see `[workspace.package]` in `Cargo.toml`), not independently.

Format loosely follows [Keep a Changelog](https://keepachangelog.com/), with
one addition: every entry names the doc where the full write-up lives, since
this project keeps its reasoning in `docs/`, not in this file.

## v0.2.1

**Fixed**

- A note's own unison strings (2-3 detuned copies) could collapse to
  silence within about 120 ms across roughly G5-B5 — worst at A5 — instead
  of decaying normally. Root cause: exactly one dispersion allpass section
  is active in that band, and high inharmonicity there clamped its
  coefficient close enough to the unit circle that a few cents of unison
  detuning spun the strings out of phase with each other, which the local
  unison blend then fed back as a compounding, self-cancelling loss. Fixed
  by lowering `dispersion::MAX_COEFFICIENT`. See `docs/TIMBRE-PLAN.md`, D10.
- `make run-studio` now ships a real `meu-piano.piano.json` example instead
  of an empty stub.

## v0.2.0

**Fixed**

- The soundboard's resonant modes had two compounding defects that produced
  the reported "metallic knocking" in the mid register: `Resonator::new`'s
  gain normalisation was wrong by up to 34 dB (bass-tilted), and the modes'
  quality factor (`Q` 302–1100) made them ring like a struck bar rather than
  a damped wooden board (`Q` 20–50). Both fixed; see `docs/TIMBRE-PLAN.md`,
  D8.
- `piano studio --midi` silently discarded every control change, including
  the sustain pedal — it carried its own copy of the MIDI event mapping that
  never handled `ControlChange`. Now calls the same mapping `piano midi`
  uses. See `docs/TIMBRE-PLAN.md`, D9.
- A `.piano.json` file's `registers` block was parsed and then silently
  ignored — editing `decay_seconds`, `damping` or `inharmonicity` under
  `registers` had no effect, with no error. Now wired into resolution. See
  `docs/TIMBRE-PLAN.md`, P1.
- `make run-studio` tried to `cargo run` inside `crates/piano-studio`, a
  library crate with no binary — fixed to run `piano-cli`'s `studio`
  subcommand instead, with a `PIANO=` variable to pick the file.
- The loop filter was solved against the fundamental's decay time alone,
  leaving every upper partial's decay an uncontrolled side effect (A0's 8th
  partial decayed only 1.18× faster than its fundamental — the spectral
  signature of an organ, not a piano). Now solved against three per-key
  decay targets at once. See `docs/TIMBRE-PLAN.md`, F1/D1/D2.

**Added**

- `Makefile`: a formatted `make help` menu covering every command this
  project's contributors run by hand — formatting, linting, tests, builds,
  running the instrument, docs, and cleanup.
- This changelog, and the versioning practice it exists to record: a
  meaningful unit of work gets a version bump and an entry here, not just a
  commit message.

## v0.1.0

Initial tagged state: all 8 planned milestones (`docs/ROADMAP.md`) done —
offline rendering, live keyboard and MIDI play with full 88-key polyphony,
and a WebAssembly build.
