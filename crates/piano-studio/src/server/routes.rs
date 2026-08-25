//! What each route does. See `docs/PARAMETER-STUDIO.md`'s "HTTP /
//! WebSocket API" table for the contract these implement.
//!
//! The page's assets are embedded with `include_str!` rather than read
//! from disk at run time, so a built `piano` binary carries its own studio
//! and there is no "works from the repo, breaks once installed" failure.

use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::live::LiveState;
use crate::server::StudioState;
use crate::server::http::{Request, Response};

/// The control page. See `docs/PARAMETER-STUDIO.md`'s "Browser UI".
const INDEX_HTML: &str = include_str!("../../www/index.html");

/// The page's behaviour.
const APP_JS: &str = include_str!("../../www/app.js");

/// The page's styling.
const STYLE_CSS: &str = include_str!("../../www/style.css");

/// Answers one request. Every unhandled path is a 404 and every unhandled
/// method a 405 — there is no fallthrough that silently succeeds.
pub(crate) fn respond(state: &StudioState, request: &Request) -> Response {
    match (request.method.as_str(), request.path.as_str()) {
        ("GET", "/" | "/index.html") => Response::asset("text/html; charset=utf-8", INDEX_HTML),
        ("GET", "/app.js") => Response::asset("text/javascript; charset=utf-8", APP_JS),
        ("GET", "/style.css") => Response::asset("text/css; charset=utf-8", STYLE_CSS),
        ("GET", "/api/piano") => serve_snapshot(state),
        ("POST", "/api/live") => apply_edit(state, &request.body),
        ("POST", "/api/save") => save(state, &request.body),
        ("POST", "/api/load") => load(state, &request.body),
        ("GET" | "POST", path) => Response::text(404, &format!("no route for {path}")),
        (method, path) => Response::text(405, &format!("{method} is not allowed on {path}")),
    }
}

/// `GET /api/piano` — the full resolved state: every string plus the
/// instrument block.
fn serve_snapshot(state: &StudioState) -> Response {
    let Ok(live) = state.live.lock() else {
        return unavailable();
    };
    match serde_json::to_string(&live.snapshot()) {
        Ok(json) => Response::json(200, json),
        Err(error) => Response::text(500, &format!("could not serialise the piano: {error}")),
    }
}

/// `POST /api/live` — one parameter change, applied to the running engine
/// and echoed to every other connected page.
fn apply_edit(state: &StudioState, body: &str) -> Response {
    let edit = match serde_json::from_str(body) {
        Ok(edit) => edit,
        Err(error) => return Response::text(400, &format!("unreadable edit: {error}")),
    };
    let Ok(mut live) = state.live.lock() else {
        return unavailable();
    };
    let commands = live.apply(&edit);
    drop(live);
    state.send(&commands);
    state.publish(&edit);
    Response::json(200, format!("{{\"applied\":{}}}", commands.len()))
}

/// The body both `POST /api/save` and `POST /api/load` take.
#[derive(Debug, Deserialize)]
struct PathRequest {
    /// Where to write or read. Optional on save, where the file the studio
    /// was started from is the default.
    path: Option<String>,
}

/// `POST /api/save` — resolves the current live state and writes it out in
/// full, per `docs/PARAMETER-STUDIO.md`'s "Persistence" section.
fn save(state: &StudioState, body: &str) -> Response {
    let requested = match parse_path_request(body) {
        Ok(request) => request,
        Err(response) => return *response,
    };
    let Ok(mut live) = state.live.lock() else {
        return unavailable();
    };
    let Some(path) = requested.or_else(|| live.path().map(PathBuf::from)) else {
        return Response::text(
            400,
            "no path to save to: this studio was started without a file",
        );
    };
    if let Err(error) = crate::save(&path, &live.to_piano_file()) {
        return Response::text(400, &format!("{error}"));
    }
    live.set_path(path.clone());
    drop(live);
    let event = saved_event(&path);
    state.publish_raw(&event);
    Response::json(200, event)
}

/// `POST /api/load` — replaces the running instrument's state with a
/// file's, and tells every connected page to re-read it.
fn load(state: &StudioState, body: &str) -> Response {
    let requested = match parse_path_request(body) {
        Ok(request) => request,
        Err(response) => return *response,
    };
    let Some(path) = requested else {
        return Response::text(400, "load needs a path");
    };
    let file = match crate::load(&path) {
        Ok(file) => file,
        Err(error) => return Response::text(400, &format!("{error}")),
    };
    let loaded = LiveState::from_file(&file, Some(&path), state.tuning, state.sample_rate);
    let commands = loaded.commands();
    let Ok(mut live) = state.live.lock() else {
        return unavailable();
    };
    *live = loaded;
    drop(live);
    state.send(&commands);
    state.publish_raw("{\"type\":\"reload\"}");
    Response::json(200, format!("{{\"applied\":{}}}", commands.len()))
}

/// Reads the optional `path` out of a request body, treating an empty body
/// as "no path given" so `POST /api/save` with nothing at all still means
/// "save over what I opened".
fn parse_path_request(body: &str) -> Result<Option<PathBuf>, Box<Response>> {
    if body.trim().is_empty() {
        return Ok(None);
    }
    match serde_json::from_str::<PathRequest>(body) {
        Ok(request) => Ok(request
            .path
            .filter(|path| !path.is_empty())
            .map(PathBuf::from)),
        Err(error) => Err(Box::new(Response::text(
            400,
            &format!("unreadable request: {error}"),
        ))),
    }
}

fn saved_event(path: &Path) -> String {
    let path = path.display().to_string();
    serde_json::json!({ "type": "saved", "path": path }).to_string()
}

/// The one answer a poisoned lock can get. Unreachable in practice —
/// nothing in this crate panics while holding the live state — but
/// answering it beats unwrapping and taking a second thread down with it.
fn unavailable() -> Response {
    Response::text(503, "the studio's live state is unavailable")
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::sync::mpsc::Receiver;

    use piano_core::SampleRate;
    use piano_params::Tuning;

    use super::*;
    use crate::command::StudioCommand;
    use crate::format::PianoFile;

    fn fixture() -> (StudioState, Receiver<StudioCommand>) {
        let sample_rate = SampleRate::new(48_000.0).expect("48 kHz is valid");
        let tuning = Tuning::default();
        let live = LiveState::from_file(&PianoFile::default(), None, tuning, sample_rate);
        StudioState::new(live, tuning, sample_rate)
    }

    fn request(method: &str, path: &str, body: &str) -> Request {
        Request {
            method: method.to_string(),
            path: path.to_string(),
            body: body.to_string(),
        }
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("piano-routes-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir creates");
        dir
    }

    #[test]
    fn the_page_and_its_two_assets_are_all_served() {
        let (state, _commands) = fixture();
        for path in ["/", "/index.html", "/app.js", "/style.css"] {
            let response = respond(&state, &request("GET", path, ""));
            assert_eq!(response.status(), 200, "{path} was not served");
        }
    }

    #[test]
    fn the_snapshot_route_answers_with_json() {
        let (state, _commands) = fixture();
        let response = respond(&state, &request("GET", "/api/piano", ""));
        assert_eq!(response.status(), 200);
    }

    #[test]
    fn an_edit_reaches_the_command_channel() {
        let (state, commands) = fixture();
        let body = r#"{"type":"set_string","midi":69,"string_index":0,
                       "parameter":"damping","value":0.25}"#;
        let response = respond(&state, &request("POST", "/api/live", body));
        assert_eq!(response.status(), 200);
        assert_eq!(
            commands.try_recv().expect("one command was queued"),
            StudioCommand::SetStringDamping {
                midi: 69,
                string_index: 0,
                damping: 0.25,
            }
        );
    }

    #[test]
    fn playing_a_note_reaches_the_command_channel_too() {
        let (state, commands) = fixture();
        let body = r#"{"type":"note_on","midi":69,"velocity":0.8}"#;
        assert_eq!(
            respond(&state, &request("POST", "/api/live", body)).status(),
            200
        );
        assert!(matches!(
            commands.try_recv().expect("one command was queued"),
            StudioCommand::NoteOn { midi: 69, .. }
        ));
    }

    #[test]
    fn an_edit_is_echoed_to_every_other_connected_page() {
        let (state, _commands) = fixture();
        let (_, events) = state.broadcaster.subscribe();
        let body = r#"{"type":"set_bridge","parameter":"local_coupling_gain","value":0.4}"#;
        let _ = respond(&state, &request("POST", "/api/live", body));
        let echoed = events.try_recv().expect("the change was published");
        assert!(echoed.contains("local_coupling_gain"), "{echoed}");
    }

    #[test]
    fn an_unreadable_edit_is_refused_and_queues_nothing() {
        let (state, commands) = fixture();
        let response = respond(&state, &request("POST", "/api/live", "not json"));
        assert_eq!(response.status(), 400);
        assert!(commands.try_recv().is_err());
    }

    #[test]
    fn saving_without_a_path_and_without_a_file_is_refused_rather_than_guessing() {
        let (state, _commands) = fixture();
        let response = respond(&state, &request("POST", "/api/save", ""));
        assert_eq!(response.status(), 400);
    }

    #[test]
    fn saving_then_loading_restores_the_same_instrument() {
        let (state, commands) = fixture();
        let directory = temp_dir("roundtrip");
        let path = directory.join("saved.piano.json");

        let body = r#"{"type":"set_string","midi":69,"string_index":0,
                       "parameter":"damping","value":0.25}"#;
        let _ = respond(&state, &request("POST", "/api/live", body));
        while commands.try_recv().is_ok() {}

        let path_body = format!("{{\"path\":{:?}}}", path.display().to_string());
        assert_eq!(
            respond(&state, &request("POST", "/api/save", &path_body)).status(),
            200
        );
        assert_eq!(
            respond(&state, &request("POST", "/api/load", &path_body)).status(),
            200
        );

        // A load re-sends the whole instrument, so the damping edit has to
        // be somewhere in the flood — that is what proves it survived the
        // trip through the file.
        let queued: Vec<StudioCommand> = std::iter::from_fn(|| commands.try_recv().ok()).collect();
        assert!(queued.contains(&StudioCommand::SetStringDamping {
            midi: 69,
            string_index: 0,
            damping: 0.25,
        }));
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_successful_save_becomes_the_default_for_the_next_one() {
        let (state, _commands) = fixture();
        let directory = temp_dir("saveas");
        let path = directory.join("named.piano.json");
        let body = format!("{{\"path\":{:?}}}", path.display().to_string());
        let _ = respond(&state, &request("POST", "/api/save", &body));
        // A save with no path at all must now land on the same file.
        assert_eq!(
            respond(&state, &request("POST", "/api/save", "")).status(),
            200
        );
        assert!(path.exists());
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn loading_a_file_that_is_not_there_is_refused() {
        let (state, _commands) = fixture();
        let body = r#"{"path":"/nonexistent/nope.piano.json"}"#;
        let response = respond(&state, &request("POST", "/api/load", body));
        assert_eq!(response.status(), 400);
    }

    #[test]
    fn loading_without_a_path_is_refused() {
        let (state, _commands) = fixture();
        assert_eq!(
            respond(&state, &request("POST", "/api/load", "")).status(),
            400
        );
    }

    #[test]
    fn an_unknown_path_is_a_404_and_a_wrong_method_is_a_405() {
        let (state, _commands) = fixture();
        assert_eq!(respond(&state, &request("GET", "/nope", "")).status(), 404);
        assert_eq!(
            respond(&state, &request("DELETE", "/api/piano", "")).status(),
            405
        );
    }
}
