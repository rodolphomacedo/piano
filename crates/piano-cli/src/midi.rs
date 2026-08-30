//! `piano midi` — play notes live from a MIDI controller: a digital piano
//! plugged in over USB or a MIDI cable.
//!
//! No file is written. Same real-time queue as `piano keyboard`, just fed by
//! `piano-midi` instead of the computer keyboard.

use std::io;
use std::time::Duration;

use anyhow::{Context, Result};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::terminal;
use piano_audio::AudioSession;
use piano_midi::{MidiEvent, MidiListener};
use piano_params::Tuning;

use crate::report_timing;
use crate::sink::NoteSink;

/// How long `event::poll` waits before re-checking for a reason to stop.
const POLL_INTERVAL: Duration = Duration::from_millis(50);

/// How long to wait for a MIDI port to appear, by default.
///
/// Not zero, because the previous behaviour — enumerate once, fail — turned
/// "switch the piano on, then run the command" into a race the player loses
/// silently. Long enough for a USB instrument to finish announcing itself,
/// short enough that a genuinely absent instrument is reported while the
/// player is still looking at the terminal.
pub(crate) const DEFAULT_WAIT_SECONDS: f32 = 5.0;

/// What to check when no MIDI port is there. Every line names something the
/// player can actually look at; an empty list on its own named nothing.
const PORT_TROUBLESHOOTING: &str = "  - is the instrument powered on, with its cable connected?\n  - is its USB port set to send MIDI, rather than audio only?\n  - on macOS, does it show up in Audio MIDI Setup > MIDI Studio?";

/// MIDI events drained per tick: bounded so a flooded queue cannot make one
/// iteration of the control loop run unboundedly long.
const MAX_EVENTS_PER_TICK: usize = 256;

/// Control-change controller number mapped to the damping knob. CC74 is
/// General MIDI 2's "brightness" — the closest standard controller to this
/// instrument's damping parameter, and the one most controllers expose on a
/// physical knob or slider. Turning it up brightens a real synth, so it is
/// inverted here: `damping = 1 - cc74`.
const DAMPING_CC: u8 = 74;

/// Control-change controller number mapped to the sustain knob. CC1 is the
/// modulation wheel, present on nearly every controller, standing in for a
/// dedicated "sustain amount" control that General MIDI does not define.
///
/// **Not** the sustain *pedal* ([`SUSTAIN_PEDAL_CC`]) — this is
/// `piano_core::PluckedString`'s decay-rate voicing parameter, an unrelated
/// concept that happens to share the word "sustain".
const SUSTAIN_CC: u8 = 1;

/// Control-change controller number of the sustain (hold) pedal — General
/// MIDI's standard CC64, `>=64` is "down", `<64` is "up" (MIDI 1.0
/// Detailed Specification, controller number assignments). Distinct from
/// [`SUSTAIN_CC`]: this is the physical pedal, not a voicing knob.
const SUSTAIN_PEDAL_CC: u8 = 64;

/// The 7-bit MIDI value at which CC64 counts as "pedal down", in the same
/// `[0, 1]`-normalised units `piano-midi` decodes every controller value
/// into: `64 / 127`.
const SUSTAIN_PEDAL_DOWN_THRESHOLD: f32 = 64.0 / 127.0;

/// Arguments for `piano midi`.
#[derive(Debug, clap::Args)]
pub(crate) struct MidiArgs {
    /// Substring of the MIDI input port name to connect to,
    /// case-insensitive. Defaults to the first port found.
    #[arg(short, long)]
    port: Option<String>,

    /// List available MIDI input ports and exit, without opening audio
    /// output. Reports what is there right now; it does not wait.
    #[arg(long)]
    list: bool,

    /// How many seconds to wait for a MIDI port to appear before giving up.
    /// Zero fails immediately if nothing is plugged in.
    #[arg(long, default_value_t = DEFAULT_WAIT_SECONDS)]
    wait: f32,

    /// Frequency of concert A, in hertz.
    #[arg(long, default_value_t = 440.0)]
    concert_a: f32,
}

/// Runs `piano midi` until Esc or Ctrl+C.
///
/// # Errors
///
/// Returns an error if `--list` fails to enumerate ports, if no MIDI port
/// matches, if the concert-A pitch is invalid, if no audio output device
/// could be opened, or if the terminal could not be put into raw mode.
pub(crate) fn run(args: &MidiArgs) -> Result<()> {
    if args.list {
        return list_ports();
    }

    let tuning = Tuning::with_concert_a(args.concert_a)
        .with_context(|| format!("invalid concert A: {}", args.concert_a))?;
    let mut session = AudioSession::start(tuning).context("could not start audio playback")?;
    let mut listener = connect(args.port.as_deref(), args.wait)?;

    print_instructions(listener.port_name());
    let raw_mode = RawModeGuard::enable().context("could not enable terminal raw mode")?;
    let outcome = play_until_quit(&mut session, &mut listener);
    drop(raw_mode);
    outcome?;

    report_timing(&session);
    Ok(())
}

fn list_ports() -> Result<()> {
    let ports = MidiListener::available_ports().context("could not list MIDI input ports")?;
    println!("{}", format_port_list(&ports));
    Ok(())
}

/// What `--list` prints, built as a value rather than a run of `println!`s
/// so that what the player is told when nothing is plugged in is covered by
/// a test instead of only ever seen in a terminal.
pub(crate) fn format_port_list(ports: &[String]) -> String {
    if ports.is_empty() {
        return format!("no MIDI input port found.\n{PORT_TROUBLESHOOTING}");
    }
    let mut listing = String::from("available MIDI input ports:");
    for name in ports {
        listing.push_str("\n  ");
        listing.push_str(name);
    }
    listing
}

/// Opens a MIDI port, waiting `wait_seconds` for one to appear — shared with
/// `crate::studio`, which has exactly the same instrument to find.
pub(crate) fn connect(port: Option<&str>, wait_seconds: f32) -> Result<MidiListener> {
    let wait = wait_duration(wait_seconds);
    if !wait.is_zero() {
        println!(
            "looking for a MIDI input port (waiting up to {:.0}s if none is there yet)...",
            wait.as_secs_f32()
        );
    }
    MidiListener::connect_within(port, wait).context("could not connect to a MIDI input port")
}

/// Turns a `--wait` value in seconds into a duration, treating anything that
/// is not a representable positive number — negative, `NaN`, infinite,
/// beyond `Duration`'s range — as "do not wait".
///
/// Total by construction: `Duration::try_from_secs_f32` rejects exactly
/// those cases, so no value the player can type reaches a panic.
pub(crate) fn wait_duration(seconds: f32) -> Duration {
    Duration::try_from_secs_f32(seconds).unwrap_or(Duration::ZERO)
}

fn play_until_quit(session: &mut AudioSession, listener: &mut MidiListener) -> Result<()> {
    loop {
        drain_midi(session, listener);
        if !event::poll(POLL_INTERVAL)? {
            continue;
        }
        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind != KeyEventKind::Release && should_quit(&key) {
            return Ok(());
        }
    }
}

fn should_quit(key: &KeyEvent) -> bool {
    key.code == KeyCode::Esc
        || (key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL))
}

fn drain_midi(session: &mut AudioSession, listener: &mut MidiListener) {
    for _ in 0..MAX_EVENTS_PER_TICK {
        let Some(midi_event) = listener.poll() else {
            break;
        };
        apply(session, midi_event);
    }
}

/// Plays one decoded MIDI event on `sink`.
///
/// Generic over [`NoteSink`] rather than taking `AudioSession` directly so
/// this mapping is testable without an audio device — see [`crate::sink`].
/// `crate::studio` calls this same function, so both subcommands answer a
/// controller identically by construction, rather than by two match arms
/// being kept in step by hand — which they were not: `studio` used to
/// discard every control change, sustain pedal included.
pub(crate) fn apply(sink: &mut impl NoteSink, event: MidiEvent) {
    match event {
        MidiEvent::NoteOn { note, velocity } => sink.note_on(note, velocity),
        MidiEvent::NoteOff { note } => sink.note_off(note),
        MidiEvent::ControlChange { controller, value } => {
            apply_control_change(sink, controller, value);
        }
    }
}

/// The controller half of [`apply`], split out so both stay under this
/// project's 20-line function limit (`CONTRIBUTING.md`).
fn apply_control_change(sink: &mut impl NoteSink, controller: u8, value: f32) {
    match controller {
        DAMPING_CC => sink.set_damping(1.0 - value),
        SUSTAIN_CC => sink.set_sustain(value),
        SUSTAIN_PEDAL_CC => sink.set_sustain_pedal(value >= SUSTAIN_PEDAL_DOWN_THRESHOLD),
        // Every other control change is not one this instrument understands.
        _ => {}
    }
}

fn print_instructions(port_name: &str) {
    println!("piano midi — connected to \"{port_name}\".");
    println!("play your MIDI controller; nothing is written to disk.");
    println!(
        "CC{DAMPING_CC} (brightness) -> damping, inverted   CC{SUSTAIN_CC} (mod wheel) -> sustain"
    );
    println!(
        "CC{SUSTAIN_PEDAL_CC} (sustain/hold pedal) -> holds every released note until the pedal comes back up"
    );
    println!("note-off releases a key early — release the damper instead of ringing on;");
    println!("play a chord and every held note sounds together.");
    println!("Esc or Ctrl+C (in this terminal) to quit.\n");
}

/// Ensures raw mode is always disabled on the way out, including on error —
/// otherwise an early return would leave the user's terminal broken.
struct RawModeGuard;

impl RawModeGuard {
    fn enable() -> io::Result<Self> {
        terminal::enable_raw_mode()?;
        Ok(Self)
    }
}

impl Drop for RawModeGuard {
    fn drop(&mut self) {
        let _ = terminal::disable_raw_mode();
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use crate::sink::recorder::{Played, Recorder};

    fn names(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| (*name).to_owned()).collect()
    }

    /// Feeds `messages` in as raw MIDI bytes — exactly what a controller
    /// puts on the wire — and returns every call they produced on the
    /// instrument. Decoding is `piano_midi`'s real decoder, not a stub, so
    /// this covers the whole chain from wire format to engine call with only
    /// the audio device replaced.
    fn play_bytes(messages: &[&[u8]]) -> Vec<Played> {
        let mut recorder = Recorder::default();
        for message in messages {
            if let Some(event) = MidiEvent::decode(message) {
                apply(&mut recorder, event);
            }
        }
        recorder.played
    }

    /// Middle C, struck hard: status `0x90` (note-on, channel 1), note 60,
    /// velocity 100.
    const MIDDLE_C_ON: &[u8] = &[0x90, 60, 100];
    /// Middle C released: status `0x80`, note 60, release velocity 64.
    const MIDDLE_C_OFF: &[u8] = &[0x80, 60, 64];

    #[test]
    fn pressing_a_key_on_the_controller_strikes_that_note() {
        assert_eq!(
            play_bytes(&[MIDDLE_C_ON]),
            vec![Played::NoteOn {
                midi: 60,
                velocity: 100.0 / 127.0
            }]
        );
    }

    #[test]
    fn releasing_a_key_on_the_controller_releases_that_note() {
        assert_eq!(
            play_bytes(&[MIDDLE_C_ON, MIDDLE_C_OFF]),
            vec![
                Played::NoteOn {
                    midi: 60,
                    velocity: 100.0 / 127.0
                },
                Played::NoteOff { midi: 60 },
            ]
        );
    }

    #[test]
    fn a_note_on_at_zero_velocity_releases_rather_than_striking_silently() {
        // The running-status idiom: many controllers never send `0x80` at
        // all. Reading it as a strike would leave every key held forever.
        assert_eq!(
            play_bytes(&[MIDDLE_C_ON, &[0x90, 60, 0]]),
            vec![
                Played::NoteOn {
                    midi: 60,
                    velocity: 100.0 / 127.0
                },
                Played::NoteOff { midi: 60 },
            ]
        );
    }

    #[test]
    fn a_chord_strikes_every_note_in_it() {
        let played = play_bytes(&[&[0x90, 60, 90], &[0x90, 64, 90], &[0x90, 67, 90]]);
        let struck: Vec<u8> = played
            .iter()
            .filter_map(|call| match call {
                Played::NoteOn { midi, .. } => Some(*midi),
                _ => None,
            })
            .collect();
        assert_eq!(struck, vec![60, 64, 67], "a C major triad lost a note");
    }

    #[test]
    fn a_controller_on_any_channel_is_heard() {
        // A digital piano set to channel 5 must not be silently ignored:
        // only the status byte's high nibble names the message type.
        for channel in 0..16u8 {
            let played = play_bytes(&[&[0x90 | channel, 60, 100]]);
            assert!(
                matches!(played.as_slice(), [Played::NoteOn { midi: 60, .. }]),
                "channel {channel} was ignored"
            );
        }
    }

    /// The regression test for the defect this change fixes. CC64 is the
    /// hold pedal; `>= 64` is down, `< 64` is up (MIDI 1.0 Detailed
    /// Specification). `piano studio --midi` reached a `match` that dropped
    /// every control change, so the pedal did nothing there while working
    /// under `piano midi`. Both now go through this one function.
    #[test]
    fn the_sustain_pedal_goes_down_and_comes_back_up() {
        assert_eq!(
            play_bytes(&[&[0xB0, 64, 127], &[0xB0, 64, 0]]),
            vec![
                Played::SustainPedal { down: true },
                Played::SustainPedal { down: false },
            ]
        );
    }

    #[test]
    fn the_pedal_threshold_sits_where_the_midi_specification_puts_it() {
        // 63 is up, 64 is down — a half-pedal controller sweeping through
        // the middle must switch at exactly one place, and the right one.
        assert_eq!(
            play_bytes(&[&[0xB0, 64, 63]]),
            vec![Played::SustainPedal { down: false }]
        );
        assert_eq!(
            play_bytes(&[&[0xB0, 64, 64]]),
            vec![Played::SustainPedal { down: true }]
        );
    }

    #[test]
    fn the_brightness_knob_is_inverted_into_damping() {
        // CC74 up means brighter, and brighter means *less* damping.
        assert_eq!(
            play_bytes(&[&[0xB0, 74, 127]]),
            vec![Played::Damping { damping: 0.0 }]
        );
        assert_eq!(
            play_bytes(&[&[0xB0, 74, 0]]),
            vec![Played::Damping { damping: 1.0 }]
        );
    }

    #[test]
    fn the_modulation_wheel_sets_sustain() {
        assert_eq!(
            play_bytes(&[&[0xB0, 1, 127]]),
            vec![Played::Sustain { sustain: 1.0 }]
        );
    }

    #[test]
    fn the_pedal_and_the_sustain_knob_are_not_the_same_control() {
        // They share the word "sustain" and nothing else: CC64 is the
        // physical hold pedal, CC1 is a decay-rate voicing parameter.
        // Wiring one to the other would be silent and badly wrong.
        assert_eq!(
            play_bytes(&[&[0xB0, 64, 127], &[0xB0, 1, 127]]),
            vec![
                Played::SustainPedal { down: true },
                Played::Sustain { sustain: 1.0 },
            ]
        );
    }

    #[test]
    fn a_control_this_instrument_does_not_map_changes_nothing() {
        // CC7 is volume, CC10 is pan: real controllers send them, and
        // acting on them would be worse than ignoring them.
        assert!(play_bytes(&[&[0xB0, 7, 100], &[0xB0, 10, 64]]).is_empty());
    }

    #[test]
    fn messages_this_synthesiser_does_not_act_on_are_dropped_not_misread() {
        // Pitch bend, program change, aftertouch and clock arrive
        // constantly from a real instrument. Misreading any of them as a
        // note is how a controller starts playing notes nobody pressed.
        let played = play_bytes(&[
            &[0xE0, 0, 64],  // pitch bend
            &[0xC0, 5],      // program change
            &[0xD0, 64],     // channel aftertouch
            &[0xA0, 60, 64], // polyphonic aftertouch
            &[0xF8],         // timing clock
            &[0xFE],         // active sensing
            &[],             // an empty read
        ]);
        assert!(
            played.is_empty(),
            "a non-note message reached the engine: {played:?}"
        );
    }

    #[test]
    fn a_truncated_message_never_panics() {
        // A cable pulled mid-message, or a driver handing over a partial
        // buffer, must not take the process down: this runs off a driver
        // callback thread.
        for message in [
            [0x90].as_slice(),
            &[0x90, 60],
            &[0xB0],
            &[0xB0, 64],
            &[0x80, 60],
        ] {
            let _ = play_bytes(&[message]);
        }
    }

    #[test]
    fn every_key_of_an_88_key_piano_reaches_the_engine() {
        // A0 is MIDI 21 and C8 is 108. A controller sending the top or
        // bottom of its range must not be filtered out before the engine
        // gets a say about which notes it can voice.
        for note in 21..=108u8 {
            let played = play_bytes(&[&[0x90, note, 64]]);
            assert!(
                matches!(played.as_slice(), [Played::NoteOn { midi, .. }] if *midi == note),
                "MIDI note {note} did not reach the instrument"
            );
        }
    }

    #[test]
    fn every_velocity_arrives_inside_the_unit_range_and_in_order() {
        // `pluck` clamps, but a velocity that arrives outside `[0, 1]` — or
        // that does not rise with the player's touch — is a bug here, not
        // there.
        let mut previous = 0.0f32;
        for velocity in 1..=127u8 {
            let played = play_bytes(&[&[0x90, 60, velocity]]);
            let [Played::NoteOn { velocity: sent, .. }] = played.as_slice() else {
                panic!("velocity {velocity} did not produce a strike");
            };
            assert!(
                (0.0..=1.0).contains(sent),
                "velocity {velocity} normalised to {sent}"
            );
            assert!(*sent > previous, "velocity {velocity} did not rise");
            previous = *sent;
        }
        assert!(
            (previous - 1.0).abs() < f32::EPSILON,
            "full velocity normalised to {previous}, not 1.0"
        );
    }

    #[test]
    fn an_empty_listing_says_what_to_check() {
        // The report this fixes was three runs of `--list` printing "no
        // MIDI input ports found" and nothing else, which left the player
        // with no next move.
        let listing = format_port_list(&names(&[]));
        assert!(listing.contains("powered on"), "unhelpful: {listing}");
        assert!(listing.contains("audio only"), "unhelpful: {listing}");
        assert!(
            listing.contains("Audio MIDI Setup"),
            "no way to confirm the system sees it: {listing}"
        );
    }

    #[test]
    fn a_listing_shows_every_port_one_per_line() {
        let listing = format_port_list(&names(&["Digital Piano", "IAC Driver Bus 1"]));
        assert_eq!(
            listing,
            "available MIDI input ports:\n  Digital Piano\n  IAC Driver Bus 1"
        );
    }

    #[test]
    fn the_default_wait_gives_a_usb_instrument_time_to_announce_itself() {
        assert_eq!(
            wait_duration(DEFAULT_WAIT_SECONDS),
            Duration::from_secs_f32(5.0)
        );
    }

    #[test]
    fn zero_seconds_keeps_the_old_fail_immediately_behaviour() {
        assert_eq!(wait_duration(0.0), Duration::ZERO);
    }

    #[test]
    fn a_wait_that_is_not_a_positive_number_is_no_wait_at_all() {
        for seconds in [-1.0, f32::NAN, f32::INFINITY, f32::NEG_INFINITY, f32::MAX] {
            assert_eq!(
                wait_duration(seconds),
                Duration::ZERO,
                "--wait {seconds} should not wait, and must never panic"
            );
        }
    }

    #[test]
    fn a_fractional_wait_is_kept_rather_than_rounded_away() {
        assert_eq!(wait_duration(0.5), Duration::from_millis(500));
    }
}
