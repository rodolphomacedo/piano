# The physics, and how it becomes code

Every component in `piano-core` corresponds to something a real string does. This
document is the map between the two. It is deliberately intuitive first and
mathematical second — the equations are in the papers listed in
`docs/PRIOR-ART.md`.

## Why a delay line is a string

The ideal string obeys the wave equation, and its solution is not a single
vibration but **two travelling waves**, one going left and one going right,
bouncing between the two fixed ends.

That is the whole insight behind digital waveguides. Instead of simulating the
string's shape at every point (which costs one update per grid point per sample),
you simulate the two travelling waves — and a travelling wave in a computer is
just a **delay line**. One round trip takes

```
period = sample_rate / fundamental_frequency
```

samples, and that is the delay length. The cost is `O(1)` per string per sample
regardless of how long the string is, which is why this project chose waveguides
over finite differences.

## Why there is a lowpass filter in the loop

If the reflections were perfect, the note would ring forever. They are not: energy
leaves through the bridge, and the string loses energy to air and to internal
friction. Crucially, **it does not lose it uniformly** — high partials die much
faster than low ones.

That is exactly the behaviour of a lowpass filter. It is why a piano note goes
bright, then mellow, then quiet, in that order.

Implemented as `filter::LoopFilter`, a one-pole, one-zero design (M4; see "Why
upper partials sit sharp", below, for the allpass dispersion cascade this
composes with). The `damping` parameter is the pole; `zero_mix` is the second
parameter the section below exists to explain.

### Why the zero had to become a second, per-string parameter

M4 added a fixed zero at Nyquist on top of the pole, for extra high-frequency
rolloff a bare pole cannot give (D. Jaffe & J. O. Smith, "Extensions of the
Karplus-Strong Plucked-String Algorithm", 1983). That zero costs real
amplitude at any fundamental that is not close to DC, independent of the
pole — even `pole = 0` still carries the zero's own loss. For a bass string
that loss is negligible (A0's fundamental sits far below Nyquist), but for a
treble string completing thousands of round trips a second it compounds
catastrophically: measured with the zero fixed at Nyquist and `damping`
applied uncorrected, a C8 string decayed to silence in about **12
milliseconds** against this document's own 1-2 s target — the zero alone was
costing about 16% of the fundamental's amplitude every round trip, and no
value `sustain` can take (bounded to `[0, 1]`) can put amplitude back that
the filter already removed.

`piano_audio::voicing::loop_filter_coefficients` is the fix: `filter::
LoopFilter` now takes `zero_mix` as well as `pole`, and that function scales
both down together, key by key, until the loop filter's own gain at that
key's fundamental clears the round-trip budget its target decay time
allows. Bass keeps the full desired pair (`pole = 0.6`, `zero_mix = 0.5`,
unaffected); by C8 both are turned down to roughly a hundredth of
themselves. Verified two ways: analytically, against `LoopFilter::
magnitude_at`'s own closed-form gain; and empirically, by seeding a minimal
delay-line-plus-filter loop (no dispersion, no hammer) with a plain sine
wave and measuring its actual per-cycle decay, which matched the analytic
prediction to within `0.03%`.

**What this does not yet fix.** A real, hammer-plucked note still falls
well short of the full target even after this correction — a *different*,
pre-existing limitation, not a flaw in this calibration. See "What the
current model still does not do", below.

### Why the mix bus needed a limiter too

A separate, unrelated gap surfaced alongside the one above: nothing summed
every ringing voice plus the soundboard against any ceiling before handing
samples to the audio device, contradicting this project's own stated
invariant ("Output is bounded by construction",
`docs/REALTIME-AUDIO-RULES.md`) once more than one voice could be ringing —
a full chord, or the sustain pedal's sympathetic resonance
(`piano_core::bridge`), could sum past `±1.0` and hit whatever hard-clipping
the host's `f32 -> PCM` conversion does. `piano_audio::engine`'s
`process_chunk` now runs every sample through `limiter::soft_limit`: the
identity function below `0.9`, so a single voice's own render is bit-for-bit
unaffected, and a smooth `tanh` saturation into the remaining headroom above
it — a standard soft-knee limiter, not a physical model, so (unlike
everything else in this document) it carries no literature citation.

## Why the loop is shorter than the period

The loss filter is *inside* the feedback loop, so it contributes its own delay.
A one-pole lowpass `H(z) = (1-a)/(1 - a·z⁻¹)` has phase delay `a/(1-a)` samples at
low frequency. There is also one sample of delay from the feedback path itself.

So the delay line must be

```
loop_delay = period − filter_phase_delay − 1
```

Skipping this term detunes the instrument, mildly in the bass and badly in the
treble. It is implemented in `PluckedString::new`, and the test
`loop_delay_is_close_to_the_period` guards it. Measured output for A4 lands within
about 1.5 cents of 440 Hz.

## Why the excitation is a shaped noise burst, not flat noise (M4)

Karplus–Strong fills the delay line with flat noise. Physically this is a very
crude model of a pluck: it says "give the string a random initial
displacement". A piano is not plucked, it is struck by a felt hammer, and the
hammer is what makes a piano sound like a piano.

`piano_core::hammer::simulate_contact` models the hammer's side of a
Hertzian-contact nonlinear spring (Chaigne & Askenfelt 1994,
`F = K·x^p`, `p ≈ 2–3`): harder strikes compress the felt more, which makes
the effective spring stiffer, which makes contact both shorter *and* higher
in peak force — exactly the two effects that make a hard-struck note brighter
rather than merely louder. The string's own motion is not solved
simultaneously (that full coupled problem is `PERF-007`'s harder,
not-yet-built form); this shapes the existing excitation noise rather than
replacing it, which keeps every partial excited (still noise under the hood)
while making the excitation's spectral balance a function of strike velocity.

### Why the envelope alone was not enough

The paragraph above was, for one milestone, only half true, and the missing
half was audible as a hard dry knock — one piece of wood hitting another —
in front of every note. Worth recording, because the mistake is a natural one
to make again.

The contact force envelope reaches the excitation by *multiplication*: the
delay line is filled with `white_noise · velocity · force[n]`. Multiplying in
time is convolution in frequency, and convolving a flat spectrum with
anything at all leaves it flat. The expected power spectrum of an enveloped
white noise burst is `σ²·Σw[n]²` — it depends on the window's total energy,
never on its shape. So the envelope changed *how much* energy a strike
injected and *when*, but not *where in the spectrum*, and every strike at
every velocity injected flat energy density all the way to Nyquist. Measured
on A4: a spectral centroid of 11.8 kHz at velocity 0.1 against 12.7 kHz at
velocity 1.0, either side of the 12 kHz a perfectly flat spectrum gives at
48 kHz. A 7% spread across the whole dynamic range, where the document
claimed that spread *was* the brightness.

The fix is to band-limit the excitation as well as envelope it. A real
hammer's contact force is a smooth pulse of duration `τ`, and a pulse of
duration `τ` is band-limited on the order of `1/τ`; the felt's finite contact
width along the string adds a second rolloff (Van Duyne & Smith,
"Developments for the Commuted Piano", ICMC 1995, use exactly such a
hammer-width lowpass on the excitation). `hammer::excitation_cutoff_hz`
derives a corner from the contact duration `simulate_contact` already
computes, and `PluckedString::write_excitation` runs the burst through two
one-pole sections at that corner. The same nonlinearity that shortens contact
at higher velocity now raises the corner too, so *harder is brighter* is
finally a property of the output rather than of this document.

The proportionality constant between `1/τ` and the corner is calibrated, not
derived, and the reason is the simplification stated above: a real strike's
energy well above `1/τ` comes largely from the string pushing back on the
hammer during contact, and that back-reaction is precisely what this model
does not solve. Taking the force pulse's own corner literally would put a
*fortissimo* strike near 300 Hz and mute the instrument.

Measured on A4 at velocity 0.8, before and after, over the first 5 ms:
spectral centroid 11.2 kHz → 5.9 kHz, peak 1.97 → 0.65. The second number
matters as much as the first: the old attack sat 17 dB above the body of its
own note *and* above full scale, so live playback clipped it into something
harsher still.

### What the hammer still gets wrong

`PluckedString::pluck` writes exactly one loop length of excitation, so a
note whose period is shorter than the felt's contact has its contact envelope
truncated. E2's period is 12.1 ms and holds the whole 3.5-6.5 ms contact;
A4's is 2.27 ms and holds about a third of it; C6's is 1 ms and holds a
seventh. Two consequences: velocity's effect on the attack is weakest exactly
where the truncation is worst, and the top octave is markedly quieter than
the rest of the keyboard. A real hammer stays in contact while the wave makes
several round trips, adding force to a string that is already moving — the
same back-reaction problem, seen from the other side.

## Why upper partials sit sharp (M4)

A real string is stiff, not an idealised flexible one, so its restoring force
includes a bending-stiffness term the wave equation for an ideal string does
not. The consequence, first derived by Fletcher (1964): partial `n` sits at
`f_n ≈ n·f_1·sqrt(1 + B·n²)` rather than exactly `n·f_1`, where `B` is the
string's inharmonicity coefficient. `piano_core::dispersion::DispersionCascade`
reproduces this with a cascade of first-order allpass sections inside the
loop, after the loss filter (Jaffe & Smith 1983's extension to
Karplus-Strong): each section is flat in magnitude but adds a
frequency-dependent phase delay, and enough of them approximate the stretched
dispersion curve. Section count scales with register per the table below —
the bass needs many, the treble barely any.

## Why most notes are more than one string (M6)

A real piano key strikes more than one physical string for most of the
keyboard: a standard modern instrument is single-strung (monochord) in the
bass, double-strung (bichord) through the tenor, and triple-strung
(trichord) for the rest of the treble (A. Reblitz, *Piano Servicing,
Tuning, and Rebuilding*, 1993 — the layout piano technicians work to;
qualitatively consistent with Fletcher & Rossing's own bass/tenor/treble
description, already cited above for inharmonicity). This project uses 12
single-strung keys, 18 double-strung and 58 triple-strung — a
representative choice of break points within that convention, not a
measurement of one specific instrument (exact break points vary by piano
model and scale design) — implemented as
`piano_core::unison::unison_count_for_key_index` and looked up per key by
`piano_audio::voicing::unison_count_for_key`. That raises the *effective*
number of strings the engine ever processes simultaneously from 88 to
`12·1 + 18·2 + 58·3 = 222`, close to `PERF-008`'s own illustrative `N =
240` estimate for "up to three strings on all 88 keys."

`piano_core::unison::UnisonGroup` reuses [`piano_core::PluckedString`]
rather than forking it: a unison group is 1-3 independent strings, each
detuned by a small fixed offset (a few cents — see the module for the
honesty note on the exact figures) and struck together by
`UnisonGroup::pluck`, exactly like a single hammer really does strike every
string of one note at once.

## Why a note has a two-stage decay (M6)

G. Weinreich, "Coupled Piano Strings" (JASA 62(6), 1977) is the seminal
model and measurement of this: near-unison strings are not independent —
they share one mechanical connection, the bridge, and that shared,
slightly-yielding contact point is what makes a real piano note's envelope
bend rather than follow one clean exponential. Two strings tuned a few
cents apart, coupled through a shared bridge, first beat and dephase
against each other (a fast "pre-decay"), then — once that differential
energy has dissipated — settle into a shared mode that decays close to
what a single string's own natural loss rate would give (a slower
"aftersound").

This project reproduces that mechanism, not a hand-tuned envelope shape,
with two coupling tiers implemented as a **convex combination** of each
string's own signal with its neighbours' (`piano_core::string::
PluckedString::write_mixed_feedback`'s doc comment explains why a convex
blend, not a raw sum, is what keeps this stable for any `sustain`):

- **Local** (`piano_core::unison`): a note's own 1-3 strings blend
  sample-accurately, every sample — cheap, since a unison group already
  processes all its strings in one call.
- **Global** (`piano_core::bridge::BridgeBus`, `PERF-008`): cross-*key*
  coupling, e.g. from the sustain pedal, blends with one block
  (~2.7 ms at 48 kHz) of latency, because summing all 88 keys' contributions
  before any one of them can read the total back is not representable in a
  single per-voice, per-sample call without sacrificing the cache-friendly
  loop order `PERF-010` already established.

Measured, not asserted: `crates/piano-render/tests/m6_spectral.rs` renders
a trichord A4 and shows its early decay rate (nepers per 0.1 s block, just
after the attack transient) is measurably faster than its settled rate
once beating has died down, and that the settled rate then sits close to
(within a factor of two of) the same note rendered as a monochord control
— the qualitative and quantitative signature Weinreich's model predicts.
This measurement also caught two real implementation bugs during
development, not by inspection: an earlier *additive* coupling term
diverged to infinity for near-lossless (high-`sustain`) strings once
enough voices became simultaneously receptive (fixed by the convex-blend
redesign above and by `BridgeBus` averaging rather than summing
contributions), and every unison string sharing one excitation noise seed
suppressed the very beating the model exists to produce (fixed by
`piano_core::unison::reseed_for_string`). Both are documented in full in
the modules that fixed them.

## Why the sustain pedal makes the rest of the instrument ring (M6, `PERF-008`)

Before M6 the sustain pedal only changed *when* a struck voice's own
damper engaged — it had no effect on any other string, because nothing
coupled voices together. `piano_core::bridge::BridgeBus` is the fix: every
voice writes its own bridge-end signal into one shared running average and
reads back everyone else's, so a string whose damper the pedal has lifted
(even if it was never struck) picks up a little energy from whatever *is*
ringing and starts to audibly resonate — a real piano's sustain pedal
lifts every damper on the instrument, not just the one under a held key.
`piano_audio::engine::Engine::set_sustain_pedal` is what actually lifts
those idle dampers; `piano_core::unison::UnisonGroup::is_receptive`
distinguishes a genuinely-damped, safely-skippable voice
(`PERF-006`) from a silent-but-undamped one that must still be processed
so it can wake up.

The bridge deliberately does not model per-pair coupling (`O(N²)`,
infeasible at this string count — see `PERF-008` in
`docs/PERFORMANCE.md`) or per-string admittance (this project has no
per-instrument measurement to derive one from): every voice shares the
same coupling gain and the same bus, an engineering simplification stated
plainly rather than presented as more faithful than it is.

## Why every note is coloured by a soundboard (M6, `PERF-009`)

A real piano's strings barely radiate sound on their own — their thin
cross-section couples poorly to air. Almost everything a listener hears
comes from the soundboard, the large wooden plate the bridge drives.
`piano_core::soundboard::Soundboard` models that as a bank of
[`MODE_COUNT`](../crates/piano-core/src/soundboard.rs) damped resonant
filters (modal synthesis — B. Bank et al., already cited in
`docs/PRIOR-ART.md` for exactly this architecture) rather than convolving
against a measured impulse response. That second option is not a choice
this project can make regardless of engineering merit: an impulse response
is, by definition, recorded from a real instrument, and this repository's
own rule (see the root `CLAUDE.md`) is that no recorded or sampled audio
asset may be added, ever, for any reason. Modal synthesis needs no such
recording — each mode is a frequency, a decay time and a gain, all
literature-informed order-of-magnitude values (see the module's own
honesty note on where they come from and what they are not), not fit to
any instrument's measured response.

The soundboard's output is **mixed in**, not substituted for the direct
signal (`piano_audio::engine::Engine::process_chunk`'s
`SOUNDBOARD_MIX_GAIN`): replacing the direct signal entirely would be more
faithful to how a real piano is *only* ever heard through its soundboard,
but would also change the level and shape of the fundamental partial in
every already-measured tuning and inharmonicity test this project has
(M1's cents figure, M4's partial-sharpening and brightness measurements)
— an honest trade stated here rather than silently made.

## What the current model still does not do

Stated plainly, because these are the gaps a later milestone would close:

| Missing | Consequence | Milestone |
|---|---|---|
| **Longitudinal modes** | No metallic "phantom partials" of the low bass. | Backlog |
| **Simultaneous hammer/string coupling** | The hammer model (above) does not yet feed the string's own motion back into the contact force during the strike. | Backlog |
| **Per-string bridge admittance** | Every voice couples to the shared bridge bus at the same fixed gain; a real bridge's admittance varies with frequency and string position. | Backlog |
| **The hammer excitation's noise burst does not concentrate on the resonant fundamental** | `PluckedString::pluck` seeds the loop with broadband noise, shaped only in envelope and overall cutoff (see "Why the excitation is a shaped noise burst", above) — most of that energy is off-resonance and the delay line's own comb selectivity cancels it out within the first few hundred round trips, *regardless* of how gentle the loop filter is. That burn-in spends a large fraction of a note's total dynamic range before the correctly-calibrated slow tail (see "Why the zero had to become a second, per-string parameter", above) ever gets to dominate, so a real render still lands well under the per-register decay times this document states — measured on a real, calibrated C8: about 0.4 s reached, not the full 1-2 s. Bass notes are least affected (their decay budget is largest relative to the burn-in's fixed cost); this is most visible from the upper mid register up. | Backlog |

## Numbers worth having

At 48 kHz on an 88-key piano:

| | A0 (27.5 Hz) | A4 (440 Hz) | C8 (4186 Hz) |
|---|---|---|---|
| Period, samples | 1745 | 109 | 11.5 |
| Delay buffer (pow-2) | 2048 | 128 | 16 |
| Memory per string | 8 KB | 512 B | 64 B |
| Typical decay | 30–40 s | 8–15 s | 1–2 s |
| Dispersion sections needed | ~8 | ~2 | ~0–1 |

The last row is why `PERF-005` insists that dispersion order be scaled per
register rather than fixed: paying for eight allpass sections on C8 would be
half the treble's CPU budget spent on an effect nobody can hear.
