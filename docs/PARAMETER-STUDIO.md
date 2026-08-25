# The live parameter studio

Every physically-modelled parameter in this project — per-string decay,
per-string brightness, the felt hammer's contact physics, the soundboard's
modal bank, the bridge's coupling gains — has, until now, lived as a Rust
constant or a `docs/ROADMAP.md`-cited interpolation table
(`piano-audio::voicing`). Changing any of it meant editing source and
recompiling. This document is the accepted design for closing that gap: a
new crate, `piano-studio`, that exposes **every** tunable parameter through
a local web page and a piano configuration file, live, while the instrument
is playing — including from a real MIDI keyboard at the same time — without
weakening any of `docs/REALTIME-AUDIO-RULES.md`'s guarantees.

Brainstormed and approved 2026-08-24. The two scope decisions made during
that conversation, stated up front because they shape everything below:

1. **Full scope, not phased.** Parameters that do not yet have a live
   setter anywhere — the hammer's contact exponent/stiffness/mass, the
   soundboard's 8 modes, the bridge's coupling gains — get one this round,
   not "later."
2. **Per-string is the atomic unit.** A "group" is a named, saved selection
   of individual strings plus a set of values to write into each of them —
   never a new kind of entity the engine has to know about.

## Goal

Play through a real MIDI keyboard (or the computer keyboard, or nothing —
just audition a held note) while dragging sliders for any string, any group
of strings, or the instrument as a whole, hear the change immediately, and
optionally save the result as a named `.piano.json` file that
`piano-cli` can load later.

## Non-goals

- **A full string-to-string bridge coupling matrix.** The bridge stays two
  scalar gains (`LOCAL_COUPLING_GAIN` for a key's own 2-3 strings,
  `GLOBAL_COUPLING_GAIN` for cross-key sympathetic resonance), now
  live-settable instead of `const`, but still not a ~230×230 matrix. This is
  not a new restriction — it is the existing, already-documented `PERF-008`
  decision (`docs/ARCHITECTURE.md`: "not all-to-all coupling") carried
  forward, called out explicitly here because "tudo, realmente tudo" is
  this document's own design goal everywhere else.
- **Autosave.** Every edit is live-audible immediately but changes nothing
  on disk until an explicit save action. See "Persistence", below.
- **Remote or multi-user access.** The web server binds to `localhost`
  only; there is no authentication because there is nothing to
  authenticate against.
- **Undo/redo, diffing "what changed from the file."** Save writes the
  fully resolved state back out. See "Piano file format."

## Why this fits the existing architecture without a redesign

`docs/ARCHITECTURE.md` already states the intended shape of exactly this
feature, written before it existed: *"Every voice parameter — damping,
sustain, the excitation seed — has a live setter reachable through the same
command queue, so a controller, a CLI flag or (later) a Bayesian parameter
estimator all reach the engine through one path, not three."* The lock-free
SPSC command ring (`piano-audio::commands`, ADR-0005) that already carries
`NoteOn`/`NoteOff`/`SustainPedal`/`SetDamping`/`SetSustain` from a control
thread to the audio callback is that one path. This feature adds commands
to it; it does not add a second path.

Crate placement follows `docs/ARCHITECTURE.md`'s existing split: `piano-core`
never learns that a web server exists, the same way it never learned `cpal`
or `midir` exist. `piano-studio` sits where `piano-render` and `piano-cli`
already sit — the allocating, file-touching, non-real-time side of the
fence — and depends on `piano-audio` and `piano-params` exactly as
`piano-cli` does today.

```
piano-core → piano-params → piano-audio ─┐
                                          ├─→ piano-studio (new)
                            piano-midi  ──┘
```

## The three parameter tiers

**Per-string** (the atomic unit — up to 3 per key, up to 88 keys, ~230
total on a full instrument):

| Parameter | Exists today? |
|---|---|
| `damping` | Yes — `PluckedString::set_damping` |
| `sustain` | Yes — `PluckedString::set_sustain` |
| `inharmonicity` | Yes — `DispersionCascade::set_inharmonicity` |
| `detune_cents` | No — currently a fixed per-unison-position constant in `unison.rs` (`DETUNE_CENTS_BICHORD`/`TRICHORD`) |
| `seed` | Partially — set at construction (`StringConfig::seed`), no live setter |
| `hammer.contact_exponent` | No — `hammer::CONTACT_EXPONENT`, a module constant shared by all 230 strings |
| `hammer.stiffness` | No — `hammer::CONTACT_STIFFNESS`, same |
| `hammer.mass` | No — `hammer::HAMMER_MASS`, same |

**Per-instrument** (a single value shared by the whole piano, not per
string — because the physical thing they model is a single wooden board or
a single bridge, not a string):

| Parameter | Exists today? |
|---|---|
| `soundboard.modes[0..8]` (`frequency_hz`/`decay_seconds`/`gain` each) | No — `soundboard::MODES`, a fixed `const` array |
| `bridge.local_coupling_gain` | No — `unison::LOCAL_COUPLING_GAIN`, a module constant |
| `bridge.global_coupling_gain` | No — `unison::GLOBAL_COUPLING_GAIN`, a module constant |

**Groups**: a name plus a list of `{midi, string_index}` pairs plus a set of
values. Applying a group resolves to N individual per-string writes — the
engine, the command queue and the saved file never represent "a group" as
its own thing; only the studio's UI and file format do.

## Piano file format

A `.piano.json` resolves every one of the ~230 strings through a cascade,
most specific wins:

```
strings[]  (explicit, one entry per string)
    overrides   groups[].overrides   (matching group, in list order — later listed groups win ties)
        overrides   registers        (bass/mid/treble anchor interpolation — today's piano-audio::voicing, now data instead of Rust consts)
            falls back to   defaults
```

```json
{
  "name": "My Piano",
  "defaults": {
    "damping": 0.5,
    "sustain": 0.996,
    "inharmonicity": 0.0004,
    "hammer": { "contact_exponent": 2.5, "stiffness": 1.7e9, "mass": 1.0 }
  },
  "registers": {
    "bass":   { "anchor_midi": 21,  "decay_seconds": 35.0, "damping": 0.6, "inharmonicity": 0.0001 },
    "mid":    { "anchor_midi": 69,  "decay_seconds": 11.0 },
    "treble": { "anchor_midi": 108, "decay_seconds": 1.5,  "damping": 0.4, "inharmonicity": 0.05 }
  },
  "groups": [
    {
      "name": "darker bass overtones",
      "strings": [ { "midi": 30, "string_index": 0 }, { "midi": 32, "string_index": 1 } ],
      "overrides": { "damping": 0.7 }
    }
  ],
  "strings": [
    { "midi": 69, "string_index": 1, "detune_cents": 3.0, "seed": 12345 }
  ],
  "instrument": {
    "soundboard_modes": [
      { "frequency_hz": 80.0, "decay_seconds": 1.2, "gain": 1.0 }
    ],
    "bridge": { "local_coupling_gain": 0.15, "global_coupling_gain": 0.08 }
  }
}
```

Loading resolves the cascade once, at startup or on `POST /api/load`, into
the same flat per-key table `piano-audio::voicing::voicing_for_key` builds
today — the difference is the table is now data-driven, and
`registers` in the file *is* today's three-anchor interpolation, serialised
instead of hardcoded.

### Persistence

Every slider move is applied to the running engine immediately (audible
right away) but the in-memory table only, never the file. `POST /api/save`
resolves the current live state — every one of the ~230 strings plus the
instrument block — and writes it out in full. There is no attempt to infer
"what did the user actually change" versus what is still at its computed
default; writing the fully resolved table is simpler, always correct, and
avoids building a diffing feature nobody asked for.

## Command queue extensions (`piano-audio::commands::Command`)

New variants, additive — the existing global `SetDamping`/`SetSustain` (used
today by the computer-keyboard's `[`/`]`/`-`/`=` live controls) are
untouched:

```rust
SetStringDamping { midi: u8, string_index: u8, damping: f32 },
SetStringSustain { midi: u8, string_index: u8, sustain: f32 },
SetStringInharmonicity { midi: u8, string_index: u8, inharmonicity: f32 },
SetStringDetune { midi: u8, string_index: u8, cents: f32 },
SetStringSeed { midi: u8, string_index: u8, seed: u32 },
SetStringHammer { midi: u8, string_index: u8, contact_exponent: f32, stiffness: f32, mass: f32 },
SetSoundboardMode { index: u8, frequency_hz: f32, decay_seconds: f32, gain: f32 },
SetBridgeCoupling { local_gain: f32, global_gain: f32 },
```

All `Copy` plain data, same as every existing variant — ADR-0005's
constraint (nothing on this queue can be the last clone of anything that
gets `Drop`ped on the audio thread) is untouched by this feature. Applying
a "group" from the studio UI enqueues one command per string in the group,
not a new batch variant.

## What has to change inside `piano-core` — the honest part

Three things here do not have live setters today because the value they
control has never varied per string or per instant before. Each needs its
own careful pass, to the same standard as everything already in
`piano-core` — `Copy`, no allocation, total for `NaN`/`±∞`/zero, covered by
`proptest`, not just a unit test at one value (`REALTIME-AUDIO-RULES.md`,
the top-level `CLAUDE.md`'s hard rule 5):

- **`hammer::simulate_contact`** currently reads `CONTACT_EXPONENT`,
  `CONTACT_STIFFNESS`, `HAMMER_MASS` as module constants. It needs to take
  them as parameters (a small `Copy` `HammerConfig` struct, not three loose
  floats) instead, and `StringConfig` needs matching fields so each
  `PluckedString` carries its own hammer. This is the largest surgical
  change this feature makes to `piano-core` — every existing caller of
  `simulate_contact` needs updating, and the existing "contact duration is
  physically plausible" proptest needs to be re-checked across the *new*
  parameter ranges, not just the old fixed constants.
- **`soundboard::Soundboard`** needs to be checked at implementation time
  for whether it already copies `MODES` into an instance field at
  construction (in which case this is "add a `set_mode` setter") or reads
  the `const` directly inside `process` (in which case the struct's shape
  changes). Not yet confirmed either way — first task of the implementation
  phase that touches this file.
- **`unison::LOCAL_COUPLING_GAIN`/`GLOBAL_COUPLING_GAIN`** move from module
  constants to fields owned by `piano-audio::Engine` (where the bridge bus
  itself already lives), read by the per-sample coupling code instead of
  the constant.

## HTTP / WebSocket API

`piano-studio` embeds its own HTML/JS/CSS via `include_str!` (no bundler,
no separate static-file server — same "plain files" spirit as
`piano-wasm/www`, but the binary serves them itself since this is native,
not a browser sandbox):

| Route | Method | Purpose |
|---|---|---|
| `/` | GET | The control page |
| `/api/piano` | GET | Full resolved state (~230 strings + instrument), JSON |
| `/api/live` | WS | Bidirectional: client sends one parameter change at a time, server applies it via the command queue and broadcasts the new resolved value to every connected client, so a second tab or a tablet on the piano's music stand stays in sync |
| `/api/save` | POST | `{ path, as_new: bool }` — resolves and writes the current live state |
| `/api/load` | POST | `{ path }` — reloads a file into the running engine |

## Browser UI, sketched

- An 88-key strip, coloured by register, each key expandable to its 1-3
  strings.
- Per-string panel: sliders for damping, sustain, inharmonicity, detune,
  the three hammer parameters, plus the seed as a number field.
- An instrument-wide panel: the 8 soundboard modes (frequency/decay/gain
  each) and the two bridge gains.
- A groups panel: name a group, multi-select strings on the keyboard strip
  to add them, set values, apply.
- Save / Save As, and a file picker for Load.

## CLI integration

```sh
cargo run --release -p piano-cli -- studio --piano my-piano.json --midi
```

Starts the same `Engine`/`AudioSession` machinery `piano midi` already
starts — a MIDI keyboard keeps playing live through it exactly as before —
alongside the local web server, and prints the URL
(`http://localhost:7878` by default) rather than trying to launch a
browser itself.

### Implementation notes

Where the shipped code (`crates/piano-cli/src/studio.rs`,
`crates/piano-studio/www/`) settled differently than this section's
original sketch, or adds detail it left unstated:

- The port flag is `--web-port`, not `--port` — `--port` was already taken
  by `piano midi`'s MIDI-port-name substring filter, which `studio` also
  accepts.
- `--piano <file>` must name a file that already exists; `studio` loads and
  resolves it, it does not create one. The smallest valid file is `{}`
  (every field falls back to a documented default).
- The web server always starts, with or without `--midi` — a browser tab
  is a full controller on its own, not only a companion to a MIDI keyboard.
- `studio` needs a real interactive terminal, `--midi` or not: quitting on
  `Esc`/`Ctrl+C` goes through the same raw-mode terminal reader `piano
  midi` uses, so it must run attached to a terminal, not backgrounded.
- The browser UI does not yet support naming and saving a new group from a
  selection — the "groups panel" sketched under "Browser UI" above is
  simplified to applying an edit to an ad hoc multi-key "selection"
  (shift-click), which is not persisted as a named group. Turning a
  selection into a saved, reusable group still means hand-editing the
  file's `groups` array.

## Testing strategy

- `piano-studio`'s file parsing and cascade-resolution logic: ordinary
  `std` unit tests, same tier as `piano-render`/`piano-cli` today — nothing
  here runs on the audio thread.
- The new `Command` variants in `piano-audio`: extend the existing
  no-allocation test (`tests_no_allocation.rs`) and property tests the same
  way every prior command addition did.
- The three `piano-core` changes above each need their own `proptest`
  coverage across the *new* live-settable range, not just the values that
  used to be constants — a value that was safe as a hand-picked constant is
  not automatically safe as an arbitrary `f32` a slider can send.

## Suggested implementation phases

Left for the implementation plan to size properly, but roughly in
dependency order: (1) piano file format + cascade resolver + CLI
`--piano` loading, reusing today's `voicing.rs` logic as the `registers`
tier; (2) the three `piano-core`/`piano-audio` changes that create new live
setters (hammer, soundboard, bridge); (3) the new `Command` variants and
their queue plumbing; (4) the HTTP/WebSocket server; (5) the browser UI.
Each is independently testable before the next begins.
