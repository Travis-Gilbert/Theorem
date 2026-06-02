//! Trace export: a run's events as training rows.

use serde::{Deserialize, Serialize};

use crate::event::Event;

/// A single training row exported from a run's event trace.
///
/// The receipts and the typed event log ARE the training corpus. Exporting them
/// is the SDK-level expression of the compounding loop: a consumer's runs become
/// the substrate's training signal that bootstraps the affordance-selection and
/// scoring heads. This first shape is a faithful, lossless per-event record;
/// instruction/response and preference-pair shaping for specific trainers builds
/// on top of it (THPS-013).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TraceRow {
    /// The run this row came from.
    pub run_id: String,
    /// The event's monotonic sequence number within the run.
    pub seq: u64,
    /// The typed kind, e.g. `Created`, `Cancelled`, `Validation`.
    pub kind: String,
    /// The raw canonical transition type, e.g. `RUN.CREATED`.
    pub event_type: String,
    /// The content-addressed state hash after this transition (the provenance
    /// anchor: it ties the row to an exact, replayable run state).
    pub state_hash_after: String,
    /// The transition payload.
    pub payload: serde_json::Value,
}

/// Export a run's events as training rows, in sequence order.
pub fn export_run_trace(events: &[Event]) -> Vec<TraceRow> {
    events
        .iter()
        .map(|event| TraceRow {
            run_id: event.run_id().to_string(),
            seq: event.seq(),
            kind: format!("{:?}", event.kind()),
            event_type: event.event_type().to_string(),
            state_hash_after: event.state_hash_after().to_string(),
            payload: serde_json::Value::Object(event.payload().clone()),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use theorem_harness_core::types::EventState;

    fn event(seq: u64, event_type: &str) -> Event {
        let inner: EventState = serde_json::from_value(serde_json::json!({
            "run_id": "harnessrun:xyz",
            "seq": seq,
            "type": event_type,
            "state_hash_after": format!("hash-{seq}")
        }))
        .expect("event state");
        Event::new(inner)
    }

    #[test]
    fn exports_rows_in_sequence() {
        let events = vec![event(1, "RUN.CREATED"), event(2, "RUN.CANCELLED")];
        let rows = export_run_trace(&events);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].seq, 1);
        assert_eq!(rows[0].event_type, "RUN.CREATED");
        assert_eq!(rows[0].kind, "Created");
        assert_eq!(rows[1].kind, "Cancelled");
        assert_eq!(rows[1].state_hash_after, "hash-2");
    }

    #[test]
    fn rows_serialize_to_json() {
        let rows = export_run_trace(&[event(1, "RUN.CREATED")]);
        let json = serde_json::to_string(&rows).expect("serialize");
        assert!(json.contains("RUN.CREATED"));
    }
}
