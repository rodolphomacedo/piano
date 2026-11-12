# piano

**A piano with no piano in it.**

No samples, no recordings, no impulse responses. Every sound is computed,
48,000 times a second, from what a steel string does when a felt hammer
hits it.

```sh
git clone https://github.com/rodolphomacedo/piano.git && cd piano
cargo run --release -p piano-cli -- keyboard
```

The first run compiles for a few minutes. After that your computer keyboard
is the instrument: `Z S X D C V G B H N J M` is an octave, `Q 2 W 3 E R …`
the next one up. All you need is [`rustup`](https://rustup.rs).

> **Em português:** [`docs/pt-BR/LEIA-ME.md`](docs/pt-BR/LEIA-ME.md) explica o
> que é, como instalar e como tocar, sem jargão.

## What is being computed

A sampler plays back what a microphone once heard. This asks what the
instrument is doing, and does that instead.

| A piano has | Here it is |
|---|---|
| a felt hammer | a mass on a nonlinear spring, `F = K·ξ^p`, solved against the moving string every sample |
| a stiff steel string | a delay line one period long, a filter that loses the treble first, and an allpass cascade that pushes the partials sharp |
| two or three strings to a key | strings that share a bridge: together they give the loud start of a note, against each other the long tail |
| a soundboard | 28 resonances, a diffuse field of bending waves, and a radiation curve that explains why you never hear a bass note's fundamental |
| a tuner | octaves tuned beatless from the strings' own stiffness, so the top is 30 cents sharp, as on a real one |
| three pedals | one rule about where each damper rests |
| a room | a delay network, off when you measure and on when you play |

Nobody chose that a harder strike should sound brighter. The felt gets
stiffer as it is squeezed, throws the hammer back sooner, and the shorter
blow carries more high partials. Most of what the model does well it does
that way: it falls out.

The numbers the physics leaves open — how hard this felt is, how fast that
partial dies — were measured on a concert grand. The measurements are in
the repository. The grand is not.
[`docs/PHYSICS.md`](docs/PHYSICS.md) has the reasoning and the papers;
[`docs/CALIBRATION.md`](docs/CALIBRATION.md) has the method.

## Play it

```sh
cargo run --release -p piano-cli -- keyboard                 # the computer keyboard
cargo run --release -p piano-cli -- midi                     # a MIDI controller, pedals included
make run-studio                                              # a browser page with every parameter live
cargo run --release -p piano-cli -- render --note A4 -o a4.wav
cargo run --release -p piano-cli -- analyze --note A4        # measure a note instead of hearing it
```

`make run-studio` serves `http://127.0.0.1:7878`: play from the page or
from a MIDI keyboard, move a string's stiffness or a hammer's felt while it
rings, save the result as a `.piano.json`. Every subcommand answers
`--help`.

<details>
<summary>Try it in a browser</summary>

The core also compiles to WebAssembly and runs in an `AudioWorklet` — one
voice, no MIDI, a demonstration that the engine does not care where it is.

```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.100 --locked   # must match Cargo.toml's wasm-bindgen
cargo build -p piano-wasm --release --target wasm32-unknown-unknown
wasm-bindgen --target web --out-dir crates/piano-wasm/www/pkg target/wasm32-unknown-unknown/release/piano_wasm.wasm
cd crates/piano-wasm/www && python3 -m http.server 8080
```

</details>

## Three rules

**The audio thread never waits.** It allocates nothing, locks nothing,
cannot panic and has no unbounded loop. The hammer's nonlinear equation is
solved in four Newton steps, always four. This is enforced by the shape of
the code, not by care: [`docs/REALTIME-AUDIO-RULES.md`](docs/REALTIME-AUDIO-RULES.md).

**Costs are written down before they hurt.** A full piano is 222 strings
and a deadline of 2.67 ms. Every known bottleneck has an entry in
[`docs/PERFORMANCE.md`](docs/PERFORMANCE.md), opened when it is noticed and
closed only by a measurement.

**The engine does not know where it runs.** `piano-core` is `no_std`,
forbids `unsafe`, and is the same code in a terminal, a browser tab and a
test.

## What it does not do yet

It is not a concert grand, and it says where. The softest notes at the very
top can lose their fundamental. One key can sit a few decibels off its
neighbours. The strings' longitudinal resonances are missing. The list,
with causes, closes [`docs/PHYSICS.md`](docs/PHYSICS.md). The ear's verdict
on the rest is still out.

## Inside

| Crate | |
|---|---|
| `piano-core` | the physics: string, hammer, unison, soundboard, body, radiation, room |
| `piano-params` | note names, MIDI numbers, tuning |
| `piano-audio` | the 88-key engine, voicing, realtime output, the lock-free command queue |
| `piano-midi` | MIDI input, decoded into that queue |
| `piano-render` | offline rendering to WAV |
| `piano-analysis` | measurements of a rendered note |
| `piano-studio` | the `.piano.json` format and the local web studio |
| `piano-wasm` | browser bindings |
| `piano-cli` | the `piano` binary |

Further reading: [architecture](docs/ARCHITECTURE.md) ·
[roadmap](docs/ROADMAP.md) · [prior art and licence policy](docs/PRIOR-ART.md) ·
[decisions](docs/adr/) · [changelog](CHANGELOG.md) ·
[a aula completa, em português](docs/pt-BR/COMO-FUNCIONA.md)

## Working on it

```sh
make verify    # fmt, clippy -D warnings, tests, and the no_std build
```

[`CONTRIBUTING.md`](CONTRIBUTING.md) has the rest.

MIT or Apache-2.0, at your option. No copyleft source was read to write it.
Sponsored by [Grabatus Labs](https://grabatus.com).
