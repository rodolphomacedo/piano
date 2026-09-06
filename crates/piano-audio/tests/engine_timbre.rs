//! Full-`Engine` timbre regression — issue #87 / `docs/MODEL-REVIEW.md` N1.
//!
//! Every other timbre measurement in this repository
//! (`tests/timbre_diagnostic.rs`, `piano-render`'s `m*_spectral.rs`) renders
//! a bare [`piano_core::PluckedString`] or a `PluckedString` + `Soundboard`
//! pair. **None render the `Engine`** — the only path a player hears, and
//! where unison coupling, the shared bridge bus, the soundboard mix and the
//! output limiter live. That gap is exactly how the A5 trichord collapse
//! (`docs/TIMBRE-PLAN.md` D10) passed every test: A5 measured fine as one
//! string and died in ~120 ms once three detuned strings were blended.
//!
//! This file renders through [`piano_audio::offline::OfflineEngine`] and,
//! for each key, checks four things against that key's *own* solved intent
//! ([`piano_audio::voicing::solved_decay_targets`]):
//!
//! 1. **Fundamental decay** — seconds to −20 dB, inside a wide band around
//!    the solved target.
//! 2. **Per-partial decay ratio** — H1 vs the bright partial, guarding
//!    against the "every partial decays together" organ signature (D1).
//! 3. **Radiated energy** — an early window and a later one, so a key that
//!    collapses to silence is caught rather than waited for.
//! 4. **Monochord vs trichord** — a key whose trichord behaves very
//!    differently from its own monochord is the D10 signature.
//!
//! Every failure names the key and the measured value. The all-88 sweep is
//! `#[ignore]`d because it is minutes of 88-voice synthesis; run it with
//! `cargo test -p piano-audio --release --test engine_timbre -- --ignored`.
//! The representative-keys test runs in the normal gate and is the CI guard.
//!
//! The sweep that calibrated the bands (`MEASURED_OVER_ANALYTIC`,
//! `DECAY_BAND`, `UNISON_ENERGY_BAND`) also surfaced a residual: across the
//! upper treble the trichord radiates less late-window energy than its own
//! monochord, worst at A5 — the same region as D10, not fully cleared by
//! the dispersion-coefficient fix that closed it. A5's fundamental decays at
//! 0.42× its solved intent and its trichord at 0.09× its monochord: inside
//! the bands here, but the closest any key comes to failing. Tracked in
//! issue #92; a fix belongs to the bridge/soundboard-coupling milestone
//! (`docs/MODEL-REVIEW.md` P4), not to this harness.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use piano_audio::offline::OfflineEngine;
use piano_audio::voicing::{solved_decay_targets, unison_count_for_key};
use piano_params::{HIGHEST_PIANO_KEY, LOWEST_PIANO_KEY, PianoKey, Tuning};

const SAMPLE_RATE_HZ: f32 = 48_000.0;

/// One analysis window for the windowed single-bin DFT, matching
/// `tests/timbre_diagnostic.rs` — long enough to resolve a bass
/// fundamental, short enough to sample a treble note's decay several times.
const WINDOW: usize = 8_192;

/// What fraction of the analytic ring-out `solved_decay_targets` reports a
/// full-engine −20 dB decay actually measures. Two effects multiply:
///
/// * **−20 dB vs 80 dB** — the diagnostic looks for a 20 dB drop while the
///   solve targets 80 dB (`SILENCE_THRESHOLD` is `1e-4`). `docs/TIMBRE-PLAN.md`'s
///   F1 note measures this at ~0.25 for a bare string ("A0's analytic H1 is
///   33.9 s and it measures 8.53 s").
/// * **bare string vs full engine** — `docs/MODEL-REVIEW.md` N2 measures the
///   engine's unison blend and bridge coupling taking a further ~20% ("bare
///   string 2.56 s, full engine 2.05 s" on A4).
///
/// `0.25 × 0.80 ≈ 0.20`, and the all-88 sweep in this file lands every key at
/// 0.79–1.32× the `expected` this produces — bar A5, at 0.42×, the low
/// outlier of the upper-treble trichord lean tracked in issue #92. Renders
/// and bands are sized off it.
const MEASURED_OVER_ANALYTIC: f32 = 0.20;

/// How far either side of the expected measured decay a key may land before
/// it is a regression. A factor of three catches a note that has collapsed
/// (measures far short) or is ringing like a bar (far long) without pinning
/// a calibration the least-squares fit deliberately only approximates —
/// same "wide on purpose" reasoning as `voicing`'s own ratio bands. The
/// all-88 sweep spans 0.42–1.32× (A5 the low end), so a factor of three
/// clears every current key while still failing a true D10-style collapse
/// (~0.07×) or a bar-like over-ring.
const DECAY_BAND: f32 = 3.0;

/// Band for H1 / H(bright) decay-time ratio. Below ~1.6 the partials are
/// dying together (the "metallic" D1 defect, A0 measured 1.18 before F1);
/// above ~25 the bright partial is gone before the note has spoken.
const RATIO_BAND: std::ops::Range<f32> = 1.6..25.0;

/// The H1 / H(bright) ratio is only asserted where the bright partial sits
/// below this. Above roughly 2 kHz a piano's 8th partial is already 40–60 dB
/// down and its *own* 20 dB decay time, measured off a full-engine render
/// with soundboard and limiter in the path, is noise — `voicing`'s own
/// `upper_partials_decay_several_times_faster` reads that ratio analytically
/// for exactly this reason. The metallic D1 defect this guards against was
/// measured at A0, where the bright partial is strong; that is the register
/// this check belongs in.
const RATIO_MAX_PARTIAL_HZ: f32 = 2_000.0;

/// RMS floor for the first ~0.4 s of a note: below this the strike barely
/// made a sound at all. The all-88 sweep's quietest early window is ~9e-3
/// (top octave); this sits an order of magnitude under that.
const EARLY_RMS_FLOOR: f32 = 1.0e-3;

/// RMS floor for the later window: below this the note has collapsed to
/// silence well before its target ring-out — the D10 failure. The sweep's
/// quietest later window is ~1.4e-3 (a bass monochord control); a true D10
/// collapse fell to the noise floor, orders of magnitude below this.
const LATE_RMS_FLOOR: f32 = 1.0e-4;

/// Bounds on trichord-energy / monochord-energy in the later window. D10 was
/// a trichord at essentially zero against a healthy monochord; the upper
/// bound catches the opposite runaway. The all-88 sweep spans 0.09–2.55
/// (A5 the low end, top of the treble the high end — the same upper-treble
/// trichord lean as `DECAY_BAND`'s note, issue #92); `0.04..4.0` clears
/// every current key while still failing a trichord that has genuinely
/// collapsed (D10 measured ~5e-3) or run away.
const UNISON_ENERGY_BAND: std::ops::Range<f32> = 0.04..4.0;

/// Keys spanning the compass, including the G5–B5 band where D10's single
/// dispersion section makes a trichord unstable and A5 where it was worst,
/// and F2 (41) where a too-low `DEFAULT_EXCITATION_NOISE_MIX` starves the
/// bright partial and the H1/H8 ratio runs past its ceiling (#77 follow-up).
const REPRESENTATIVE_KEYS: &[u8] = &[21, 33, 41, 45, 57, 69, 76, 79, 80, 81, 82, 83, 93, 100, 108];

/// A far smaller set for the debug build, where 88-voice synthesis is an
/// order of magnitude slower — still one key per register, plus A5 for D10
/// and F2 (41) for the #77 bright-partial-starvation regression.
const DEBUG_KEYS: &[u8] = &[21, 41, 45, 69, 81, 108];

fn tuning() -> Tuning {
    Tuning::default()
}

fn key(midi: u8) -> PianoKey {
    PianoKey::from_midi(midi).expect("midi names a piano key")
}

fn fundamental_hz(midi: u8) -> f32 {
    key(midi).frequency(tuning()).hertz()
}

/// Magnitude of `samples` at exactly `frequency_hz` — a Hann-windowed DFT
/// evaluated at one frequency, so a partial sitting sharp of an exact
/// multiple is not snapped to the wrong bin. Same routine as
/// `tests/timbre_diagnostic.rs`.
fn magnitude_at(samples: &[f32], frequency_hz: f32) -> f32 {
    let omega = std::f32::consts::TAU * frequency_hz / SAMPLE_RATE_HZ;
    let (mut real, mut imag) = (0.0f32, 0.0f32);
    for (index, &sample) in samples.iter().enumerate() {
        let window =
            0.5 - 0.5 * (std::f32::consts::TAU * index as f32 / samples.len() as f32).cos();
        let phase = omega * index as f32;
        real += sample * window * phase.cos();
        imag -= sample * window * phase.sin();
    }
    (real * real + imag * imag).sqrt() / samples.len() as f32
}

/// Seconds for the partial at `frequency_hz` to fall 20 dB below its own
/// peak window, or `None` if it never does within `samples`.
fn decay_to_minus_20db(samples: &[f32], frequency_hz: f32) -> Option<f32> {
    let windows: Vec<f32> = samples
        .chunks_exact(WINDOW)
        .map(|chunk| magnitude_at(chunk, frequency_hz))
        .collect();
    let (peak_index, peak) =
        windows
            .iter()
            .enumerate()
            .fold((0usize, 0.0f32), |best, (index, &magnitude)| {
                if magnitude > best.1 {
                    (index, magnitude)
                } else {
                    best
                }
            });
    if peak <= 0.0 {
        return None;
    }
    let target = peak * 0.1;
    windows[peak_index..]
        .iter()
        .position(|&magnitude| magnitude < target)
        .map(|offset| (offset * WINDOW) as f32 / SAMPLE_RATE_HZ)
}

/// RMS of the `seconds`-long window starting at `start_seconds`, clamped to
/// what the buffer actually holds.
fn rms_window(samples: &[f32], start_seconds: f32, seconds: f32) -> f32 {
    let start = ((start_seconds * SAMPLE_RATE_HZ) as usize).min(samples.len());
    let end = (((start_seconds + seconds) * SAMPLE_RATE_HZ) as usize).min(samples.len());
    let slice = &samples[start..end];
    if slice.is_empty() {
        return 0.0;
    }
    (slice.iter().map(|s| s * s).sum::<f32>() / slice.len() as f32).sqrt()
}

/// What one key's rendered note actually did, through the full engine.
struct KeyTimbre {
    expected_h1_seconds: f32,
    late_window_start: f32,
    h1_decay_seconds: Option<f32>,
    bright_partial: f32,
    bright_hz: f32,
    bright_decay_seconds: Option<f32>,
    early_rms: f32,
    late_rms: f32,
    render_seconds: f32,
}

/// Renders `midi` at mezzo-forte through `engine` and measures it. Render
/// length is `DECAY_BAND` times the expected measured H1, capped: long
/// enough to watch the fundamental fall 20 dB with margin, short enough that
/// 88 of them finish.
fn measure(engine: &mut OfflineEngine, midi: u8) -> KeyTimbre {
    let targets = solved_decay_targets(key(midi), tuning(), engine.sample_rate());
    let target_h1 = targets[0].1;
    let bright_partial = targets[2].0;
    let expected_h1_seconds = target_h1 * MEASURED_OVER_ANALYTIC;
    let render_seconds = (expected_h1_seconds * DECAY_BAND).clamp(3.0, 16.0);
    let late_window_start = (expected_h1_seconds * 0.6).clamp(0.3, render_seconds - 0.4);

    engine.note_on(midi, 0.8);
    let samples = engine.render(render_seconds);

    let f0 = fundamental_hz(midi);
    let bright_hz = f0 * bright_partial;
    let bright_decay_seconds = (bright_hz < SAMPLE_RATE_HZ * 0.45)
        .then(|| decay_to_minus_20db(&samples, bright_hz))
        .flatten();

    KeyTimbre {
        expected_h1_seconds,
        late_window_start,
        h1_decay_seconds: decay_to_minus_20db(&samples, f0),
        bright_partial,
        bright_hz,
        bright_decay_seconds,
        early_rms: rms_window(&samples, 0.0, 0.4),
        late_rms: rms_window(&samples, late_window_start, 0.5),
        render_seconds,
    }
}

/// Asserts the radiated-energy and decay checks for one key's measurement.
fn check_energy_and_decay(midi: u8, m: &KeyTimbre) {
    assert!(
        m.early_rms > EARLY_RMS_FLOOR,
        "key {midi}: early RMS {:.2e} — the strike barely made a sound through the engine",
        m.early_rms
    );
    assert!(
        m.late_rms > LATE_RMS_FLOOR,
        "key {midi}: RMS collapsed to {:.2e} by {:.1}s, though its fundamental targets \
         ~{:.1}s to −20 dB — the D10 collapse signature",
        m.late_rms,
        m.late_window_start,
        m.expected_h1_seconds
    );

    match m.h1_decay_seconds {
        Some(measured) => {
            let low = m.expected_h1_seconds / DECAY_BAND;
            let high = m.expected_h1_seconds * DECAY_BAND;
            assert!(
                (low..=high).contains(&measured),
                "key {midi}: fundamental fell 20 dB in {measured:.2}s, outside the \
                 [{low:.2}, {high:.2}]s band around its expected {:.2}s",
                m.expected_h1_seconds
            );
        }
        None => assert!(
            m.render_seconds < m.expected_h1_seconds * DECAY_BAND,
            "key {midi}: fundamental never fell 20 dB in {:.1}s, though it should reach \
             that in ~{:.2}s — ringing far too long",
            m.render_seconds,
            m.expected_h1_seconds
        ),
    }
}

/// Asserts the H1 / H(bright) decay-ratio check, where the bright partial is
/// measurable at all.
fn check_partial_ratio(midi: u8, m: &KeyTimbre) {
    let (Some(h1), Some(bright)) = (m.h1_decay_seconds, m.bright_decay_seconds) else {
        return;
    };
    if m.bright_hz > RATIO_MAX_PARTIAL_HZ || bright <= 0.0 {
        return;
    }
    let ratio = h1 / bright;
    assert!(
        RATIO_BAND.contains(&ratio),
        "key {midi}: H1 {h1:.2}s / H{:.1} {bright:.2}s is a ratio of {ratio:.2}, outside \
         {RATIO_BAND:?} — near 1 means every partial decays together (the metallic D1 defect)",
        m.bright_partial
    );
}

/// The full check for one key: energy, fundamental decay and partial ratio
/// through the production-voiced engine, plus — for a multi-strung key — a
/// monochord control rendered through the same engine with every voice
/// forced to one string.
fn check_key(midi: u8) {
    let mut engine = OfflineEngine::new(rate(), tuning());
    let natural = measure(&mut engine, midi);
    check_energy_and_decay(midi, &natural);
    check_partial_ratio(midi, &natural);

    if unison_count_for_key(key(midi)) < 2 {
        return;
    }
    let mut monochord_engine = OfflineEngine::with_unison_strings(rate(), tuning(), 1);
    let monochord = measure(&mut monochord_engine, midi);
    assert!(
        monochord.late_rms > LATE_RMS_FLOOR,
        "key {midi}: even the monochord control collapsed to {:.2e} — the measurement, \
         not the note, is suspect",
        monochord.late_rms
    );
    let ratio = natural.late_rms / monochord.late_rms;
    assert!(
        UNISON_ENERGY_BAND.contains(&ratio),
        "key {midi}: trichord radiates {ratio:.2}x its own monochord's later-window energy \
         ({:.2e} vs {:.2e}), outside {UNISON_ENERGY_BAND:?} — a trichord that behaves very \
         differently from its monochord is the D10 signature",
        natural.late_rms,
        monochord.late_rms
    );
}

fn rate() -> piano_core::SampleRate {
    piano_core::SampleRate::new(SAMPLE_RATE_HZ).expect("48 kHz is valid")
}

#[test]
fn full_engine_timbre_holds_across_representative_keys() {
    let keys = if cfg!(debug_assertions) {
        DEBUG_KEYS
    } else {
        REPRESENTATIVE_KEYS
    };
    for &midi in keys {
        check_key(midi);
    }
}

#[test]
#[ignore = "88 keys x several seconds of 88-voice synthesis; run with --release --ignored"]
fn full_engine_timbre_holds_across_all_88_keys() {
    for midi in LOWEST_PIANO_KEY..=HIGHEST_PIANO_KEY {
        check_key(midi);
    }
}

/// Not a pass/fail: prints the full-engine measurement next to a bare-string
/// one for a handful of keys, the way `tests/timbre_diagnostic.rs` is meant
/// to be read. Run with `--nocapture`.
#[test]
fn report_full_engine_timbre() {
    println!("\n=== FULL-ENGINE TIMBRE (issue #87) ===");
    println!("key  f0(Hz)   strings  H1 −20dB   H{{bright}} −20dB   early RMS  late RMS");
    for &midi in REPRESENTATIVE_KEYS {
        let mut engine = OfflineEngine::new(rate(), tuning());
        let m = measure(&mut engine, midi);
        let h1 = m.h1_decay_seconds.map_or_else(
            || format!(">{:.1}s", m.render_seconds),
            |s| format!("{s:.2}s"),
        );
        let bright = m
            .bright_decay_seconds
            .map_or_else(|| "  n/a".to_string(), |s| format!("{s:.2}s"));
        println!(
            "{midi:>3}  {:>7.1}  {:>7}  {h1:>8}   H{:.1} {bright:>7}   {:.2e}  {:.2e}",
            fundamental_hz(midi),
            unison_count_for_key(key(midi)),
            m.bright_partial,
            m.early_rms,
            m.late_rms
        );
    }
}
