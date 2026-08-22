# piano

A physically modelled piano synthesiser in Rust.

Not a sampler. There are no recordings of a piano anywhere in this repository —
the sound is computed from a model of what a vibrating string actually does. The
long-term target is the class of instrument that commercial modelled pianos
occupy; the short-term target is to get there one audible step at a time.

**Status: milestone M2.** A plucked string renders to a WAV file, in tune to
within about 1.5 cents, and now also plays live through the speakers — either
one note at a time or from the computer keyboard. See the
[roadmap](docs/ROADMAP.md).

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

Play one note live, no file involved:

```sh
cargo run --release -p piano-cli -- play --note A4 --seconds 3
```

Play live from the computer keyboard — nothing is written to disk:

```sh
cargo run --release -p piano-cli -- keyboard
```

The bottom row (`Z S X D C V G B H N J M ,`) is one octave; the top row
(`Q 2 W 3 E R 5 T 6 Y 7 U I 9 O 0 P`) continues upward. Esc or Ctrl+C to quit.

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
| [Decisions](docs/adr/) | Architecture decision records |

## Development

```sh
cargo test --workspace                                      # 70 tests
cargo clippy --workspace --all-targets -- -D warnings       # must be clean
cargo fmt --all --check
cargo build -p piano-core --no-default-features             # no_std must keep building
```

See [CONTRIBUTING.md](CONTRIBUTING.md).

## Licence

MIT OR Apache-2.0, at your option.

This project does not copy from copyleft-licensed prior art. See
[docs/PRIOR-ART.md](docs/PRIOR-ART.md) for what that means in practice and how
other open piano models are used as benchmarks rather than sources.
