//! The stream surface: a resumable cursor over a run's typed events.

use rustyred_thg_core::GraphStore;

use crate::event::Event;
use crate::run::{RunHandle, SdkResult};

/// A resumable, poll-based cursor over a run's events.
///
/// This is the synchronous core of "runs that stream typed events." It is
/// intentionally NOT an async push-stream: the core stays runtime-free so it
/// binds everywhere. Each binding (NAPI tokio, UniFFI RustFuture polling, a
/// browser EventSource) wraps this cursor into its own language-native async
/// stream.
///
/// The cursor tracks the last sequence it returned, so it survives a reconnect:
/// persist the cursor with [`RunStream::cursor`], and a fresh stream resumed via
/// [`RunStream::resume_from`] picks up exactly where the last one stopped, as a
/// bounded replay over the durable event log with no data loss. This is the
/// resumable-streaming guarantee a mobile client on a dropping connection needs.
///
/// Two views, per the SDK v2 surface and Travis's decision that text is the
/// default: [`RunStream::poll`] yields the full typed [`Event`]s (provenance and
/// control), and [`RunStream::poll_text`] yields the concatenated text
/// projection (the convenience view for callers that only want the answer text).
/// Both consume from the same cursor, so a caller picks one view per stream.
#[derive(Clone, Debug)]
pub struct RunStream {
    run: RunHandle,
    cursor: u64,
}

impl RunStream {
    /// A stream over the whole run, from the beginning.
    pub fn new(run: &RunHandle) -> Self {
        Self {
            run: run.clone(),
            cursor: 0,
        }
    }

    /// A stream resumed after a known sequence boundary: pass the last sequence a
    /// previous stream returned to reconnect without re-reading seen events.
    pub fn resume_from(run: &RunHandle, after_seq: u64) -> Self {
        Self {
            run: run.clone(),
            cursor: after_seq,
        }
    }

    /// The last sequence this stream has returned. Persist it to resume later.
    pub fn cursor(&self) -> u64 {
        self.cursor
    }

    /// The typed view: drain every event after the cursor, advancing it. Returns
    /// an empty vec once the stream has caught up to the durable log.
    pub fn poll<S: GraphStore>(&mut self, store: &S) -> SdkResult<Vec<Event>> {
        let events = self.run.events_since(store, self.cursor)?;
        if let Some(last) = events.last() {
            self.cursor = last.seq();
        }
        Ok(events)
    }

    /// The text view (the default convenience): drain new events and return the
    /// concatenation of their text projections, advancing the cursor. Events
    /// without text contribute nothing; their full form is on [`RunStream::poll`].
    pub fn poll_text<S: GraphStore>(&mut self, store: &S) -> SdkResult<String> {
        let events = self.poll(store)?;
        let mut text = String::new();
        for event in &events {
            if let Some(chunk) = event.text() {
                text.push_str(&chunk);
            }
        }
        Ok(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::RunEventKind;
    use crate::idempotency::IdempotencyToken;
    use rustyred_thg_core::InMemoryGraphStore;
    use theorem_harness_core::types::Payload;

    fn start_run(store: &mut InMemoryGraphStore) -> RunHandle {
        RunHandle::start(
            store,
            "task",
            "claude-code",
            Payload::new(),
            IdempotencyToken::new("k-create"),
        )
        .expect("run starts")
    }

    #[test]
    fn polls_typed_events_then_catches_up_then_resumes_on_new_event() {
        let mut store = InMemoryGraphStore::default();
        let run = start_run(&mut store);
        let mut stream = RunStream::new(&run);

        // First poll drains the created event and advances the cursor.
        let first = stream.poll(&store).expect("poll");
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].kind(), RunEventKind::Created);
        assert!(stream.cursor() >= 1);

        // Caught up: the next poll is empty.
        assert!(stream.poll(&store).expect("poll").is_empty());

        // A new transition appears and the same stream picks it up.
        run.cancel(&mut store, "stop", IdempotencyToken::new("k-cancel"))
            .expect("cancel");
        let next = stream.poll(&store).expect("poll");
        assert_eq!(next.len(), 1);
        assert_eq!(next[0].kind(), RunEventKind::Cancelled);
    }

    #[test]
    fn text_view_projects_payload_text() {
        let mut store = InMemoryGraphStore::default();
        let run = start_run(&mut store);
        run.cancel(
            &mut store,
            "user stopped",
            IdempotencyToken::new("k-cancel"),
        )
        .expect("cancel");

        let mut stream = RunStream::new(&run);
        let text = stream.poll_text(&store).expect("poll_text");
        // The created event carries no answer-text; the cancellation reason does.
        assert!(text.contains("user stopped"));
    }

    #[test]
    fn resume_from_skips_already_seen_events() {
        let mut store = InMemoryGraphStore::default();
        let run = start_run(&mut store);
        let created_seq = run.events(&store).expect("events")[0].seq();

        let mut stream = RunStream::resume_from(&run, created_seq);
        // Nothing has happened after the created event yet.
        assert!(stream.poll(&store).expect("poll").is_empty());
        assert_eq!(stream.cursor(), created_seq);
    }
}
