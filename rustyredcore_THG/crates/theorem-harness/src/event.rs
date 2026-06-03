//! The typed view of the canonical harness transition log.

use theorem_harness_core::types::{EventState, Payload};

/// The typed kind of a run event, derived from the canonical transition type.
///
/// The harness transition log uses string event types (`RUN.CREATED`,
/// `CONTEXT.PACKED`, `VALIDATION.FINISHED`, ...). This enum lifts the lifecycle
/// types a consumer cares about into a typed shape, while carrying everything
/// else through [`RunEventKind::Other`] without losing information.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RunEventKind {
    /// `RUN.CREATED`: the run opened.
    Created,
    /// `RUN.CLOSED`: the run finished with a recorded outcome.
    Closed,
    /// `RUN.FAILED`: the run failed.
    Failed,
    /// `RUN.CANCELLED`: the run was cancelled cleanly.
    Cancelled,
    /// `RUN.REPLAYED`: the run was replayed from its log.
    Replayed,
    /// `RUN.FORKED`: the run was forked at a sequence boundary.
    Forked,
    /// A validation transition (`VALIDATION.STARTED` / `RUNNING` / `FINISHED`).
    Validation,
    /// `OUTCOME.RECORDED`: the run's outcome was recorded.
    Outcome,
    /// Any other transition type, carried verbatim (context, toolkit, cache,
    /// oracle, federation, and the rest of the lifecycle).
    Other(String),
}

impl RunEventKind {
    /// Map a canonical transition type string to a [`RunEventKind`].
    pub fn from_event_type(event_type: &str) -> Self {
        match event_type {
            "RUN.CREATED" => Self::Created,
            "RUN.CLOSED" => Self::Closed,
            "RUN.FAILED" => Self::Failed,
            "RUN.CANCELLED" => Self::Cancelled,
            "RUN.REPLAYED" => Self::Replayed,
            "RUN.FORKED" => Self::Forked,
            "OUTCOME.RECORDED" => Self::Outcome,
            other if other.starts_with("VALIDATION.") => Self::Validation,
            other => Self::Other(other.to_string()),
        }
    }
}

/// A typed view over a persisted [`EventState`].
///
/// Carries the event's provenance: its sequence number (the resumable-streaming
/// cursor; see [`crate::RunHandle::events_since`]), the post-transition state
/// hash, and the payload. The inner [`EventState`] is always available via
/// [`Event::as_inner`] for consumers that need every field.
#[derive(Clone, Debug)]
pub struct Event {
    inner: EventState,
}

impl Event {
    /// Wrap a persisted event.
    pub fn new(inner: EventState) -> Self {
        Self { inner }
    }

    /// The typed kind of this event.
    pub fn kind(&self) -> RunEventKind {
        RunEventKind::from_event_type(&self.inner.event_type)
    }

    /// The monotonic per-run sequence number (the resume cursor).
    pub fn seq(&self) -> u64 {
        self.inner.seq
    }

    /// The run this event belongs to.
    pub fn run_id(&self) -> &str {
        &self.inner.run_id
    }

    /// The raw canonical transition type string.
    pub fn event_type(&self) -> &str {
        &self.inner.event_type
    }

    /// The transition payload.
    pub fn payload(&self) -> &Payload {
        &self.inner.payload
    }

    /// The content-addressed state hash after this transition.
    pub fn state_hash_after(&self) -> &str {
        &self.inner.state_hash_after
    }

    /// The client idempotency token recorded with this event (empty if none). A
    /// retry carrying the same token short-circuits to this event rather than
    /// appending again.
    pub fn idempotency_key(&self) -> &str {
        &self.inner.idempotency_key
    }

    /// A human-readable text projection of this event, if it carries one.
    ///
    /// The text stream (the default convenience view) is built from these
    /// projections. The payload is checked for a text-bearing field in priority
    /// order: answer/synthesis content first, then summaries, messages, and
    /// reasons. Events that carry no text (pure structural transitions like a
    /// toolkit compile) return `None` and contribute nothing to the text stream,
    /// while their full typed form is always available on the typed stream.
    pub fn text(&self) -> Option<String> {
        const TEXT_FIELDS: [&str; 8] = [
            "text",
            "content",
            "synthesis",
            "answer",
            "summary",
            "message",
            "reason",
            "note",
        ];
        for field in TEXT_FIELDS {
            if let Some(serde_json::Value::String(value)) = self.inner.payload.get(field) {
                if !value.is_empty() {
                    return Some(value.clone());
                }
            }
        }
        None
    }

    /// Borrow the full underlying event.
    pub fn as_inner(&self) -> &EventState {
        &self.inner
    }

    /// Consume into the full underlying event.
    pub fn into_inner(self) -> EventState {
        self.inner
    }
}

impl From<EventState> for Event {
    fn from(inner: EventState) -> Self {
        Self::new(inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_lifecycle_types() {
        assert_eq!(
            RunEventKind::from_event_type("RUN.CREATED"),
            RunEventKind::Created
        );
        assert_eq!(
            RunEventKind::from_event_type("RUN.CANCELLED"),
            RunEventKind::Cancelled
        );
        assert_eq!(
            RunEventKind::from_event_type("VALIDATION.FINISHED"),
            RunEventKind::Validation
        );
        assert_eq!(
            RunEventKind::from_event_type("CONTEXT.PACKED"),
            RunEventKind::Other("CONTEXT.PACKED".to_string())
        );
    }

    #[test]
    fn wraps_event_state() {
        let inner: EventState = serde_json::from_value(serde_json::json!({
            "run_id": "harnessrun:abc",
            "seq": 3u64,
            "type": "RUN.CREATED",
            "state_hash_after": "deadbeef"
        }))
        .expect("event state deserializes");
        let event = Event::new(inner);
        assert_eq!(event.seq(), 3);
        assert_eq!(event.run_id(), "harnessrun:abc");
        assert_eq!(event.kind(), RunEventKind::Created);
        assert_eq!(event.state_hash_after(), "deadbeef");
    }
}
