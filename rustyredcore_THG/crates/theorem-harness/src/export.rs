//! Trace export: a run's events as training rows.

use serde::{Deserialize, Serialize};

use crate::event::{Event, RunEventKind};

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

/// An instruction/response training pair distilled from a run's trace.
///
/// The SFT shape a trainer consumes: `instruction` is the run's task (what was
/// asked), `response` is the text the run produced and concluded with. One record
/// per run. Preference pairs (chosen vs rejected) compare two runs and are built
/// by [`export_preference_pair`].
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SftRecord {
    /// The run this record came from.
    pub run_id: String,
    /// The task the run was created to do.
    pub instruction: String,
    /// The text the run produced (outcome summaries, reasons, synthesis), in order.
    pub response: String,
}

/// Distill a run's events into one instruction/response SFT record, or `None` if
/// the run has no task (no `RUN.CREATED` carrying a non-empty `task`).
pub fn export_run_sft(events: &[Event]) -> Option<SftRecord> {
    let created = events
        .iter()
        .find(|event| event.kind() == RunEventKind::Created)?;
    let instruction = created
        .payload()
        .get("task")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_string();
    if instruction.is_empty() {
        return None;
    }
    let mut response = String::new();
    for event in events {
        if let Some(text) = event.text() {
            if !response.is_empty() {
                response.push('\n');
            }
            response.push_str(&text);
        }
    }
    Some(SftRecord {
        run_id: created.run_id().to_string(),
        instruction,
        response,
    })
}

/// A preference pair (chosen vs rejected response to the same instruction), the
/// shape a DPO-style trainer consumes.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PreferencePair {
    /// The shared task both runs addressed.
    pub instruction: String,
    /// The response from the preferred run.
    pub chosen: String,
    /// The response from the rejected run.
    pub rejected: String,
}

/// Build a preference pair from two runs of the same task. The caller picks the
/// `winner` and `loser` from the runs' outcomes (for example closed vs cancelled,
/// or accepted vs rejected). Returns `None` if either run lacks a task or the two
/// addressed different tasks.
pub fn export_preference_pair(winner: &[Event], loser: &[Event]) -> Option<PreferencePair> {
    let chosen = export_run_sft(winner)?;
    let rejected = export_run_sft(loser)?;
    if chosen.instruction != rejected.instruction {
        return None;
    }
    Some(PreferencePair {
        instruction: chosen.instruction,
        chosen: chosen.response,
        rejected: rejected.response,
    })
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

    fn event_p(seq: u64, event_type: &str, payload: serde_json::Value) -> Event {
        let inner: EventState = serde_json::from_value(serde_json::json!({
            "run_id": "harnessrun:xyz",
            "seq": seq,
            "type": event_type,
            "state_hash_after": format!("hash-{seq}"),
            "payload": payload
        }))
        .expect("event state");
        Event::new(inner)
    }

    #[test]
    fn export_run_sft_pairs_task_with_outcome() {
        let events = vec![
            event_p(
                1,
                "RUN.CREATED",
                serde_json::json!({"task": "do the thing", "actor": "x"}),
            ),
            event_p(
                2,
                "RUN.CANCELLED",
                serde_json::json!({"reason": "stopped early"}),
            ),
        ];
        let sft = export_run_sft(&events).expect("sft");
        assert_eq!(sft.instruction, "do the thing");
        assert!(sft.response.contains("stopped early"));
    }

    #[test]
    fn export_run_sft_none_without_a_task() {
        assert!(export_run_sft(&[event(1, "HOST.OBSERVED")]).is_none());
    }

    #[test]
    fn export_preference_pair_from_two_runs() {
        let winner = vec![
            event_p(1, "RUN.CREATED", serde_json::json!({"task": "T"})),
            event_p(
                2,
                "RUN.CLOSED",
                serde_json::json!({"summary": "good outcome"}),
            ),
        ];
        let loser = vec![
            event_p(1, "RUN.CREATED", serde_json::json!({"task": "T"})),
            event_p(
                2,
                "RUN.CANCELLED",
                serde_json::json!({"reason": "bad outcome"}),
            ),
        ];
        let pref = export_preference_pair(&winner, &loser).expect("pref");
        assert_eq!(pref.instruction, "T");
        assert!(pref.chosen.contains("good outcome"));
        assert!(pref.rejected.contains("bad outcome"));
    }

    #[test]
    fn export_preference_pair_none_for_different_tasks() {
        let a = vec![event_p(1, "RUN.CREATED", serde_json::json!({"task": "A"}))];
        let b = vec![event_p(1, "RUN.CREATED", serde_json::json!({"task": "B"}))];
        assert!(export_preference_pair(&a, &b).is_none());
    }
}
