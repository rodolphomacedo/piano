//! MIDI input for the piano synthesiser: turns messages from a hardware
//! controller — a digital piano plugged in over USB or a MIDI cable — into
//! decoded [`MidiEvent`]s a control thread can turn into
//! `piano_audio::AudioSession` calls.
//!
//! This crate never touches `piano-audio`'s internals and never runs inside
//! its audio callback. The platform's MIDI driver calls back on its own
//! thread ([`midir`]'s), which only ever decodes one message and pushes it
//! into an SPSC ring buffer — the same lock-free pattern as `piano-audio`'s
//! command queue (ADR-0005), reused here because a driver callback thread is
//! just as unsuited to blocking as an audio callback is. [`MidiListener::poll`]
//! drains that queue from whichever thread owns the listener; under queue
//! pressure an event is dropped rather than the driver thread blocking.
//!
//! [`MidiEvent::NoteOff`] and CC64 (the sustain pedal, decoded as an
//! ordinary [`MidiEvent::ControlChange`] — no dedicated variant needed)
//! are both decoded here; the CLI is what turns them into
//! `piano_audio::AudioSession::note_off`/`set_sustain_pedal` calls (M5),
//! since acting on them is `piano-audio`'s job, not this crate's.

use std::thread;
use std::time::Duration;

use midir::{MidiInput, MidiInputPort};
use rtrb::{Consumer, Producer, RingBuffer};

mod error;
mod event;

pub use error::MidiError;
pub use event::MidiEvent;

/// How many decoded events the queue holds before new ones are dropped
/// rather than blocking the driver's callback thread.
const EVENT_QUEUE_CAPACITY: usize = 256;

/// How long [`MidiListener::connect_within`] waits between enumeration
/// attempts. Short enough that switching the instrument on feels like it
/// worked immediately, long enough that a whole `MidiInput` client is not
/// created and torn down in a tight spin.
const PORT_POLL_INTERVAL: Duration = Duration::from_millis(250);

/// A live connection to one MIDI input port.
///
/// Dropping this closes the connection, same as `piano_audio::AudioSession`
/// closes its stream on drop.
pub struct MidiListener {
    // Never read directly: kept alive purely so dropping `MidiListener`
    // closes the underlying connection.
    #[allow(dead_code)]
    connection: midir::MidiInputConnection<Producer<MidiEvent>>,
    consumer: Consumer<MidiEvent>,
    port_name: String,
}

impl MidiListener {
    /// Connects to a MIDI input port.
    ///
    /// `name_filter`, when given, picks the first port whose name contains
    /// it, case-insensitively; when `None`, the first port at all is used —
    /// the common case of one digital piano plugged in.
    ///
    /// # Errors
    ///
    /// Returns [`MidiError`] if the platform's MIDI backend cannot be
    /// initialised, a port's name cannot be read, no port matches, or the
    /// connection itself fails.
    pub fn connect(name_filter: Option<&str>) -> Result<Self, MidiError> {
        let input = MidiInput::new("piano-midi")?;
        let (port, port_name) = select_port(&input, name_filter)?;
        let (producer, consumer) = RingBuffer::new(EVENT_QUEUE_CAPACITY);
        let connection = input
            .connect(&port, "piano-midi-input", on_midi_message, producer)
            .map_err(|error| MidiError::Connect(error.to_string()))?;
        Ok(Self {
            connection,
            consumer,
            port_name,
        })
    }

    /// Connects to a MIDI input port, waiting up to `timeout` for one to
    /// appear before giving up.
    ///
    /// [`MidiListener::connect`] samples the port list exactly once, which
    /// makes it a coin toss whenever the instrument is not already awake:
    /// the player switches the piano on and runs the command, and whichever
    /// of the two wins the race decides whether it works. That is the whole
    /// of the "MIDI stopped working" report this exists to answer — the
    /// enumeration was never wrong, it was just asked once, at the wrong
    /// moment. Waiting turns the race into a wait.
    ///
    /// Only [`MidiError::is_port_absent`] failures are retried; a backend
    /// that refuses to start does not start on the ninth attempt either.
    /// Each attempt builds a fresh [`MidiInput`] rather than re-reading one
    /// long-lived client, because a device that appears *after* a backend
    /// handle was made is not guaranteed to show up in that handle on every
    /// platform.
    ///
    /// # Errors
    ///
    /// Returns the last [`MidiError`] seen: [`MidiError::NoPortsAvailable`]
    /// or [`MidiError::NoMatchingPort`] if nothing suitable ever appeared,
    /// or immediately whatever non-absence error occurred.
    pub fn connect_within(name_filter: Option<&str>, timeout: Duration) -> Result<Self, MidiError> {
        retry_while_absent(
            attempts_for(timeout),
            || Self::connect(name_filter),
            || thread::sleep(PORT_POLL_INTERVAL),
        )
    }

    /// The name of the connected port, for status messages.
    #[must_use]
    pub fn port_name(&self) -> &str {
        &self.port_name
    }

    /// Every input port name currently visible to the system, in the order
    /// [`MidiListener::connect`] would consider them — for `--list`
    /// diagnostics before a port is chosen.
    ///
    /// # Errors
    ///
    /// Returns [`MidiError::BackendInit`] if the backend itself could not be
    /// started.
    pub fn available_ports() -> Result<Vec<String>, MidiError> {
        let input = MidiInput::new("piano-midi")?;
        input
            .ports()
            .iter()
            .map(|port| input.port_name(port).map_err(MidiError::from))
            .collect()
    }

    /// Pops the next queued event, if any, without blocking.
    pub fn poll(&mut self) -> Option<MidiEvent> {
        self.consumer.pop().ok()
    }
}

impl core::fmt::Debug for MidiListener {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("MidiListener")
            .field("port_name", &self.port_name)
            .finish_non_exhaustive()
    }
}

/// Whether a port's name should be selected under `filter`. `None` matches
/// the first port seen — pure and unit-tested independent of any real MIDI
/// backend, unlike [`select_port`] itself.
#[must_use]
fn matches_filter(port_name: &str, filter: Option<&str>) -> bool {
    filter.is_none_or(|filter| port_name.to_lowercase().contains(&filter.to_lowercase()))
}

/// Which of `port_names` [`MidiListener::connect`] would pick — pure, so
/// the rule that decides between "nothing is plugged in" and "the filter
/// matched nothing" is unit-tested without a MIDI backend, which no CI
/// machine has.
fn select_index(port_names: &[String], filter: Option<&str>) -> Result<usize, MidiError> {
    if port_names.is_empty() {
        return Err(MidiError::NoPortsAvailable);
    }
    let Some(filter) = filter else {
        return Ok(0);
    };
    port_names
        .iter()
        .position(|name| matches_filter(name, Some(filter)))
        .ok_or_else(|| MidiError::NoMatchingPort {
            filter: filter.to_owned(),
            available: port_names.to_vec(),
        })
}

/// Reads every port's name, then hands the choice to [`select_index`].
fn select_port(
    input: &MidiInput,
    name_filter: Option<&str>,
) -> Result<(MidiInputPort, String), MidiError> {
    let ports = input.ports();
    let mut names = Vec::with_capacity(ports.len());
    for port in &ports {
        names.push(input.port_name(port)?);
    }
    let index = select_index(&names, name_filter)?;
    let (Some(port), Some(name)) = (ports.get(index), names.get(index)) else {
        // `select_index` only ever returns an index into the slice it was
        // handed, so this is unreachable — expressed as a value rather than
        // an `expect`, which production code may not use.
        return Err(MidiError::NoPortsAvailable);
    };
    Ok((port.clone(), name.clone()))
}

/// How many enumeration attempts fit in `timeout`, never fewer than one, so
/// a zero timeout still probes once: waiting is strictly added to the old
/// behaviour rather than replacing it. Saturating, so an absurd timeout
/// yields `u32::MAX` attempts instead of overflowing.
fn attempts_for(timeout: Duration) -> u32 {
    let interval = PORT_POLL_INTERVAL.as_millis().max(1);
    let attempts = timeout.as_millis() / interval + 1;
    u32::try_from(attempts).unwrap_or(u32::MAX)
}

/// Runs `attempt` until it succeeds, fails for a reason waiting cannot fix,
/// or has run `attempts` times — calling `wait` between tries but never
/// after the last one.
///
/// Generic over both so the retry policy is tested against a fake that
/// counts calls, rather than against real hardware and a real clock.
fn retry_while_absent<T>(
    attempts: u32,
    mut attempt: impl FnMut() -> Result<T, MidiError>,
    mut wait: impl FnMut(),
) -> Result<T, MidiError> {
    let mut last_error = MidiError::NoPortsAvailable;
    for remaining in (0..attempts).rev() {
        match attempt() {
            Ok(value) => return Ok(value),
            Err(error) if !error.is_port_absent() => return Err(error),
            Err(error) => last_error = error,
        }
        if remaining > 0 {
            wait();
        }
    }
    Err(last_error)
}

/// Runs on the MIDI backend's own callback thread. Decodes one message and
/// pushes it into the queue, dropping it rather than blocking if the queue
/// is full — the same drop-not-block choice `piano-audio` makes for
/// commands, and for the same reason: a driver callback thread must return
/// promptly.
fn on_midi_message(_timestamp_micros: u64, message: &[u8], producer: &mut Producer<MidiEvent>) {
    if let Some(midi_event) = event::parse(message) {
        let _ = producer.push(midi_event);
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn no_filter_matches_the_first_port_seen() {
        assert!(matches_filter("Yamaha P-125", None));
        assert!(matches_filter("", None));
    }

    #[test]
    fn a_filter_matches_case_insensitively_by_substring() {
        assert!(matches_filter("Yamaha P-125", Some("yamaha")));
        assert!(matches_filter("Yamaha P-125", Some("P-125")));
        assert!(!matches_filter("Yamaha P-125", Some("Roland")));
    }

    // MidiError must stay Send + Sync on every platform, not just the one
    // this happens to run on: `anyhow::Context` requires it, and a variant
    // that embeds a platform backend handle (as midir's `ConnectError` does,
    // to hand the backend back to the caller) can be `Sync` on one backend
    // and not another. This caught exactly that on Linux/ALSA after passing
    // clean on macOS/CoreMIDI.
    fn assert_send_sync<T: Send + Sync>() {}

    #[test]
    fn midi_error_is_send_and_sync_on_every_platform() {
        assert_send_sync::<MidiError>();
    }

    fn names(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| (*name).to_owned()).collect()
    }

    #[test]
    fn an_empty_port_list_is_an_absent_instrument_not_an_unmatched_filter() {
        // The distinction the "MIDI stopped working" report turned on: the
        // player needs to be told the instrument is missing, not shown an
        // empty list.
        let error = select_index(&names(&[]), None).expect_err("no ports means no choice");
        assert!(matches!(error, MidiError::NoPortsAvailable));

        let error = select_index(&names(&[]), Some("yamaha")).expect_err("still no choice");
        assert!(
            matches!(error, MidiError::NoPortsAvailable),
            "a filter does not turn a missing instrument into a filter problem"
        );
    }

    #[test]
    fn without_a_filter_the_first_port_wins() {
        let ports = names(&["Digital Piano", "IAC Driver Bus 1"]);
        assert_eq!(select_index(&ports, None).expect("a port exists"), 0);
    }

    #[test]
    fn a_filter_picks_the_first_port_whose_name_contains_it() {
        let ports = names(&["IAC Driver Bus 1", "Digital Piano", "Digital Piano 2"]);
        assert_eq!(
            select_index(&ports, Some("digital")).expect("one port matches"),
            1
        );
    }

    #[test]
    fn an_unmatched_filter_reports_the_filter_and_the_ports_that_were_there() {
        let ports = names(&["Digital Piano"]);
        let error = select_index(&ports, Some("roland")).expect_err("nothing matches");
        let MidiError::NoMatchingPort { filter, available } = error else {
            panic!("expected an unmatched filter, got a different failure");
        };
        assert_eq!(filter, "roland");
        assert_eq!(available, ports);
    }

    #[test]
    fn a_zero_timeout_still_probes_once() {
        assert_eq!(attempts_for(Duration::ZERO), 1);
        assert_eq!(attempts_for(Duration::from_millis(1)), 1);
    }

    #[test]
    fn a_timeout_becomes_one_attempt_per_interval_plus_the_immediate_one() {
        assert_eq!(attempts_for(Duration::from_secs(5)), 21);
        assert_eq!(attempts_for(PORT_POLL_INTERVAL), 2);
    }

    #[test]
    fn an_absurd_timeout_saturates_rather_than_overflowing() {
        assert_eq!(attempts_for(Duration::MAX), u32::MAX);
    }

    /// Drives [`retry_while_absent`] with a scripted sequence of outcomes,
    /// counting attempts and waits — no hardware, no sleeping.
    fn run_retry(attempts: u32, script: Vec<Result<&'static str, MidiError>>) -> Retry {
        let mut remaining = script.into_iter();
        let mut tries = 0;
        let mut waits = 0;
        let outcome = retry_while_absent(
            attempts,
            || {
                tries += 1;
                remaining.next().unwrap_or(Err(MidiError::NoPortsAvailable))
            },
            || waits += 1,
        );
        Retry {
            connected: outcome.is_ok(),
            tries,
            waits,
        }
    }

    struct Retry {
        connected: bool,
        tries: u32,
        waits: u32,
    }

    #[test]
    fn an_instrument_already_there_is_never_waited_for() {
        let retry = run_retry(21, vec![Ok("Digital Piano")]);
        assert!(retry.connected);
        assert_eq!(retry.tries, 1);
        assert_eq!(retry.waits, 0, "waited despite connecting immediately");
    }

    #[test]
    fn an_instrument_switched_on_late_is_picked_up_without_a_rerun() {
        // The exact scenario behind the report: the port is not there for
        // the first two enumerations and then appears.
        let retry = run_retry(
            21,
            vec![
                Err(MidiError::NoPortsAvailable),
                Err(MidiError::NoPortsAvailable),
                Ok("Digital Piano"),
            ],
        );
        assert!(retry.connected, "gave up on an instrument that showed up");
        assert_eq!(retry.tries, 3);
        assert_eq!(retry.waits, 2, "one wait between each pair of attempts");
    }

    #[test]
    fn an_instrument_that_never_appears_gives_up_after_the_last_attempt() {
        let retry = run_retry(4, vec![]);
        assert!(!retry.connected);
        assert_eq!(retry.tries, 4);
        assert_eq!(retry.waits, 3, "must not sleep after the final attempt");
    }

    #[test]
    fn a_failure_waiting_cannot_fix_is_not_waited_out() {
        let retry = run_retry(21, vec![Err(MidiError::Connect("port busy".to_owned()))]);
        assert!(!retry.connected);
        assert_eq!(retry.tries, 1, "retried a connection the port refused");
        assert_eq!(retry.waits, 0);
    }

    #[test]
    fn zero_attempts_fails_without_touching_the_backend() {
        let retry = run_retry(0, vec![Ok("Digital Piano")]);
        assert!(!retry.connected);
        assert_eq!(retry.tries, 0);
        assert_eq!(retry.waits, 0);
    }
}
