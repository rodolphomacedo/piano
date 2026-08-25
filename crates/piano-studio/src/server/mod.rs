//! The local control surface: a web server bound to `127.0.0.1`.
//!
//! `docs/PARAMETER-STUDIO.md`'s "Remote or multi-user access" non-goal is
//! the security model in full — this listens on the loopback interface
//! only, and there is no authentication because there is nothing to
//! authenticate against.
//!
//! # Threads, and what never crosses them
//!
//! ```text
//! browser ──HTTP──▶ connection thread ──▶ StudioState ──mpsc──▶ the thread
//!                                             │                that owns the
//!                     SSE ◀───────────────────┘                AudioSession
//! ```
//!
//! Connection threads never touch the audio session and never touch the
//! lock-free command ring: they push [`StudioCommand`]s onto an ordinary
//! channel, and the single thread that owns the session drains it and does
//! the pushing. ADR-0005's one-producer rule for that ring is preserved
//! exactly.
//!
//! The `Mutex` around the live state is a *control-thread* lock. The audio
//! thread never sees it, so `docs/REALTIME-AUDIO-RULES.md`'s "locks
//! nothing" rule is untouched.

mod http;
mod routes;
mod sse;

use std::io::Write;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use piano_core::SampleRate;
use piano_params::Tuning;

use crate::command::StudioCommand;
use crate::edit::Edit;
use crate::error::StudioError;
use crate::live::LiveState;

/// Connections served at once. A browser opens a handful — one page, one
/// event stream, a few in flight — so this leaves generous room for
/// several tabs while bounding how many threads one client can make this
/// process spawn.
const MAX_CONNECTIONS: usize = 32;

/// How long a kept-alive connection may sit silent before it is closed.
const IDLE_TIMEOUT: Duration = Duration::from_secs(120);

/// How long a write may block before the client is treated as gone. Bounds
/// what one wedged browser tab can do to a connection thread.
const WRITE_TIMEOUT: Duration = Duration::from_secs(10);

/// How often an idle event stream sends a comment frame, so a client that
/// vanished without closing gets noticed.
const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(15);

/// What the studio needs to know beyond the instrument's own state.
#[derive(Debug, Clone, Copy)]
pub struct StudioConfig {
    /// TCP port to listen on. `0` asks the OS for a free one, which is
    /// what the tests use.
    pub port: u16,
    /// The tuning a reloaded file is resolved against.
    pub tuning: Tuning,
    /// The rate the engine actually opened at. Resolving against the wrong
    /// one silently mistunes every string's decay — see [`crate::resolve`].
    pub sample_rate: SampleRate,
}

/// Everything a connection thread can reach.
#[derive(Debug)]
pub(crate) struct StudioState {
    /// The authoritative picture of the running instrument.
    pub(crate) live: Mutex<LiveState>,
    /// Where applied changes go on their way to the audio session.
    commands: Mutex<Sender<StudioCommand>>,
    /// Every connected page.
    pub(crate) broadcaster: sse::Broadcaster,
    /// See [`StudioConfig::tuning`].
    pub(crate) tuning: Tuning,
    /// See [`StudioConfig::sample_rate`].
    pub(crate) sample_rate: SampleRate,
}

impl StudioState {
    /// Builds the shared state and the channel carrying commands out to
    /// whoever owns the audio session.
    pub(crate) fn new(
        live: LiveState,
        tuning: Tuning,
        sample_rate: SampleRate,
    ) -> (Self, Receiver<StudioCommand>) {
        let (sender, receiver) = channel();
        let state = Self {
            live: Mutex::new(live),
            commands: Mutex::new(sender),
            broadcaster: sse::Broadcaster::default(),
            tuning,
            sample_rate,
        };
        (state, receiver)
    }

    /// Queues `commands` for the audio session's own thread.
    ///
    /// A closed channel means that thread has stopped — the studio is
    /// shutting down — so there is nothing left to do and nothing to
    /// report.
    pub(crate) fn send(&self, commands: &[StudioCommand]) {
        let Ok(sender) = self.commands.lock() else {
            return;
        };
        for command in commands {
            if sender.send(*command).is_err() {
                return;
            }
        }
    }

    /// Echoes an applied change to every other connected page.
    pub(crate) fn publish(&self, edit: &Edit) {
        match serde_json::to_string(edit) {
            Ok(json) => self.broadcaster.broadcast(&json),
            Err(error) => eprintln!("studio: could not publish a change: {error}"),
        }
    }

    /// Publishes an already-serialised event, for the two the studio
    /// generates itself rather than echoes: `reload` and `saved`.
    pub(crate) fn publish_raw(&self, message: &str) {
        self.broadcaster.broadcast(message);
    }
}

/// A running studio server. Dropping this does not stop it: the listener
/// lives on a detached thread for the lifetime of the process, which is
/// exactly as long as the audio session it is driving.
#[derive(Debug, Clone)]
pub struct StudioServer {
    address: SocketAddr,
}

impl StudioServer {
    /// The address to point a browser at.
    #[must_use]
    pub fn url(&self) -> String {
        format!("http://{}", self.address)
    }

    /// The address actually bound, which matters when [`StudioConfig::port`]
    /// was `0` and the OS chose.
    #[must_use]
    pub fn address(&self) -> SocketAddr {
        self.address
    }
}

/// Starts the studio's web server on the loopback interface.
///
/// Returns the running server and the channel every change arrives on. The
/// caller owns the audio session and is responsible for draining that
/// channel — see `piano-cli`'s `studio` subcommand for the pacing that
/// keeps a flood of changes from overrunning the command ring.
///
/// # Errors
///
/// Returns [`StudioError::Bind`] if the port cannot be bound, or if the
/// accept thread cannot be started.
pub fn serve(
    live: LiveState,
    config: StudioConfig,
) -> Result<(StudioServer, Receiver<StudioCommand>), StudioError> {
    let requested = SocketAddr::from(([127, 0, 0, 1], config.port));
    let listener = TcpListener::bind(requested).map_err(|source| StudioError::Bind {
        address: requested,
        source,
    })?;
    let address = listener.local_addr().unwrap_or(requested);
    let (state, commands) = StudioState::new(live, config.tuning, config.sample_rate);
    spawn_accept_loop(listener, address, Arc::new(state))?;
    Ok((StudioServer { address }, commands))
}

/// Puts the accept loop on its own thread, so [`serve`] returns to a
/// caller that still has an instrument to play.
fn spawn_accept_loop(
    listener: TcpListener,
    address: SocketAddr,
    state: Arc<StudioState>,
) -> Result<(), StudioError> {
    std::thread::Builder::new()
        .name("piano-studio-accept".to_string())
        .spawn(move || accept_loop(&listener, &state))
        .map(|_handle| ())
        .map_err(|source| StudioError::Bind { address, source })
}

/// Accepts connections until the listener fails, which on a loopback
/// socket means the process is going away.
fn accept_loop(listener: &TcpListener, state: &Arc<StudioState>) {
    let live_connections = Arc::new(AtomicUsize::new(0));
    for incoming in listener.incoming() {
        match incoming {
            Ok(stream) => dispatch(stream, state, &live_connections),
            Err(error) => eprintln!("studio: could not accept a connection: {error}"),
        }
    }
}

/// Hands one accepted connection to its own thread, or turns it away when
/// too many are already open.
fn dispatch(mut stream: TcpStream, state: &Arc<StudioState>, live: &Arc<AtomicUsize>) {
    if live.fetch_add(1, Ordering::SeqCst) >= MAX_CONNECTIONS {
        live.fetch_sub(1, Ordering::SeqCst);
        let _ = http::Response::text(503, "too many open connections").write_to(&mut stream);
        return;
    }
    let state = Arc::clone(state);
    let counter = Arc::clone(live);
    let spawned = std::thread::Builder::new()
        .name("piano-studio-connection".to_string())
        .spawn(move || {
            serve_connection(&state, stream);
            counter.fetch_sub(1, Ordering::SeqCst);
        });
    if let Err(error) = spawned {
        live.fetch_sub(1, Ordering::SeqCst);
        eprintln!("studio: could not start a connection thread: {error}");
    }
}

/// Serves requests on one connection until the client goes away, sends
/// something unreadable, or upgrades to the event stream.
fn serve_connection(state: &StudioState, mut stream: TcpStream) {
    let _ = stream.set_nodelay(true);
    let _ = stream.set_read_timeout(Some(IDLE_TIMEOUT));
    let _ = stream.set_write_timeout(Some(WRITE_TIMEOUT));
    let Ok(peer) = stream.try_clone() else {
        eprintln!("studio: could not read from an accepted connection");
        return;
    };
    let mut reader = std::io::BufReader::new(peer);
    while handle_one_request(state, &mut reader, &mut stream) {}
}

/// Answers one request. Returns whether the connection is still usable —
/// the event stream never comes back, so it answers `false` on its way
/// out.
fn handle_one_request(
    state: &StudioState,
    reader: &mut impl std::io::BufRead,
    stream: &mut TcpStream,
) -> bool {
    let request = match http::read_request(reader) {
        Ok(Some(request)) => request,
        Ok(None) => return false,
        Err(error) => {
            let _ = http::Response::text(400, &format!("{error}")).write_to(stream);
            return false;
        }
    };
    if request.method == "GET" && request.path == "/api/live" {
        stream_events(state, stream);
        return false;
    }
    routes::respond(state, &request).write_to(stream).is_ok()
}

/// Holds one connection open and writes every published change to it, per
/// [`sse`]'s module docs.
fn stream_events(state: &StudioState, stream: &mut TcpStream) {
    if stream.write_all(sse::EVENT_STREAM_HEAD.as_bytes()).is_err() {
        return;
    }
    let (id, events) = state.broadcaster.subscribe();
    loop {
        let frame = match events.recv_timeout(KEEPALIVE_INTERVAL) {
            Ok(message) => sse::data_frame(&message),
            Err(RecvTimeoutError::Timeout) => sse::KEEPALIVE_FRAME.to_string(),
            Err(RecvTimeoutError::Disconnected) => break,
        };
        if stream.write_all(frame.as_bytes()).is_err() || stream.flush().is_err() {
            break;
        }
    }
    state.broadcaster.unsubscribe(id);
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use std::io::{BufRead, BufReader, Read, Write};

    use crate::format::PianoFile;

    use super::*;

    fn start() -> (StudioServer, Receiver<StudioCommand>) {
        let sample_rate = SampleRate::new(48_000.0).expect("48 kHz is valid");
        let tuning = Tuning::default();
        let live = LiveState::from_file(&PianoFile::default(), None, tuning, sample_rate);
        serve(
            live,
            StudioConfig {
                port: 0,
                tuning,
                sample_rate,
            },
        )
        .expect("an ephemeral port binds")
    }

    /// Sends one request and returns the whole response, headers included.
    fn round_trip(server: &StudioServer, request: &str) -> String {
        let mut stream = TcpStream::connect(server.address()).expect("connects");
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .expect("timeout is settable");
        stream.write_all(request.as_bytes()).expect("writes");
        let mut reader = BufReader::new(stream);
        let head = read_head(&mut reader);
        let length: usize = head
            .lines()
            .find_map(|line| line.strip_prefix("Content-Length: "))
            .and_then(|value| value.trim().parse().ok())
            .unwrap_or(0);
        let mut body = vec![0u8; length];
        reader.read_exact(&mut body).expect("reads the body");
        format!("{head}\r\n{}", String::from_utf8_lossy(&body))
    }

    fn read_head(reader: &mut impl BufRead) -> String {
        let mut head = String::new();
        loop {
            let mut line = String::new();
            let read = reader.read_line(&mut line).expect("reads");
            if read == 0 || line == "\r\n" {
                return head;
            }
            head.push_str(&line);
        }
    }

    #[test]
    fn the_server_binds_to_loopback_only() {
        let (server, _commands) = start();
        assert!(server.address().ip().is_loopback());
        assert!(server.url().starts_with("http://127.0.0.1:"));
    }

    #[test]
    fn the_control_page_is_served_over_a_real_socket() {
        let (server, _commands) = start();
        let response = round_trip(&server, "GET / HTTP/1.1\r\nHost: localhost\r\n\r\n");
        assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");
        assert!(response.contains("<!doctype html>"), "{response}");
    }

    #[test]
    fn the_snapshot_route_serves_all_88_keys_over_a_real_socket() {
        let (server, _commands) = start();
        let response = round_trip(&server, "GET /api/piano HTTP/1.1\r\nHost: x\r\n\r\n");
        assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");
        let body = response.split("\r\n\r\n").nth(1).expect("has a body");
        let snapshot: serde_json::Value = serde_json::from_str(body).expect("is JSON");
        assert_eq!(
            snapshot
                .get("keys")
                .expect("snapshot has keys")
                .as_array()
                .expect("keys is a list")
                .len(),
            88
        );
    }

    #[test]
    fn an_edit_posted_over_a_real_socket_reaches_the_command_channel() {
        let (server, commands) = start();
        let body = r#"{"type":"note_on","midi":69,"velocity":0.8}"#;
        let request = format!(
            "POST /api/live HTTP/1.1\r\nHost: x\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        );
        let response = round_trip(&server, &request);
        assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");
        assert!(matches!(
            commands.recv_timeout(Duration::from_secs(5)),
            Ok(StudioCommand::NoteOn { midi: 69, .. })
        ));
    }

    #[test]
    fn a_connection_survives_more_than_one_request() {
        // Every slider move is its own request; reopening a socket for
        // each would work, but would churn one connection per pixel.
        let (server, _commands) = start();
        let mut stream = TcpStream::connect(server.address()).expect("connects");
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .expect("timeout is settable");
        for _ in 0..2 {
            stream
                .write_all(b"GET /style.css HTTP/1.1\r\nHost: x\r\n\r\n")
                .expect("writes");
            let mut buffer = [0u8; 16];
            let read = stream.read(&mut buffer).expect("reads");
            assert!(read > 0, "the connection was closed after one request");
        }
    }

    #[test]
    fn an_event_stream_receives_a_change_made_by_another_client() {
        let (server, _commands) = start();
        let mut listener = TcpStream::connect(server.address()).expect("connects");
        listener
            .set_read_timeout(Some(Duration::from_secs(10)))
            .expect("timeout is settable");
        listener
            .write_all(b"GET /api/live HTTP/1.1\r\nHost: x\r\n\r\n")
            .expect("writes");
        let mut reader = BufReader::new(listener);
        let _ = read_head(&mut reader);

        let body = r#"{"type":"set_bridge","parameter":"global_coupling_gain","value":0.2}"#;
        let request = format!(
            "POST /api/live HTTP/1.1\r\nHost: x\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        );
        let _ = round_trip(&server, &request);

        let mut seen = String::new();
        for _ in 0..40 {
            let mut line = String::new();
            if reader.read_line(&mut line).expect("reads") == 0 {
                break;
            }
            seen.push_str(&line);
            if seen.contains("global_coupling_gain") {
                return;
            }
        }
        panic!("the change never arrived on the event stream:\n{seen}");
    }

    #[test]
    fn a_malformed_request_is_refused_rather_than_hanging_the_connection() {
        let (server, _commands) = start();
        let response = round_trip(
            &server,
            "POST /api/live HTTP/1.1\r\nContent-Length: banana\r\n\r\n",
        );
        assert!(response.starts_with("HTTP/1.1 400"), "{response}");
    }
}
