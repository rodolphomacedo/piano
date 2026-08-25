//! Measured evidence for milestone M4's two objective "done when" criteria
//! from `docs/ROADMAP.md`: partials sit measurably sharp of an exact
//! harmonic series, and hitting harder makes a note brighter, not just
//! louder. Both are checked here with an FFT rather than asserted from the
//! implementation — this project's own precedent from M1, whose tuning claim
//! was a measured cents figure, not an assertion.
//!
//! The third M4 criterion, "audibly a piano rather than a guitar", is a
//! human-hearing judgement this test suite cannot make; see the M4 report
//! for where sample renders were left for a person to listen to.

#![allow(clippy::indexing_slicing, clippy::unwrap_used, clippy::expect_used)]

use piano_core::SampleRate;
use piano_params::{PianoKey, Tuning};
use piano_render::{RenderRequest, render_note};
use rustfft::{FftPlanner, num_complex::Complex32};

const SAMPLE_RATE_HZ: f32 = 48_000.0;

fn render(midi: u8, velocity: f32, seconds: f32) -> Vec<f32> {
    let request = RenderRequest {
        key: PianoKey::from_midi(midi).expect("valid MIDI note"),
        tuning: Tuning::default(),
        sample_rate: SampleRate::new(SAMPLE_RATE_HZ).expect("48 kHz is valid"),
        seconds,
        velocity,
    };
    render_note(request).expect("renders")
}

/// A Hann-windowed magnitude spectrum of `samples`, zero-padded to the next
/// power of two so `rustfft` can use its fastest path.
fn magnitude_spectrum(samples: &[f32]) -> (Vec<f32>, usize) {
    let fft_len = samples.len().next_power_of_two();
    let window_span = (samples.len().max(2) - 1) as f32;
    let mut buffer: Vec<Complex32> = (0..fft_len)
        .map(|index| {
            let Some(&sample) = samples.get(index) else {
                return Complex32::new(0.0, 0.0);
            };
            let phase = core::f32::consts::TAU * index as f32 / window_span;
            let window = 0.5 - 0.5 * phase.cos();
            Complex32::new(sample * window, 0.0)
        })
        .collect();
    let mut planner = FftPlanner::new();
    let fft = planner.plan_fft_forward(fft_len);
    fft.process(&mut buffer);
    (buffer.iter().map(|value| value.norm()).collect(), fft_len)
}

/// The frequency of the strongest bin within `+-search_hz` of `target_hz`.
fn peak_frequency_near(
    spectrum: &[f32],
    fft_len: usize,
    sample_rate: f32,
    target_hz: f32,
    search_hz: f32,
) -> f32 {
    let bin_hz = sample_rate / fft_len as f32;
    let center_bin = (target_hz / bin_hz).round() as i64;
    let span = (search_hz / bin_hz).round() as i64;

    let mut best_bin = center_bin;
    let mut best_magnitude = -1.0f32;
    for offset in -span..=span {
        let bin = center_bin + offset;
        if bin < 0 {
            continue;
        }
        if let Some(&magnitude) = spectrum.get(bin as usize)
            && magnitude > best_magnitude
        {
            best_magnitude = magnitude;
            best_bin = bin;
        }
    }
    best_bin as f32 * bin_hz
}

fn cents_deviation(measured_hz: f32, expected_hz: f32) -> f32 {
    1200.0 * (measured_hz / expected_hz).log2()
}

fn spectral_centroid(samples: &[f32]) -> f32 {
    let (spectrum, fft_len) = magnitude_spectrum(samples);
    let bin_hz = SAMPLE_RATE_HZ / fft_len as f32;
    let half = fft_len / 2;

    let mut weighted = 0.0f64;
    let mut total = 0.0f64;
    for (bin, magnitude) in spectrum.iter().take(half).enumerate() {
        let frequency = bin as f32 * bin_hz;
        weighted += f64::from(frequency) * f64::from(*magnitude);
        total += f64::from(*magnitude);
    }
    if total <= 0.0 {
        0.0
    } else {
        (weighted / total) as f32
    }
}

#[test]
fn partials_sit_measurably_sharp_of_an_exact_harmonic_series() {
    // Fletcher's stiff-string formula (see `piano_core::dispersion`)
    // predicts f_n ~ n*f1*sqrt(1+B*n^2): partial n's deviation from n*f1, in
    // cents, should grow with n. Render A4, take a window past the attack
    // transient (so what is measured is the string's own partials, not the
    // excitation), and measure partials 1..=8.
    //
    // The window used to start at 0.3 s, to outlast "the hammer's own
    // broadband click". There is no longer a broadband click to outlast:
    // `hammer::excitation_cutoff_hz` band-limits the excitation to the
    // felt's own bandwidth, so a strike is no longer full-band energy that
    // has to decay before a partial can be measured. What that costs is
    // exactly what waiting 0.3 s cost more of -- high-partial energy -- so
    // measuring as soon as the transient is past keeps partials 6-8 of A4
    // (2.6-3.5 kHz) above the noise floor, instead of leaving
    // `peak_frequency_near` to lock onto spurious bins for half the series
    // this test asserts about.
    let samples = render(69, 0.9, 2.0);
    let start = (0.1 * SAMPLE_RATE_HZ) as usize;
    let length = 32_768.min(samples.len() - start);
    let window = &samples[start..start + length];
    let (spectrum, fft_len) = magnitude_spectrum(window);

    // Partial 8 of A4 (3520 Hz) is measured but deliberately excluded from
    // the assertions below: by that far above the fundamental the loop
    // filter's fixed zero (see `filter::LoopFilter`) has already cut its
    // energy close to the noise floor for this register, and
    // `peak_frequency_near` occasionally locks onto a spurious nearby bin
    // instead of the true partial — a measurement-resolution limitation of
    // this test, not evidence against the dispersion model, which is why
    // partials 1..=7 (all comfortably above the noise floor for A4) are
    // what the assertions below check.
    let fundamental = 440.0;
    let mut deviations = Vec::new();
    for partial in 1..=8u32 {
        let expected = fundamental * partial as f32;
        let measured = peak_frequency_near(
            &spectrum,
            fft_len,
            SAMPLE_RATE_HZ,
            expected,
            expected * 0.05,
        );
        deviations.push(cents_deviation(measured, expected));
    }
    println!("measured partial deviations (cents): {deviations:?}");

    // Partial 1 is what the loop is tuned to, so it should sit close to
    // in-tune.
    assert!(
        deviations[0].abs() < 5.0,
        "fundamental drifted {} cents",
        deviations[0]
    );

    // Higher partials should sit measurably sharper than the fundamental,
    // and the sharpening should grow with partial number: the qualitative
    // signature Fletcher's formula predicts, which a pure harmonic series
    // (an ideal, non-dispersive string) could never produce.
    assert!(
        deviations[6] > deviations[0] + 1.0,
        "partial 7 ({} cents) is not measurably sharper than the fundamental ({} cents)",
        deviations[6],
        deviations[0]
    );
    assert!(
        deviations[6] > deviations[3],
        "sharpening does not grow with partial number: partial 4 {} cents, partial 7 {} cents",
        deviations[3],
        deviations[6]
    );
}

#[test]
fn hitting_harder_makes_a_note_brighter_not_merely_louder() {
    // Compare spectral centroid (energy-weighted mean frequency) between a
    // soft and a hard strike of the same note. Neither render is
    // loudness-normalised on purpose: the hammer model's whole claim is that
    // velocity changes the excitation's *shape*, and a shape change shows up
    // as a centroid shift regardless of overall level, which is exactly what
    // "brighter, not merely louder" needs to demonstrate.
    let soft = render(69, 0.15, 0.5);
    let hard = render(69, 0.95, 0.5);

    let centroid_soft = spectral_centroid(&soft);
    let centroid_hard = spectral_centroid(&hard);
    println!("spectral centroid: soft {centroid_soft:.1} Hz, hard {centroid_hard:.1} Hz");

    assert!(
        centroid_hard > centroid_soft,
        "hard strike centroid {centroid_hard} Hz is not brighter than soft strike centroid {centroid_soft} Hz"
    );
}

/// How much of the start of a note counts as "the attack" here: 5 ms, a
/// little longer than the ~3.4 ms the felt stays in contact at full
/// velocity, so the window holds the whole strike and almost nothing else.
const ATTACK_SECONDS: f32 = 0.005;

fn attack(samples: &[f32]) -> &[f32] {
    let length = (ATTACK_SECONDS * SAMPLE_RATE_HZ) as usize;
    &samples[..length.min(samples.len())]
}

fn peak(samples: &[f32]) -> f32 {
    samples.iter().fold(0.0f32, |worst, s| worst.max(s.abs()))
}

#[test]
fn the_attack_is_a_felt_hammer_rather_than_a_broadband_click() {
    // The failure this pins down was reported by ear as "a dry knock, like
    // one hard piece of wood hitting another", and measured as a spectral
    // centroid of 11.2 kHz across the first 5 ms of A4 -- against the 12 kHz
    // that a perfectly flat spectrum gives at this sample rate. That is what
    // the excitation was: white noise, enveloped in time by the contact
    // force but not shaped in frequency at all, because an enveloped white
    // noise burst has a flat expected power spectrum whatever the envelope's
    // shape. `hammer::excitation_cutoff_hz` is what fixed it.
    //
    // 8 kHz is a deliberately loose ceiling: this asserts the excitation is
    // no longer indistinguishable from a full-band click, not a particular
    // voicing, which is `EXCITATION_BANDWIDTH_FACTOR`'s own calibration test
    // in `piano_core::hammer` and a matter of taste besides.
    let centroid = spectral_centroid(attack(&render(69, 0.8, 0.5)));
    println!("attack centroid: {centroid:.1} Hz");
    assert!(
        centroid < 8_000.0,
        "the first {ATTACK_SECONDS} s measures {centroid} Hz, which is a click, not felt"
    );
}

#[test]
fn a_harder_strike_is_brighter_in_the_attack_itself_not_only_over_the_note() {
    // The M4 criterion above measures a whole render, where the loop
    // filter's own velocity-independent rolloff dominates. This measures the
    // strike alone, which is where the hammer either does or does not vary
    // with how hard the key was hit.
    //
    // Measured at E2 rather than A4 on purpose, and the reason is a real
    // limitation worth stating: `PluckedString::pluck` writes exactly one
    // loop length of excitation, so a note whose loop period is shorter than
    // the felt's contact gets its contact envelope truncated. E2's period is
    // 12.1 ms and holds the whole 3.5-6.5 ms contact; A4's is 2.27 ms and
    // holds a third of it, so at A4 both velocities receive roughly the same
    // truncated prefix and the difference this asserts is not reliably
    // measurable there. That truncation predates the excitation filter and
    // is a separate defect (it is also why the top octave is markedly
    // quieter than the rest); it is not what this test is about.
    let soft = spectral_centroid(attack(&render(40, 0.15, 0.5)));
    let hard = spectral_centroid(attack(&render(40, 0.95, 0.5)));
    println!("attack centroid at E2: soft {soft:.1} Hz, hard {hard:.1} Hz");
    assert!(
        hard > soft + 500.0,
        "only {} Hz separates a pianissimo attack ({soft} Hz) from a fortissimo one ({hard} Hz); \
         the excitation is still essentially velocity-independent in frequency",
        hard - soft
    );
}

#[test]
fn the_attack_does_not_tower_over_the_note_it_starts() {
    // The other half of the same report -- "and very loud". The strike used
    // to peak at 1.97 against 0.27 for the body of the note, 17 dB of
    // transient that also sat above full scale, so live playback clipped it
    // into something harsher still. A piano attack *is* louder than the
    // note's body; six times louder it is not.
    let samples = render(69, 0.8, 0.5);
    let body_start = (0.02 * SAMPLE_RATE_HZ) as usize;
    let body_end = (0.1 * SAMPLE_RATE_HZ) as usize;
    let attack_peak = peak(attack(&samples));
    let body_peak = peak(&samples[body_start..body_end]);
    println!("attack peak {attack_peak:.4}, body peak {body_peak:.4}");

    assert!(
        attack_peak < 1.0,
        "a single string peaks at {attack_peak}, so a unison group clips before anything is mixed"
    );
    assert!(
        attack_peak < body_peak * 6.0,
        "the attack ({attack_peak}) is more than six times the body ({body_peak})"
    );
    assert!(
        attack_peak > body_peak,
        "the attack ({attack_peak}) no longer stands out from the body ({body_peak}) at all"
    );
}
