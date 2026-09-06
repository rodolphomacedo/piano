# Changelog

All notable changes to this project are documented here, newest first. The
version applies to the whole workspace — every `piano-*` crate is versioned
together (see `[workspace.package]` in `Cargo.toml`), not independently.

Format loosely follows [Keep a Changelog](https://keepachangelog.com/), with
one addition: every entry names the doc where the full write-up lives, since
this project keeps its reasoning in `docs/`, not in this file.

## v0.2.2

**Added**

- A full-`Engine` timbre regression net (`piano_audio::offline::OfflineEngine`
  plus `crates/piano-audio/tests/engine_timbre.rs`). Every timbre diagnostic
  before this rendered a bare string or a string + soundboard pair; none
  rendered the engine the player actually hears, which is how the A5 trichord
  collapse passed every test. The new path renders the real engine — unison
  coupling, bridge bus, soundboard mix, limiter — headless, and checks each
  key against its own solved decay intent: fundamental decay, per-partial
  decay ratio, radiated energy early and late, and trichord vs. monochord.
  The all-88 sweep is `#[ignore]`d for runtime; a representative slice runs
  in the normal gate. See `docs/MODEL-REVIEW.md`, N1/P1.
- `piano_audio::voicing::solved_decay_targets` — a key's three solved
  `(partial, seconds)` decay targets, so a test can measure against the same
  intent the loop-filter solve was given.

**Changed**

- The modelled soundboard went from 8 resonant modes topping out at
  1.4 kHz to 28, reaching 6.3 kHz. The table is now deliberately irregular
  in frequency spacing and `Q`, and its gain column is shaped like a
  soundboard's radiation efficiency — lowest modes held down, peak in the
  low mid-range, treble tail ~13 dB below the peak instead of ~34 — so the
  treble has a body instead of a bare wire. The board's own impulse
  response moves from a 189 Hz spectral centroid to ~406 Hz. Isolated
  cost re-benched: 5.10 µs per 128-sample block, still well under 1% of a
  full-instrument block. `SoundboardMode` gained a `bridge_coupling` field
  (carried for the future bridge/soundboard-load work, read nowhere yet).
  See `docs/TIMBRE-PLAN.md` F3, `docs/MODEL-REVIEW.md` claim 6.
- `SOUNDBOARD_MIX_GAIN`, a hardcoded `0.5` in `engine.rs`, is now
  `Engine::soundboard_mix_gain`: live-adjustable through
  `AudioSession::set_soundboard_mix_gain` and settable per instrument in
  `.piano.json` as `instrument.soundboard_mix_gain`.

**Known**

- F3's original gate had a second clause — "the mixed-in centroid moves by
  much more than 6%" — that was dropped rather than met. At the mix level
  the instrument uses, a physically honest fast-decaying body moves a
  broadband note's spectral centroid only 2–3%; a spectral centroid is a
  poor detector of that kind of contribution (the same point D3's own
  correction note makes). It is gated on radiated-energy change instead.
- The top octave (above ~C7) still gets little soundboard body — the
  highest mode is 6.3 kHz. Widening the bank further is left for a later
  pass.

- The all-88 sweep's first run showed the D10 region is not fully clear:
  across the upper treble a trichord radiates less sustained energy than its
  own monochord, worst at A5. It is inside the committed regression bands but
  the closest any key comes to failing. Tracked in #92; the fix belongs to
  the bridge/soundboard-coupling milestone, not to a second dispersion tweak.
  See `docs/TIMBRE-PLAN.md`, D10.

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
