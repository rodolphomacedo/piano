# piano

A physically modelled piano synthesiser in Rust.

Not a sampler. There are no recordings of a piano anywhere in this repository —
the sound is computed from a model of what a vibrating string actually does. The
long-term target is the class of instrument that commercial modelled pianos
occupy; the short-term target is to get there one audible step at a time.

**Status: milestone M8, the last on the roadmap.** A struck string (a proper digital waveguide with
dispersion and a nonlinear felt-hammer excitation, M4) renders to a WAV
file, in tune to within about 1.5 cents, plays live through the speakers —
polyphonically, a full 88-key voice pool, from the computer keyboard or a
MIDI controller — and also runs in a browser tab, compiled to WebAssembly
and driven from an `AudioWorklet`, no install required. MIDI input plays
chords, releases notes early (note-off), and honours the sustain pedal
(CC64); every voice's damping, sustain and inharmonicity has its own
per-register baseline rather than one setting for the whole keyboard, and
every parameter is still a live, adjustable control, not a build-time
constant. Most notes are struck by more than one physical string — 1 in
the bass, 2 in the tenor, 3 in the treble, the standard piano layout — and
those unison strings beat and settle into a measured two-stage decay (see
`crates/piano-render/tests/m6_spectral.rs`); holding the sustain pedal now
lets the rest of the instrument ring sympathetically through a shared
bridge bus (`PERF-008`), and every voice is coloured by a small
modal-synthesis soundboard (`PERF-009`) rather than heard as a raw string.
M7 added no audible feature: it is a dedicated performance-engineering
pass — a `criterion` benchmark harness per DSP component, a genuine
repeated-sampling p99.9 callback-timing histogram, and real measured
numbers (not opinions) closing or advancing every open entry in
[`docs/PERFORMANCE.md`](docs/PERFORMANCE.md)'s bottleneck register. M8, like
M7, adds no audible feature either: it is release and packaging — a
`workflow_dispatch` GitHub Actions build for macOS Intel, macOS Apple
Silicon and Linux (`.github/workflows/release-build.yml`), every crate
verified publish-ready against the real crates.io registry via
`cargo publish --dry-run`, and a documented (not yet built — see
[`docs/PLUGIN-PATH.md`](docs/PLUGIN-PATH.md)) path to a CLAP plugin. See the
[roadmap](docs/ROADMAP.md) for the full milestone history and M8's own honest
account of what is and is not published yet.

## Install

There are two honest states here, and this section says which one is real
today rather than describing a future that has not happened yet.

**Today: build from source.** No crate in this workspace is published to
crates.io yet — that is a deliberate choice left for this project's owner to
make, not an oversight (see `docs/ROADMAP.md`'s M8 entry for exactly why).
Assuming nothing but [`rustup`](https://rustup.rs):

```sh
git clone https://github.com/rodolphomacedo/piano.git
cd piano
cargo run --release -p piano-cli -- keyboard
```

That builds every crate the CLI depends on and starts playing from your
computer keyboard — no separate install step, no admin privileges beyond
what `cargo` itself needs. The `render`/`play`/`midi` subcommands below all
work the same way, substituting the subcommand after `--`.

**A prebuilt binary, no compiler needed.** Each milestone's-worth of source
can also be built once and downloaded, rather than compiled locally: this
repository's `.github/workflows/release-build.yml` (a manual, human-triggered
GitHub Actions workflow, not something that runs automatically) builds
`piano-cli` for macOS Intel, macOS Apple Silicon and Linux and uploads each
as a **workflow artifact** — visible and downloadable from this repository's
[Actions tab](https://github.com/rodolphomacedo/piano/actions/workflows/release-build.yml)
to anyone who can see the repo, for 14 days after the run, with no publish
step and no public Release involved. Whether a build exists at any given
moment depends on whether someone has triggered that workflow recently — if
the Actions tab shows no recent successful run, building from source above
is the only path today.

**After a human decides to publish (not yet true): `cargo install`.** Every
crate has already been checked with `cargo publish --dry-run` against the
real crates.io registry (`docs/ROADMAP.md`'s M8 entry has the full,
crate-by-crate result) and is publish-ready. Once the project's owner
actually runs `cargo publish` — a deliberate, irreversible action this
milestone intentionally did not take on their behalf — installing becomes:

```sh
cargo install piano-cli   # not live yet — see above
```

This line is written in advance so the day it becomes true, nothing else
in this README needs to change.

## Try it

Render a note to a file:

```sh
cargo run --release -p piano-cli -- render --note A4 --seconds 3 --output a4.wav
```

```
piano render [OPTIONS]

  -n, --note <NOTE>            Note name (A4, C#3) or MIDI number (69)  [default: A4]
  -s, --seconds <SECONDS>      Length of the render                     [default: 3]
  -v, --velocity <VELOCITY>    Strike strength, 0.0 to 1.0              [default: 0.8]
  -r, --sample-rate <HZ>       Output sample rate                       [default: 48000]
      --concert-a <HZ>         Reference pitch for A4                   [default: 440]
  -o, --output <FILE>          Where to write the WAV                   [default: note.wav]
```

Play one note live, no file involved — `--damping`/`--sustain` override the
built-in voicing for that note:

```sh
cargo run --release -p piano-cli -- play --note A4 --seconds 3 --damping 0.7
```

Play live from the computer keyboard — nothing is written to disk:

```sh
cargo run --release -p piano-cli -- keyboard
```

The bottom row (`Z S X D C V G B H N J M ,`) is one octave; the top row
(`Q 2 W 3 E R 5 T 6 Y 7 U I 9 O 0 P`) continues upward. `[`/`]` and `-`/`=`
change damping and sustain live, audible on notes already ringing. Esc or
Ctrl+C to quit. Holding several keys plays a chord (one voice per key, no
stealing needed — see `PERF-012`). **Key-release is terminal-dependent**:
most terminals, including macOS's default Terminal.app, never report a
key-up event at all, so notes ring out on their own; a terminal that
implements the Kitty keyboard protocol (kitty, WezTerm and similar) is
detected automatically and gets real early release. `piano midi` below
always has real note-off regardless of terminal.

Play from a MIDI controller — a digital piano over USB or a MIDI cable:

```sh
cargo run --release -p piano-cli -- midi --list   # see what is plugged in
cargo run --release -p piano-cli -- midi
```

Notes play as struck and release on note-off; CC74 (brightness, if your
controller has one) drives damping and CC1 (mod wheel) drives sustain, both
live and both distinct from CC64, the sustain (hold) *pedal* — holding it
keeps every note ringing past its own note-off, same as a real piano's
right pedal. Play several notes together for a chord. Esc or Ctrl+C in the
terminal to quit — the instrument itself has no on/off switch to send back.

As of M6, holding the sustain pedal (CC64, or `[`/`]`-adjacent keys are
unaffected — the pedal is a separate control from damping/sustain) and
playing a note makes the rest of the instrument audibly ring along with
it, through the shared bridge bus (`PERF-008`); most notes also now have
their own beating, from the 2-3 real physical strings each key is struck
by. See `docs/pt-BR/M6-como-usar.md` for a literal step-by-step of what to
listen for (Portuguese; an English pass may follow).

Play in a browser tab — no install, no toolchain beyond `rustup` and a
one-time `wasm-bindgen-cli` install:

```sh
rustup target add wasm32-unknown-unknown   # once
cargo install wasm-bindgen-cli --version 0.2.100 --locked   # once; must match the wasm-bindgen version in Cargo.toml
cargo build -p piano-wasm --release --target wasm32-unknown-unknown
wasm-bindgen --target web --out-dir crates/piano-wasm/www/pkg target/wasm32-unknown-unknown/release/piano_wasm.wasm
cd crates/piano-wasm/www && python3 -m http.server 8080
```

Open `http://localhost:8080`, move the frequency slider, click Strike. A
plain static file server is enough — and required, since browsers refuse to
load ES modules or WASM over `file://`. `simd128` is on by default for this
target (`.cargo/config.toml`, `PERF-011`); there is no MIDI input and no
polyphony in the browser yet, only a single voice, matching what this
milestone (M3) asks for.

## Why it is built this way

Three constraints drive every structural decision, and they are worth stating
before the file listing:

**It has to be efficient.** A full piano is up to 240 simultaneously ringing
strings, and the audio callback has 2.67 ms to compute every 128 samples. This is
why the string model is a digital waveguide (`O(1)` per string per sample) rather
than a finite-difference grid, and why known bottlenecks are written down
[before the code that hits them](docs/PERFORMANCE.md).

**It must never lock up.** Not "rarely" — never. An audio callback that misses its
deadline produces an audible click; one that panics takes down the host. So the
DSP core allocates nothing while processing, locks nothing, panics nowhere, and
has no unbounded loops. These are [enforced structurally](docs/REALTIME-AUDIO-RULES.md),
not by discipline.

**The engine must not know where it is running.** Native, browser, plugin, test —
the physics is the same code. Everything platform-specific lives in a crate that
depends on the core, never the reverse.

## Layout

| Crate | What it is |
|---|---|
| `piano-core` | The DSP. `no_std` + `alloc`, `forbid(unsafe_code)`, zero allocation while processing |
| `piano-params` | Note names, MIDI numbers, tuning |
| `piano-render` | Offline rendering and WAV output |
| `piano-audio` | Realtime output via `cpal`, the lock-free command queue, denormal control |
| `piano-midi` | MIDI input via `midir`, decoded into the same command queue |
| `piano-wasm` | Browser bindings via `wasm-bindgen`, driven from an `AudioWorklet` |
| `piano-cli` | The `piano` binary |

## Documentation

| | |
|---|---|
| [Roadmap](docs/ROADMAP.md) | Milestones, each ending in something you can hear |
| [Architecture](docs/ARCHITECTURE.md) | Crate boundaries and why they are where they are |
| [Physics](docs/PHYSICS.md) | What each component corresponds to on a real string |
| [Real-time rules](docs/REALTIME-AUDIO-RULES.md) | What the audio thread may and may not do |
| [Performance](docs/PERFORMANCE.md) | The bottleneck register — known costs, before they bite |
| [Prior art](docs/PRIOR-ART.md) | The literature, and the licence rules we work under |
| [Plugin path](docs/PLUGIN-PATH.md) | Whether a CLAP/VST3/AU wrapper is worth building, and why not yet |
| [Decisions](docs/adr/) | Architecture decision records |

## Development

```sh
cargo test --workspace                                      # 164 tests (2 further ignored, wall-clock only)
cargo clippy --workspace --all-targets -- -D warnings       # must be clean
cargo fmt --all --check
cargo build -p piano-core --no-default-features             # no_std must keep building

# M7's benchmark harness — not part of `cargo test`, run by hand and quoted
# in docs/PERFORMANCE.md:
cargo bench -p piano-core                                    # per-component criterion benchmarks
cargo test --release -p piano-audio -- --ignored --nocapture # whole-engine and p99.9 timing numbers
```

See [CONTRIBUTING.md](CONTRIBUTING.md).

## Licence

MIT OR Apache-2.0, at your option.

This project does not copy from copyleft-licensed prior art. See
[docs/PRIOR-ART.md](docs/PRIOR-ART.md) for what that means in practice and how
other open piano models are used as benchmarks rather than sources.
