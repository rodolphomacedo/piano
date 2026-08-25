//! Server-sent events: how a change made in one tab reaches every other.
//!
//! `docs/PARAMETER-STUDIO.md` asks `/api/live` to be bidirectional, "so a
//! second tab or a tablet on the piano's music stand stays in sync". That
//! is two directions, not necessarily one socket: `POST /api/live` carries
//! a change up, and this carries the applied result back down to everyone.
//! The design document named a WebSocket for the job; server-sent events
//! do the same work without a frame codec, a handshake hash or a
//! dependency, and the deviation is recorded in that document.
//!
//! A subscriber is a channel, never a socket. The thread serving one
//! client owns that client's connection and pulls from its own receiver,
//! so a client that has stopped reading can never stall the thread that
//! made the edit — its channel fills and its connection is dropped.

use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};

/// One connected page.
#[derive(Debug)]
struct Subscriber {
    id: u64,
    events: Sender<String>,
}

/// The set of connected pages, and the means to reach all of them.
#[derive(Debug, Default)]
pub(crate) struct Broadcaster {
    subscribers: Mutex<Vec<Subscriber>>,
    next_id: AtomicU64,
}

impl Broadcaster {
    /// Registers a new page and hands back its own event stream, plus the
    /// identifier needed to [`Broadcaster::unsubscribe`] later.
    ///
    /// A poisoned lock — reachable only if another thread panicked while
    /// holding it, which nothing here does — yields a receiver that simply
    /// never fires, rather than propagating a panic into a second thread.
    pub(crate) fn subscribe(&self) -> (u64, Receiver<String>) {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (events, receiver) = channel();
        if let Ok(mut subscribers) = self.subscribers.lock() {
            subscribers.push(Subscriber { id, events });
        }
        (id, receiver)
    }

    /// Forgets the page registered under `id`.
    pub(crate) fn unsubscribe(&self, id: u64) {
        if let Ok(mut subscribers) = self.subscribers.lock() {
            subscribers.retain(|subscriber| subscriber.id != id);
        }
    }

    /// Sends `message` to every connected page, dropping any whose
    /// receiving thread has already gone.
    pub(crate) fn broadcast(&self, message: &str) {
        let Ok(mut subscribers) = self.subscribers.lock() else {
            return;
        };
        subscribers.retain(|subscriber| subscriber.events.send(message.to_string()).is_ok());
    }

    /// How many pages are currently connected. Only tests observe this —
    /// production code reacts to subscribers, it never counts them.
    #[cfg(test)]
    pub(crate) fn subscriber_count(&self) -> usize {
        self.subscribers
            .lock()
            .map_or(0, |subscribers| subscribers.len())
    }
}

/// The `text/event-stream` response head, written once before any event.
pub(crate) const EVENT_STREAM_HEAD: &str = "HTTP/1.1 200 OK\r\n\
     Content-Type: text/event-stream\r\n\
     Cache-Control: no-store\r\n\
     Connection: keep-alive\r\n\r\n";

/// A comment frame, sent when nothing has happened for a while. It costs
/// one line and is how a dead connection gets noticed at all: with no
/// traffic a write never fails, and the serving thread would wait forever.
pub(crate) const KEEPALIVE_FRAME: &str = ": keepalive\n\n";

/// Wraps one JSON message as an SSE `data:` frame.
///
/// A newline inside `message` would otherwise split it into two events, so
/// every line gets its own prefix. `serde_json`'s compact output never
/// contains one, but nothing about this function should depend on that.
pub(crate) fn data_frame(message: &str) -> String {
    let mut frame = String::with_capacity(message.len() + 8);
    for line in message.split('\n') {
        frame.push_str("data: ");
        frame.push_str(line);
        frame.push('\n');
    }
    frame.push('\n');
    frame
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use std::sync::mpsc::TryRecvError;

    use super::*;

    #[test]
    fn a_broadcast_reaches_every_subscriber() {
        let broadcaster = Broadcaster::default();
        let (_, first) = broadcaster.subscribe();
        let (_, second) = broadcaster.subscribe();
        broadcaster.broadcast("{\"type\":\"reload\"}");
        assert_eq!(
            first.try_recv().expect("first got it"),
            "{\"type\":\"reload\"}"
        );
        assert_eq!(
            second.try_recv().expect("second got it"),
            "{\"type\":\"reload\"}"
        );
    }

    #[test]
    fn an_unsubscribed_page_stops_receiving() {
        let broadcaster = Broadcaster::default();
        let (id, events) = broadcaster.subscribe();
        broadcaster.unsubscribe(id);
        broadcaster.broadcast("one");
        assert_eq!(events.try_recv(), Err(TryRecvError::Disconnected));
        assert_eq!(broadcaster.subscriber_count(), 0);
    }

    #[test]
    fn a_page_whose_thread_is_gone_is_dropped_rather_than_broadcast_to_forever() {
        let broadcaster = Broadcaster::default();
        let (_, events) = broadcaster.subscribe();
        drop(events);
        broadcaster.broadcast("one");
        assert_eq!(broadcaster.subscriber_count(), 0);
    }

    #[test]
    fn unsubscribing_an_unknown_id_is_harmless() {
        let broadcaster = Broadcaster::default();
        let (_, _events) = broadcaster.subscribe();
        broadcaster.unsubscribe(9_999);
        assert_eq!(broadcaster.subscriber_count(), 1);
    }

    #[test]
    fn a_data_frame_ends_with_the_blank_line_that_terminates_an_event() {
        assert_eq!(data_frame("{\"a\":1}"), "data: {\"a\":1}\n\n");
    }

    #[test]
    fn a_multi_line_message_stays_one_event() {
        // Two `data:` lines then one blank line: one event carrying both
        // lines, not two events carrying half a JSON document each.
        assert_eq!(data_frame("{\n}"), "data: {\ndata: }\n\n");
    }
}
