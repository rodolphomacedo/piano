# Changelog

All notable changes to this project are documented here, newest first. The
version applies to the whole workspace — every `piano-*` crate is versioned
together (see `[workspace.package]` in `Cargo.toml`), not independently.

Format loosely follows [Keep a Changelog](https://keepachangelog.com/), with
one addition: every entry names the doc where the full write-up lives, since
this project keeps its reasoning in `docs/`, not in this file.

## Unreleased

The instrument was measured against a concert grand and rebuilt where the
measurement disagreed with it. Only numbers derived from that instrument are
in the repository; see `docs/CALIBRATION.md`.

**Changed**

- **The hammer is solved against the string every sample**
  (`piano_core::hammer`). A mass on a felt spring, `F = K·ξ^p`, acting on a
  waveguide junction at the strike point, in SI units. It replaces the
  shaped noise burst, its contact-force envelope and its lowpass: nothing is
  written into the delay line in advance, so a harder blow changes the shape
  of the excitation and a restruck string is struck while moving. Mass, felt
  stiffness, exponent and string impedance vary by register.
- **Decay rates follow the measured prompt sound**
  (`voicing_scale.rs`). Through the middle of the keyboard every partial
  falls about as fast as the fundamental, 15-25 dB/s; the earlier table kept
  the fundamental four times longer and killed the eighth partial seven
  times faster, which is a plucked string's decay.
- **Unisons** (`piano_core::unison`, `voicing_unison.rs`). The spread is set
  against each key's own decay rate (about 0.17 cents at A4) instead of a
  fixed number of cents, the strings load the bridge unequally, and the slow
  modes are spared 0.87 of the loss. Global coupling is 0.2.
- **Stretch tuning** (`voicing_stretch.rs`). Every key is tuned in beatless
  octaves from the scale's own inharmonicity: 15 cents flat at A0, 30 sharp
  at C8. Concert pitch is unchanged.
- **Regulation follows each register's trend**, not each key
  (`voicing_level.rs`, `tests/keyboard_loudness.rs`), and is applied after
  the phantom partials so that it no longer rescales them.
- **Una corda** misses the string that loads the bridge hardest.
- **The keybed knock is 11 dB lower** (`DEFAULT_THUMP_GAIN`): the radiation
  filter had lifted it to 14 dB under a *fortissimo* A4, against the 25 dB
  it is set for.

**Added**

- `piano_core::body`: the soundboard above its separate resonances, as a
  feedback delay network with lines a few milliseconds long. It is what
  turns a plucked string's attack into a piano's.
- `piano_core::radiation`: how the board's motion becomes sound — almost
  nothing below 60 Hz, two strong resonances near 85 and 128 Hz, a more
  efficient treble.
- `piano_core::svf`, `piano_core::fdn`: the filter sections and the delay
  network those two are built from; `room` now shares the network.

**Removed**

- `piano_core::excitation`, and with it the `.piano.json` fields `seed`,
  `excitation_noise_mix` and `hammer.felt_bandwidth`. There is no noise in
  the strike any more, so nothing for them to set.

**Known limits**

- Below a key velocity of 0.25 the top two octaves can lose their
  fundamental: a power-law felt has no stiffness left at so small a
  compression.
- A single key can sit several decibels off its neighbours, as the diffuse
  field favours or misses its partials.

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
- A master output gain (`Engine::master_gain`), the first volume control
  anywhere in the signal path. It is applied to the mixed, soundboard-
  coloured signal *before* `limiter::soft_limit`, so turning down reduces
  limiting instead of feeding an already-engaged limiter. Unity by default —
  the level every existing tuning/brightness/energy test was measured at —
  clamped to `[0, 4]` on the audio thread, and reachable through
  `AudioSession::set_master_gain` (live) and `.piano.json`'s
  `instrument.master_gain` (static), the same cascade `soundboard_mix_gain`
  uses. The companion velocity curve from issue #79 is left for a session
  that can listen. See `docs/TIMBRE-PLAN.md` F5, issue #79.
- Hammer **strike position** (`StringConfig::strike_position`), the first
  half of F2. A hammer strikes at one point — canonically ~1/8 of the
  string — and a string takes no energy in a mode with a node there, so the
  `1/strike_position` partial and its multiples are notched out; builders
  put that first notch on the dissonant 8th partial on purpose.
  `PluckedString::write_excitation` now sums the excitation with its own
  inverted reflection `strike_position · L` samples back
  (`DelayLine::apply_strike_comb`), so the notch falls out of the geometry —
  the mode-`n` amplitude comes out weighted by `2·|sin(π·n·strike_position)|`
  — rather than being an EQ fitted to imitate it. Set per key by
  `piano_audio::voicing` (1/8 in the bass drifting to 1/10 in the top
  treble), live-adjustable for the next strike via
  `PluckedString::set_strike_position`, and clamped so the comb reach can
  never outrun the delay line. It moves no tuning — only the attack's
  spectral shape — and turns A2's ragged attack profile into a smooth arch
  with a clear notch at H8. Putting `strike_position` on the file/live
  cascade is still #82. See `docs/PHYSICS.md` "Why the strike position
  notches out a partial", `docs/TIMBRE-PLAN.md` F2, issue #32.
- The **deterministic hammer force pulse** (`StringConfig::excitation_noise_mix`),
  the other half of F2 (#77). The delay line was filled with
  `rng.next_bipolar() * velocity * contact_force`, so every partial got a
  random amplitude and phase on every strike. The deterministic part of the
  excitation is now the contact force's first difference, `dF/dt` (the
  waveguide velocity-source form; Van Duyne & Smith 1995, Bank 2000), which
  is DC-free by construction, and the RNG is rewound to the string's seed
  before every strike, so a given velocity and seed renders bit-identical.
  `excitation_noise_mix` (new `StringConfig` field, live setter, clamped
  `[0, 1]`) keeps a broadband stochastic component: `1` is the old noise
  excitation bit-for-bit, `0` is purely deterministic. The default is
  `0.7` — the lowest even-tenth that clears every key of the all-88
  `engine_timbre` sweep, which binds low in the bass (at `0.6` F2's 8th
  partial was starved and its H1/H8 decay ratio ran past the sweep's
  ceiling; F2 and A5 are now both in the normal-gate representative slice so
  that regresses loudly). See `docs/PHYSICS.md` "Why the excitation is the
  hammer's force pulse", `docs/TIMBRE-PLAN.md` F2, issues #77 / #80.

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
