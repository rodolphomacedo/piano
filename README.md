# piano

A physically modelled piano synthesiser in Rust.

Development sponsored by [Grabatus Labs](https://grabatus.com).

Not a sampler. There are no recordings of a piano anywhere in this repository
— every sound is *computed*, sample by sample, from a simulation of what a
real steel string does when a felt hammer strikes it. See ["What is this,
really?"](#what-is-this-really) below for the plain-language version.

> **Lendo em português?** Existe um guia completo em português, escrito para
> quem nunca viu este projeto: [`docs/pt-BR/LEIA-ME.md`](docs/pt-BR/LEIA-ME.md).
> Cobre o que é o projeto, como instalar e como usar, sem nenhum jargão
> técnico necessário. O resto deste arquivo (e do código) está em inglês —
> essa é uma regra do próprio projeto (veja [`CLAUDE.md`](CLAUDE.md)) — mas
> você não precisa dele para tocar.

**Status: all 8 planned milestones are done.** It renders to a WAV file,
plays live from a computer keyboard or a MIDI controller with full 88-key
polyphony, and runs in a browser tab compiled to WebAssembly. See ["Is this
actually finished?"](#is-this-actually-finished) below, or the full history
in [`docs/ROADMAP.md`](docs/ROADMAP.md).

**Version: v0.2.1** — see [`CHANGELOG.md`](CHANGELOG.md) for what changed
last and why.

## Try it in one command

```sh
git clone https://github.com/rodolphomacedo/piano.git && cd piano
cargo run --release -p piano-cli -- keyboard
```

The first run compiles everything (a few minutes); after that it's instant.
Once it starts, your computer keyboard *is* the piano — see [Play from the
computer keyboard](#play-from-the-computer-keyboard) below for which keys do
what. That's the whole install: no separate build step, nothing beyond
[`rustup`](https://rustup.rs).

## What is this, really?

Most "digital pianos" — keyboards, plugins, phone apps — are **samplers**:
someone records a real piano playing every note at a few different
strengths, and the software plays those recordings back, pitch-shifted and
blended to fill the gaps. It works, but every sound that comes out was
captured in advance; there's no physics underneath, only audio files.

This project does the opposite. It has no audio files at all — instead, it
has a mathematical model of an actual piano string: how stiff it is, how it
loses energy over time, how the 2–3 strings behind a single key beat against
each other, how the soundboard colours what you hear. Every sample your
speakers play was calculated from that physics a moment earlier, 48,000
times a second. The payoff: any note, at any velocity, in any combination,
sounds "real" — not because someone recorded that exact combination, but
because it comes from the same underlying physics a real piano does.

## Is this actually finished?

Yes, in the sense that matters: **it works today**, on real hardware, and
every milestone that was planned for it is done, tested, and independently
verified — this isn't an unfinished prototype. What "finished" does *not*
mean here: it isn't a commercial-grade concert-piano replacement (see [Where
this can still go](#where-this-can-still-go)), and it isn't installable with
a single `cargo install` yet — see [Install](#install) for exactly why and
what that's waiting on.

## Install

There are two honest states here, and this section says which one is real
today.

**Today: build from source.** No crate in this workspace is published to
crates.io yet — that's a deliberate choice left for this project's owner to
make on purpose, not unfinished work (see `docs/ROADMAP.md`'s M8 entry for
the full reasoning). Assuming nothing but [`rustup`](https://rustup.rs):

```sh
git clone https://github.com/rodolphomacedo/piano.git
cd piano
cargo run --release -p piano-cli -- keyboard
```

That builds every crate the CLI depends on and starts playing — no separate
install step. The `render`/`play`/`midi` subcommands below all work the same
way, substituting the subcommand after `--`.

**A prebuilt binary, no compiler needed.** This repository's
`.github/workflows/release-build.yml` (triggered by hand, not on every push)
builds `piano-cli` for macOS Intel, macOS Apple Silicon and Linux and
uploads each as a **workflow artifact** — downloadable from this
repository's [Actions
tab](https://github.com/rodolphomacedo/piano/actions/workflows/release-build.yml)
for 14 days after the run, no publish step involved. If the Actions tab
shows no recent successful run, building from source above is the only path
today.

**After a human decides to publish (not yet true): `cargo install`.** Every
crate has already been checked with `cargo publish --dry-run` against the
real crates.io registry and is publish-ready. Once this project's owner
actually runs `cargo publish` — a deliberate, irreversible action nobody has
taken yet — installing becomes:

```sh
cargo install piano-cli   # not live yet — see above
```

This line is written in advance so the day it becomes true, nothing else in
this README needs to change.

## Try it

### Render a note to a file

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

### Play one note live, no file involved

`--damping`/`--sustain` override the built-in voicing for that note:

```sh
cargo run --release -p piano-cli -- play --note A4 --seconds 3 --damping 0.7
```

### Play from the computer keyboard

Nothing is written to disk — this is live playback:

```sh
cargo run --release -p piano-cli -- keyboard
```

The bottom row (`Z S X D C V G B H N J M ,`) is one octave; the top row
(`Q 2 W 3 E R 5 T 6 Y 7 U I 9 O 0 P`) continues upward. `[`/`]` and `-`/`=`
change damping and sustain live, audible on notes already ringing. Esc or
Ctrl+C to quit. Holding several keys plays a chord — one voice per key, no
stealing needed.

**Key-release is terminal-dependent**: most terminals, including macOS's
default Terminal.app, never report a key-up event at all, so notes ring out
on their own; a terminal that implements the Kitty keyboard protocol
(kitty, WezTerm and similar) is detected automatically and gets real early
release. `piano midi` below always has real note-off regardless of
terminal.

### Play from a MIDI controller

A digital piano over USB or a MIDI cable:

```sh
cargo run --release -p piano-cli -- midi --list   # see what is plugged in
cargo run --release -p piano-cli -- midi
```

Notes play as struck and release on note-off; CC74 (brightness, if your
controller has one) drives damping and CC1 (mod wheel) drives sustain, both
live and both distinct from CC64, the sustain (hold) *pedal* — holding it
keeps every note ringing past its own note-off, same as a real piano's
right pedal, and lets the rest of the instrument audibly ring along with it
through a shared bridge coupling. Most notes also have their own beating,
from the 2–3 real physical strings each key is struck by (1 in the bass, 2
in the tenor, 3 in the treble — the standard piano layout). Play several
notes together for a chord. Esc or Ctrl+C in the terminal to quit.

### Play in a browser tab

No install beyond `rustup` and a one-time `wasm-bindgen-cli`:

```sh
rustup target add wasm32-unknown-unknown   # once
cargo install wasm-bindgen-cli --version 0.2.100 --locked   # once; must match the wasm-bindgen version in Cargo.toml
cargo build -p piano-wasm --release --target wasm32-unknown-unknown
wasm-bindgen --target web --out-dir crates/piano-wasm/www/pkg target/wasm32-unknown-unknown/release/piano_wasm.wasm
cd crates/piano-wasm/www && python3 -m http.server 8080
```

Open `http://localhost:8080`, move the frequency slider, click Strike. A
plain static file server is enough — and required, since browsers refuse to
load ES modules or WASM over `file://`. This build is intentionally
simpler than the native one: a single voice, no MIDI, no polyphony yet.

## Why it is built this way

Three constraints drive every structural decision, and they are worth
stating before the file listing:

**It has to be efficient.** A full piano is up to 240 simultaneously
ringing strings, and the audio callback has 2.67 ms to compute every 128
samples. This is why the string model is a digital waveguide (`O(1)` per
string per sample) rather than a finite-difference grid, and why known
bottlenecks are written down [before the code that hits
them](docs/PERFORMANCE.md).

**It must never lock up.** Not "rarely" — never. An audio callback that
misses its deadline produces an audible click; one that panics takes down
the host. So the DSP core allocates nothing while processing, locks
nothing, panics nowhere, and has no unbounded loops. These are [enforced
structurally](docs/REALTIME-AUDIO-RULES.md), not by discipline.

**The engine must not know where it is running.** Native, browser, plugin,
test — the physics is the same code. Everything platform-specific lives in
a crate that depends on the core, never the reverse.

## Where this can still go

Everything on the original roadmap (M1–M8) is done — see
[`docs/ROADMAP.md`](docs/ROADMAP.md) for the milestone-by-milestone history,
including honest notes on what each one did and did not verify. Two
concrete next steps exist but are deliberately not started automatically,
since both involve a public/irreversible action only this project's owner
should trigger:

- **Publishing to crates.io**, so `cargo install piano-cli` works without
  cloning the repository first — everything is already verified
  publish-ready (`docs/ROADMAP.md`'s M8 entry has the full, crate-by-crate
  detail).
- **An audio plugin** (CLAP, VST3, AU) so this can run inside a DAW like
  Ableton or Logic — researched and documented in
  [`docs/PLUGIN-PATH.md`](docs/PLUGIN-PATH.md), with a concrete recommendation
  (CLAP) and an honest "not yet" on whether it's worth building right now.

Recorded but unscheduled: fitting the model's parameters automatically to a
real recorded instrument via Bayesian inference, as a separate *offline*
calibration tool that depends on this project rather than shipping inside
it — see the "Ideas beyond M8" section of `docs/ROADMAP.md`.

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
| [**Guia em português**](docs/pt-BR/LEIA-ME.md) | Comece por aqui se preferir português — o que é, como instalar, como usar |
| [**A aula completa (pt-BR)**](docs/pt-BR/COMO-FUNCIONA.md) · [PDF](docs/pt-BR/COMO-FUNCIONA.pdf) | The physics and the math, explained from scratch in Portuguese, for any reader — no background required |
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

# The benchmark harness — not part of `cargo test`, run by hand and quoted
# in docs/PERFORMANCE.md:
cargo bench -p piano-core                                    # per-component criterion benchmarks
cargo test --release -p piano-audio -- --ignored --nocapture # whole-engine and p99.9 timing numbers

# Packaging checks — also not part of `cargo test`, and never `cargo
# publish` without `--dry-run` (see docs/ROADMAP.md's M8 entry):
cargo publish --dry-run -p piano-core                         # verified clean against the real registry
gh workflow run release-build.yml                              # builds all three platforms by hand
```

See [CONTRIBUTING.md](CONTRIBUTING.md).

## Licence

MIT OR Apache-2.0, at your option.

This project does not copy from copyleft-licensed prior art. See
[docs/PRIOR-ART.md](docs/PRIOR-ART.md) for what that means in practice and
how other open piano models are used as benchmarks rather than sources.
