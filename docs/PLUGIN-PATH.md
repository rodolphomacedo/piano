# A documented plugin path

M8's own roadmap text asks for "a documented plugin path if it is worth
taking" — explicitly allowing the honest answer to be "not yet." This
document is that research and that answer. It was written from published
specifications and official documentation only, per `docs/PRIOR-ART.md`'s
licence-hygiene rule: no GPL/AGPL reference plugin implementation was read
to produce it.

## Why this is even plausible: the property already in place

`docs/ARCHITECTURE.md`'s one governing idea is that **the synthesis engine
knows nothing about the outside world**. `piano-core` has no `#[cfg]` for
native vs. WASM vs. test, no I/O, no notion of a host. `piano-audio::Engine`
already looks almost exactly like what a plugin's audio callback needs:

- A `process_block`-shaped entry point that is allocation-free, lock-free,
  panic-free and bounded — the same contract `docs/REALTIME-AUDIO-RULES.md`
  enforces for `cpal`'s callback is, verbatim, the real-time contract every
  plugin API (CLAP, VST3, AU) imposes on its own `process()` callback. This
  project did not write these rules anticipating plugin hosts, but a host's
  requirements turn out to be the same requirements a sound card already
  imposed.
- A command-queue boundary (`Command::NoteOn`/`NoteOff`/`SustainPedal`/
  `SetDamping`/`SetSustain`, the SPSC ring from `piano-audio::commands`)
  that already separates "a control changed" from "render the next block" —
  which is exactly the split a plugin wrapper needs between a host's
  parameter-automation/MIDI callback (off the audio thread, in most APIs)
  and its `process()` callback (on it).
- Per-voice, per-register parameter tables (`piano-audio::voicing`) that a
  plugin's parameter list would expose directly, rather than needing new
  ones invented.

What is **not** already in place, for any format: a way to describe
"parameters" and "ports" to a host in that host's own metadata format, a way
to receive a host's audio buffer layout (which varies by host and by block
size) rather than owning a `cpal` stream outright, and a build target that
produces the shared-library shape (`.clap`, `.vst3` bundle, or AU component)
each host expects to load.

## The formats, evaluated against this project's actual constraints

### CLAP — the realistic choice

[CLAP](https://cleveraudio.org) ("CLever Audio Plugin") is specified and
published by Bitwig and u-he under the **MIT licence**. The spec itself (the
C header, `clap.h`, and the accompanying documentation at
cleveraudio.org/clap-plugins/) is what this document was written from.

- **Licence compatible.** MIT, no dual-licence trap, no fee, nothing that
  would force a `piano-clap` crate out of this project's own MIT/Apache-2.0
  workspace.
- **Cross-platform.** One format, one build artifact shape (a `.clap`
  bundle, effectively a shared library plus a manifest) across macOS,
  Windows and Linux — matching this project's own three-OS ambition in
  M8 rather than adding a fourth per-OS branch.
- **Rust-native tooling exists.** `clap-sys` (raw FFI bindings to the C
  header) and safe wrapper crates such as `clack` exist in the Rust audio
  ecosystem. This document does not depend on either — the description
  above comes from the CLAP spec directly — and their exact current licence
  and maturity should be re-checked at the point anyone actually starts
  building against them, not assumed from this document.
- **Its parameter/event model maps onto what already exists.** CLAP's
  `process()` receives an event list (note-on, note-off, parameter changes,
  MIDI) timestamped within the block and a set of audio port buffers for
  that call — structurally the same shape as this project's own SPSC
  command queue plus `process_block`, just described in CLAP's vocabulary
  instead of this project's own `Command` enum.

### VST3 — technically fine, licence-incompatible as shipped

Steinberg's VST3 SDK is dual-licensed: **GPLv3, or a commercial Steinberg
licence**. There is no MIT/Apache-2.0-compatible path to link against the
official SDK. A crate built against it would have to either accept GPLv3
for that one crate (which `docs/PRIOR-ART.md`'s licence-hygiene stance rules
out for this project — the same reasoning that makes OpenPiano a benchmark
and never a source applies here too, even though this is Steinberg's own
SDK rather than a copylefted competitor's implementation) or obtain a
commercial licence, which is a business decision, not an engineering one,
and well outside this document's scope. **Not viable under this project's
current licensing without a deliberate, separate decision by the project's
owner.**

### AU (Audio Unit) — technically fine, platform-locked

Apple's Audio Unit API (`AudioToolbox`/`AVFoundation`, current form AUv3) has
no copyleft entanglement and is free to build against. But it is
**macOS/iOS-only** — there is no AU host on Windows or Linux — which cuts
directly against this same milestone's other stated goal of reproducible
builds across all three platforms this project already targets. It would
also be the only format requiring Swift/Objective-C glue or a
`cargo-apple`-style toolchain wrinkle none of this project's existing crates
need. Realistic only as a macOS-specific addition *after* a cross-platform
format already exists, not as a first plugin target.

## What a `piano-plugin` (working name) crate would actually need to do

Scoped against the existing crate boundary in `docs/ARCHITECTURE.md`, a CLAP
wrapper would sit at the same layer `piano-audio` and `piano-wasm` already
occupy — a new leaf depending on `piano-core` (and probably reusing
`piano-audio`'s `Engine`, `voicing` tables and `Command` enum rather than
duplicating them), never the reverse:

1. **Plugin description and factory.** Implement CLAP's `clap_plugin_entry`
   and `clap_plugin_factory` to describe the plugin (name, ID, category —
   an instrument, not an effect) to the host. New code; nothing to reuse.
2. **Parameter list.** Expose damping, sustain and inharmonicity per voice
   (or a simplified global set, a UX decision for whoever builds this) as
   CLAP parameters, each mapped to the existing `Command::SetDamping`-style
   messages `piano-audio::commands` already defines. Mostly a translation
   layer over what exists.
3. **Note/event handling.** Map CLAP's note-on/note-off/MIDI events (or
   VST3/AU's equivalents, if ever built) onto `Command::NoteOn`/`NoteOff`,
   the same mapping `piano-cli`'s `midi.rs` already does for real MIDI
   hardware — meaning this step has a working precedent to copy the shape
   of, in this project's own code, not a foreign one.
4. **The `process()` callback.** Receive the host's audio buffer (host-owned
   memory, a layout `piano-audio::Engine` does not currently know about
   since `cpal` owns that today) and call the existing `process_block_add`
   path into it. This is the one piece with a real, if bounded, unknown: it
   needs verifying that a host's arbitrary block size interacts correctly
   with `BRIDGE_BLOCK_SAMPLES`-sized chunking (`docs/ARCHITECTURE.md`
   describes why `Engine` chunks internally); this is very likely fine
   since the chunking already exists to decouple the bridge bus's own block
   size from whatever the caller passes in, but "very likely fine" is not
   the same as measured, and a real implementation would need to prove it
   the same way `docs/PERFORMANCE.md` proves everything else here — with a
   benchmark, not an assumption.
5. **The `.clap` bundle and build wiring.** A `crate-type = ["cdylib"]`
   target (the same shape `piano-wasm` already uses for its own
   browser-loadable artifact) plus the small amount of packaging CLAP's own
   docs specify (a `.clap` file is close to a renamed shared library with a
   manifest) — new work, but mechanical, not a design problem.

Roughly: steps 2 and 3 are thin translation over code that already exists;
step 1 and step 5 are new but well-specified by CLAP's own documentation;
step 4 is the one place with a real, currently unverified integration risk.

## Recommendation: documented, not taken, not yet

**CLAP is the right format if and when this project builds a plugin.** MIT
licensed, cross-platform, and its real-time contract is close enough to
this project's own `docs/REALTIME-AUDIO-RULES.md` that adopting it is
mostly plumbing rather than a new discipline to learn.

**But M8 does not build it, for reasons specific to this milestone rather
than the idea's merit:**

- M8's own "done when" line is "someone who is not the author can install
  it and play it" — already achievable through the CLI (build from source,
  or eventually `cargo install`) and the browser build, with no plugin
  needed to clear that bar.
- A real CLAP integration needs a **host to test against** (a DAW, or the
  reference `clap-validator` tool) to make any claim beyond "it compiles" —
  the same standard this document has held every other milestone to
  (`docs/PERFORMANCE.md`: "closed only with a measurement"). No such host
  was available to verify against while writing this document, so shipping
  an unverified plugin wrapper would violate this project's own stated
  standard for what "done" means, not just be premature.
- It is genuinely new surface area — a new crate, a new build target shape,
  a new category of manual/host-based testing this project has not needed
  before — for a milestone whose own scope line ends at "a documented
  plugin path if it is worth taking," which this document reads as
  permission to stop at *documented*.

**Recommended next step, if a future milestone picks this up:** start from
CLAP specifically (not VST3, not AU, per the licence and platform reasoning
above), reuse `piano-audio::Engine`/`Command`/`voicing` rather than forking
them, and treat step 4 above (arbitrary host block size vs. the bridge bus's
own chunking) as the first thing to benchmark, not the last — the same
"measure before optimising" discipline `CONTRIBUTING.md` already holds
everything else in this project to.
