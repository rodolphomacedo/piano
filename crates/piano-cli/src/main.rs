//! `piano` — render synthesised piano notes from the command line.
//!
//! This binary is the thinnest possible shell around [`piano_render`]: parse
//! arguments, call one function, report what happened. Every decision that
//! matters lives in a library crate, where it can be tested without a process.

#![forbid(unsafe_code)]

mod keyboard;
mod play;

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use piano_core::SampleRate;
use piano_params::{PianoKey, Tuning};
use piano_render::{RenderRequest, render_note, write_wav};

use keyboard::KeyboardArgs;
use play::PlayArgs;

/// Physical-modelling piano synthesiser.
#[derive(Debug, Parser)]
#[command(name = "piano", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Render one note to a WAV file.
    Render(RenderArgs),
    /// Play one note live, through the speakers, right now.
    Play(PlayArgs),
    /// Play notes live by typing on the computer keyboard.
    Keyboard(KeyboardArgs),
}

#[derive(Debug, clap::Args)]
struct RenderArgs {
    /// Note to play, as a name (`A4`, `C#3`) or a MIDI number (`69`).
    #[arg(short, long, default_value = "A4")]
    note: String,

    /// How long to record, in seconds.
    #[arg(short, long, default_value_t = 3.0)]
    seconds: f32,

    /// How hard the key is struck, from 0.0 to 1.0.
    #[arg(short, long, default_value_t = 0.8)]
    velocity: f32,

    /// Output sample rate, in hertz.
    #[arg(short = 'r', long, default_value_t = 48_000.0)]
    sample_rate: f32,

    /// Frequency of concert A, in hertz.
    #[arg(long, default_value_t = 440.0)]
    concert_a: f32,

    /// Where to write the WAV file.
    #[arg(short, long, default_value = "note.wav")]
    output: PathBuf,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Render(args) => render(&args),
        Command::Play(args) => play::run(&args),
        Command::Keyboard(args) => keyboard::run(&args),
    }
}

/// Prints the callback timing distribution for a finished [`AudioSession`],
/// shared by every subcommand that opens one.
///
/// [`AudioSession`]: piano_audio::AudioSession
pub(crate) fn report_timing(session: &piano_audio::AudioSession) {
    let report = session.timing_report();
    println!(
        "callback timing (microseconds): p50={} p99={} p99.9={} max={} over {} callbacks",
        report.p50_micros,
        report.p99_micros,
        report.p99_9_micros,
        report.max_micros,
        report.callback_count,
    );
}

fn render(args: &RenderArgs) -> Result<()> {
    let key = parse_key(&args.note)?;
    let tuning = Tuning::with_concert_a(args.concert_a)
        .with_context(|| format!("invalid concert A: {}", args.concert_a))?;
    let sample_rate = SampleRate::new(args.sample_rate)
        .with_context(|| format!("invalid sample rate: {}", args.sample_rate))?;

    let request = RenderRequest {
        key,
        tuning,
        sample_rate,
        seconds: args.seconds,
        velocity: args.velocity,
    };
    let samples = render_note(request).context("could not render the note")?;
    write_wav(&args.output, &samples, sample_rate)
        .with_context(|| format!("could not write {}", args.output.display()))?;

    println!(
        "wrote {} — {} ({:.2} Hz), {:.2} s at {} Hz",
        args.output.display(),
        key.name(),
        key.frequency(tuning).hertz(),
        args.seconds,
        sample_rate.hertz(),
    );
    Ok(())
}

/// Accepts either a note name or a raw MIDI number, so `--note 69` and
/// `--note A4` both work.
pub(crate) fn parse_key(text: &str) -> Result<PianoKey> {
    if let Ok(midi) = text.trim().parse::<u8>() {
        return PianoKey::from_midi(midi).with_context(|| format!("invalid MIDI note: {text}"));
    }
    text.parse::<PianoKey>()
        .with_context(|| format!("invalid note name: {text}"))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use clap::CommandFactory;

    use super::*;

    #[test]
    fn the_command_line_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn accepts_note_names() {
        assert_eq!(parse_key("A4").expect("A4 parses").midi_number(), 69);
        assert_eq!(parse_key(" c#3 ").expect("C#3 parses").midi_number(), 49);
    }

    #[test]
    fn accepts_midi_numbers() {
        assert_eq!(parse_key("69").expect("69 parses").midi_number(), 69);
    }

    #[test]
    fn rejects_keys_off_the_keyboard() {
        assert!(parse_key("200").is_err());
        assert!(parse_key("banana").is_err());
    }
}
