//! `piano play` — play one note live, through the speakers, right now.
//!
//! No file is written. This is the minimal demonstration that
//! [`piano_audio::AudioSession`] works: open the stream, queue one strike,
//! hold it open for a while, then let it drop.

use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};
use piano_audio::AudioSession;
use piano_params::Tuning;

use crate::{parse_key, report_timing};

/// Arguments for `piano play`.
#[derive(Debug, clap::Args)]
pub(crate) struct PlayArgs {
    /// Note to play, as a name (`A4`, `C#3`) or a MIDI number (`69`).
    #[arg(short, long, default_value = "A4")]
    note: String,

    /// How long to hold the stream open, in seconds.
    #[arg(short, long, default_value_t = 4.0)]
    seconds: f32,

    /// How hard the key is struck, from 0.0 to 1.0.
    #[arg(short, long, default_value_t = 0.8)]
    velocity: f32,

    /// Frequency of concert A, in hertz.
    #[arg(long, default_value_t = 440.0)]
    concert_a: f32,

    /// High-frequency loss, 0.0 to 1.0. Higher is duller. Defaults to the
    /// instrument's built-in voicing when omitted.
    #[arg(long)]
    damping: Option<f32>,

    /// Broadband loop gain, 0.0 to 1.0. Higher sustains longer. Defaults to
    /// the instrument's built-in voicing when omitted.
    #[arg(long)]
    sustain: Option<f32>,
}

/// Runs `piano play`.
///
/// # Errors
///
/// Returns an error if the note or concert-A pitch is invalid, or if no
/// audio output device could be opened.
pub(crate) fn run(args: &PlayArgs) -> Result<()> {
    let key = parse_key(&args.note)?;
    let tuning = Tuning::with_concert_a(args.concert_a)
        .with_context(|| format!("invalid concert A: {}", args.concert_a))?;

    let mut session = AudioSession::start(tuning).context("could not start audio playback")?;
    // Voicing commands are queued before the strike, and the queue is FIFO,
    // so the note is struck with the requested voicing already in effect.
    if let Some(damping) = args.damping {
        session.set_damping(damping);
    }
    if let Some(sustain) = args.sustain {
        session.set_sustain(sustain);
    }
    session.note_on(key.midi_number(), args.velocity);
    println!(
        "playing {} ({:.2} Hz) at {} Hz for {:.1}s",
        key.name(),
        key.frequency(tuning).hertz(),
        session.sample_rate().hertz(),
        args.seconds,
    );
    thread::sleep(Duration::from_secs_f32(args.seconds.max(0.0)));

    report_timing(&session);
    Ok(())
}
