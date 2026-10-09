//! `piano analyze` — the timbre measurements for one note, as a table to
//! read while voicing by ear or as JSON for a script (issue #86).

use anyhow::{Context, Result};
use piano_analysis::{NoteRequest, NoteSource, TimbreReport, analyze_note};
use piano_core::SampleRate;
use piano_params::Tuning;

use crate::parse_key;

/// What to render before measuring.
#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub(crate) enum SourceArg {
    /// The whole instrument, as you hear it.
    Engine,
    /// One bare string, nothing mixed on top.
    String,
    /// One string plus the soundboard.
    StringWithSoundboard,
}

impl From<SourceArg> for NoteSource {
    fn from(source: SourceArg) -> Self {
        match source {
            SourceArg::Engine => Self::Engine,
            SourceArg::String => Self::String,
            SourceArg::StringWithSoundboard => Self::StringWithSoundboard,
        }
    }
}

#[derive(Debug, clap::Args)]
pub(crate) struct AnalyzeArgs {
    /// Note to measure, as a name (`A4`, `C#3`) or a MIDI number (`69`).
    #[arg(short, long, default_value = "A4")]
    note: String,

    /// How long to render, in seconds.
    #[arg(short, long, default_value_t = 8.0)]
    seconds: f32,

    /// How hard the key is struck, from 0.0 to 1.0.
    #[arg(short, long, default_value_t = 0.8)]
    velocity: f32,

    /// What to render before measuring.
    #[arg(long, value_enum, default_value_t = SourceArg::Engine)]
    source: SourceArg,

    /// Sample rate, in hertz.
    #[arg(short = 'r', long, default_value_t = 48_000.0)]
    sample_rate: f32,

    /// Frequency of concert A, in hertz.
    #[arg(long, default_value_t = 440.0)]
    concert_a: f32,

    /// Print the report as JSON instead of a table.
    #[arg(long)]
    json: bool,
}

pub(crate) fn run(args: &AnalyzeArgs) -> Result<()> {
    let request = NoteRequest {
        key: parse_key(&args.note)?,
        tuning: Tuning::with_concert_a(args.concert_a)
            .with_context(|| format!("invalid concert A: {}", args.concert_a))?,
        sample_rate: SampleRate::new(args.sample_rate)
            .with_context(|| format!("invalid sample rate: {}", args.sample_rate))?,
        velocity: args.velocity,
        seconds: args.seconds,
    };
    let report = analyze_note(request, args.source.into())
        .with_context(|| format!("could not analyse {}", args.note))?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print!("{}", as_table(&report));
    }
    Ok(())
}

/// The report as the plain-text table a person reads.
fn as_table(report: &TimbreReport) -> String {
    let mut text = format!(
        "{} — f0 {:.2} Hz, B {:.2e}, {:?}, velocity {}, {} s\n",
        report.note,
        report.fundamental_hz,
        report.inharmonicity,
        report.source,
        report.velocity,
        report.seconds
    );
    text.push_str(&decay_section(report));
    text.push_str(&attack_section(report));
    text.push_str(&over_time_section(report));
    text
}

fn decay_section(report: &TimbreReport) -> String {
    let cells = report
        .harmonic_decay
        .iter()
        .map(|decay| match decay.seconds_to_fall_20_db {
            Some(seconds) => format!(" H{}={seconds:.2}s", decay.harmonic),
            None => format!(" H{}=>{:.0}s", decay.harmonic, report.seconds),
        })
        .collect::<Vec<_>>()
        .concat();
    format!("\nSeconds for each partial to fall 20 dB (H8 should die well before H1):\n {cells}\n")
}

fn attack_section(report: &TimbreReport) -> String {
    let cells = report
        .attack_profile
        .iter()
        .map(|level| format!(" H{}={:.0}", level.harmonic, level.db_below_strongest))
        .collect::<Vec<_>>()
        .concat();
    format!("\nAttack: each partial in dB below the strongest:\n {cells}\n")
}

fn over_time_section(report: &TimbreReport) -> String {
    let rows = report
        .over_time
        .iter()
        .map(|at| {
            format!(
                "  t={:>4.1}s  centroid {:>6.0} Hz  level {:>6.1} dB\n",
                at.seconds, at.centroid_hz, at.rms_db
            )
        })
        .collect::<Vec<_>>()
        .concat();
    format!(
        "\nOver time (a real piano's centroid falls as it rings; flat sounds metallic):\n{rows}"
    )
}
