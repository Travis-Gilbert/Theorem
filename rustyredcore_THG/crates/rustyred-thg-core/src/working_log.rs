use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::graph_store::now_ms;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkingLogEventKind {
    Episode,
    Access,
    Mutation,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct WorkingLogEvent {
    pub cursor: u64,
    pub ts_ms: i64,
    pub kind: WorkingLogEventKind,
    pub row_id: Option<String>,
    pub payload: Value,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct WorkingLog {
    events: Vec<WorkingLogEvent>,
    next_cursor: u64,
}

impl WorkingLog {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn append(
        &mut self,
        kind: WorkingLogEventKind,
        row_id: Option<String>,
        payload: Value,
    ) -> WorkingLogEvent {
        self.next_cursor += 1;
        let event = WorkingLogEvent {
            cursor: self.next_cursor,
            ts_ms: now_ms(),
            kind,
            row_id,
            payload,
        };
        self.events.push(event.clone());
        event
    }

    pub fn append_episode(&mut self, row_id: impl Into<String>, payload: Value) -> WorkingLogEvent {
        self.append(WorkingLogEventKind::Episode, Some(row_id.into()), payload)
    }

    pub fn append_access(&mut self, row_id: impl Into<String>) -> WorkingLogEvent {
        self.append(
            WorkingLogEventKind::Access,
            Some(row_id.into()),
            json!({ "access": true }),
        )
    }

    pub fn append_mutation(
        &mut self,
        row_id: impl Into<String>,
        payload: Value,
    ) -> WorkingLogEvent {
        self.append(WorkingLogEventKind::Mutation, Some(row_id.into()), payload)
    }

    pub fn subscribe_after(&self, cursor: u64, limit: usize) -> Vec<WorkingLogEvent> {
        self.events
            .iter()
            .filter(|event| event.cursor > cursor)
            .take(if limit == 0 { usize::MAX } else { limit })
            .cloned()
            .collect()
    }

    pub fn len(&self) -> usize {
        self.events.len()
    }

    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct RecencyCounter {
    rows: BTreeMap<String, RecencyState>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct RecencyState {
    pub row_id: String,
    pub last_accessed_ms: i64,
    pub access_count: u64,
    pub recency_score: f64,
}

impl RecencyCounter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn touch(&mut self, row_id: impl Into<String>, at_ms: i64) -> RecencyState {
        let row_id = row_id.into();
        let state = self.rows.entry(row_id.clone()).or_insert(RecencyState {
            row_id,
            last_accessed_ms: at_ms,
            access_count: 0,
            recency_score: 0.0,
        });
        state.last_accessed_ms = state.last_accessed_ms.max(at_ms);
        state.access_count = state.access_count.saturating_add(1);
        state.recency_score = recency_score(state.access_count, state.last_accessed_ms);
        state.clone()
    }

    pub fn state(&self, row_id: &str) -> Option<&RecencyState> {
        self.rows.get(row_id)
    }

    pub fn rows_aged_before(&self, cutoff_ms: i64) -> Vec<String> {
        self.rows
            .values()
            .filter(|state| state.last_accessed_ms <= cutoff_ms)
            .map(|state| state.row_id.clone())
            .collect()
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct TemporalFact {
    pub fact_id: String,
    pub t_valid: Option<i64>,
    pub t_invalid: Option<i64>,
    pub t_created: i64,
    pub t_expired: Option<i64>,
    pub payload: Value,
}

impl TemporalFact {
    pub fn new(fact_id: impl Into<String>, t_created: i64, payload: Value) -> Self {
        Self {
            fact_id: fact_id.into(),
            t_valid: Some(t_created),
            t_invalid: None,
            t_created,
            t_expired: None,
            payload,
        }
    }

    pub fn invalidate(&self, t_invalid: i64, t_expired: i64) -> Self {
        let mut invalidated = self.clone();
        invalidated.t_invalid = Some(t_invalid);
        invalidated.t_expired = Some(t_expired);
        invalidated
    }

    pub fn is_valid_at(&self, ts_ms: i64) -> bool {
        self.t_valid.map(|start| ts_ms >= start).unwrap_or(true)
            && self.t_invalid.map(|end| ts_ms < end).unwrap_or(true)
    }
}

fn recency_score(access_count: u64, last_accessed_ms: i64) -> f64 {
    (access_count as f64).ln_1p() + (last_accessed_ms.max(0) as f64 / 1_000.0)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn working_log_subscriber_resumes_from_cursor_in_order() {
        let mut log = WorkingLog::new();
        let first = log.append_episode("episode:1", json!({ "text": "hello" }));
        log.append_access("episode:1");
        log.append_mutation("episode:1", json!({ "field": "status" }));

        let events = log.subscribe_after(first.cursor, 0);
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].cursor, first.cursor + 1);
        assert_eq!(events[0].kind, WorkingLogEventKind::Access);
        assert_eq!(events[1].kind, WorkingLogEventKind::Mutation);

        let limited = log.subscribe_after(0, 2);
        assert_eq!(limited.len(), 2);
        assert_eq!(limited[0].kind, WorkingLogEventKind::Episode);
    }

    #[test]
    fn recency_counter_tracks_hot_rows_and_aged_drop_candidates() {
        let mut counter = RecencyCounter::new();
        counter.touch("row:old", 10);
        counter.touch("row:hot", 100);
        counter.touch("row:hot", 110);

        assert_eq!(counter.state("row:hot").unwrap().access_count, 2);
        assert!(counter.state("row:hot").unwrap().recency_score > 0.0);
        assert_eq!(counter.rows_aged_before(50), vec!["row:old".to_string()]);
    }

    #[test]
    fn temporal_fact_invalidation_preserves_validity_history() {
        let fact = TemporalFact::new("fact:1", 100, json!({ "claim": "enabled" }));
        let invalidated = fact.invalidate(200, 250);

        assert!(invalidated.is_valid_at(150));
        assert!(!invalidated.is_valid_at(220));
        assert_eq!(invalidated.t_valid, Some(100));
        assert_eq!(invalidated.t_invalid, Some(200));
        assert_eq!(invalidated.t_created, 100);
        assert_eq!(invalidated.t_expired, Some(250));
    }
}
