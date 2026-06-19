//! Append-only coordination streams for the Theorem HotGraph runtime.
//!
//! Streams are the live awareness channel for multi-head coordination. They
//! replace turn-start room polling with an append-only, cursor-read delta: an
//! actor pulls only the events on its subscribed streams that arrived after its
//! stored cursor. CRDT stays on the graph (shared *state*, the `crdt/` module);
//! streams carry *communication and awareness*.
//!
//! The primitive is the append-only special case of [`crate::ordered`]: where
//! `OrderedIndex` keys an `imbl::OrdMap` by score, a stream keys an
//! `imbl::OrdMap<u64, StreamEvent>` by a monotonic ordering token and reads the
//! tail after a cursor with the same range-after machinery:
//!
//! - `publish(stream_key, event) -> ordering_token` (append)
//! - `read_after(stream_key, cursor, limit) -> events` (OrdMap range `> cursor`)
//!
//! Cursors are per-`(actor, stream)` entries; subscriptions are per-actor sets
//! (selective attention). A ping (urgency `ask`|`block` with a `target_actor`)
//! lands on the stream like any event and additionally enqueues to the target's
//! mention/wake queue, bypassing the target's attention on purpose.
//!
//! Using `imbl` throughout mirrors [`crate::state::ThgState`]: embedding the
//! [`StreamStore`] there keeps the state clone copy-on-write (structural
//! sharing) and folds streams into the state hash like `runs`/`contexts`.
//!
//! Scope: every `stream_key` is `(tenant, topic)` resolved through the existing
//! [`sanitize_tenant_segment`] normalizer; an empty tenant is rejected rather
//! than routed to a default. No new scope resolver is introduced. The
//! coordination/MCP layer passes a tenant it already canonicalized through the
//! consolidation normalizer before calling in.

use std::ops::Bound;

use imbl::{OrdMap, OrdSet, Vector};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::graph_store::{sanitize_tenant_segment, unix_ms};

/// Default cap on the number of events returned by a single stream read.
pub const DEFAULT_READ_LIMIT: usize = 256;

/// Separator between the tenant and topic segments inside a `stream_key`. `|`
/// cannot appear in a sanitized segment (`[A-Za-z0-9_-]`), so the join is
/// unambiguous.
const STREAM_KEY_SEPARATOR: char = '|';

/// A `(stream_key, ordering_token)` reference into the append-log. Pending pings
/// store these so the mention drain resolves each to its event in O(log n)
/// without scanning, while the log stays the single source of truth.
type EventRef = (String, u64);

/// Urgency dial: the passive-vs-active selector that already exists in the
/// coordination model. `Info` is passive (delta-read only). `Ask`/`Block` are
/// intentional pings that additionally reach the target's wake path.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StreamUrgency {
    #[default]
    Info,
    Ask,
    Block,
}

impl StreamUrgency {
    /// Parse an urgency token. An empty/absent value is the passive default
    /// (`Info`); anything unrecognized is rejected by the caller.
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "" | "info" => Some(Self::Info),
            "ask" => Some(Self::Ask),
            "block" => Some(Self::Block),
            _ => None,
        }
    }

    /// A ping is an intentional escalation that bridges to the wake/mention path.
    pub fn is_ping(self) -> bool {
        matches!(self, Self::Ask | Self::Block)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Ask => "ask",
            Self::Block => "block",
        }
    }
}

/// A single append-only coordination event.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct StreamEvent {
    pub id: String,
    pub stream_key: String,
    pub ordering_token: u64,
    pub actor: String,
    pub kind: String,
    pub payload: Value,
    pub urgency: StreamUrgency,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_actor: Option<String>,
    pub created_at: u64,
}

/// A scope-resolution / validation failure. Carries a stable machine code that
/// the executor maps onto the [`crate::errors::ThgError`] envelope.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StreamError {
    pub code: &'static str,
    pub message: String,
}

impl StreamError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

/// The whole coordination substrate: all stream logs, per-`(actor, stream)`
/// cursors, per-actor subscription sets, and the per-target pending-ping queue.
///
/// Every collection is an `imbl` persistent structure so embedding this in
/// [`crate::state::ThgState`] keeps state clones copy-on-write.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct StreamStore {
    /// Monotonic source of ordering tokens. Global across streams so every event
    /// id is unique and the per-stream subsequence stays strictly increasing --
    /// `read_after` ranges over tokens within a single stream.
    seq: u64,
    /// `stream_key` -> append-only log (`ordering_token` -> event).
    streams: OrdMap<String, OrdMap<u64, StreamEvent>>,
    /// `actor` -> (`stream_key` -> last consumed ordering token).
    cursors: OrdMap<String, OrdMap<String, u64>>,
    /// `actor` -> subscribed `stream_key`s (selective attention).
    subscriptions: OrdMap<String, OrdSet<String>>,
    /// `target_actor` -> queued ping references (the mention/wake bridge).
    pending_pings: OrdMap<String, Vector<EventRef>>,
}

impl StreamStore {
    fn next_token(&mut self) -> u64 {
        self.seq += 1;
        self.seq
    }

    /// Resolve `(tenant, topic)` into a `stream_key` via the existing tenant
    /// normalizer. An empty tenant is rejected, never defaulted; an empty topic
    /// is rejected. No new scope resolver is introduced.
    pub fn resolve_stream_key(tenant: &str, topic: &str) -> Result<String, StreamError> {
        let tenant_trimmed = tenant.trim();
        if tenant_trimmed.is_empty() {
            return Err(StreamError::new(
                "empty_tenant",
                "stream tenant must not be empty",
            ));
        }
        // Reuse the existing percent-encoding normalizer (`pct_` prefix). It is
        // reversible and never collapses a non-empty tenant to a default, so the
        // empty-tenant rejection above is the whole of the "no silent default"
        // contract.
        let tenant_segment = sanitize_tenant_segment(tenant_trimmed);
        let topic_trimmed = topic.trim();
        if topic_trimmed.is_empty() {
            return Err(StreamError::new(
                "empty_stream_topic",
                "stream topic must not be empty",
            ));
        }
        let topic_segment = sanitize_tenant_segment(topic_trimmed);
        Ok(format!(
            "{tenant_segment}{STREAM_KEY_SEPARATOR}{topic_segment}"
        ))
    }

    /// Append an event to a `(tenant, topic)` stream and return it (carrying its
    /// new ordering token). A ping (urgency `ask`|`block` with a `target_actor`)
    /// also enqueues to the target's pending-ping queue, regardless of whether
    /// the target is subscribed -- a ping bypasses attention by design.
    #[allow(clippy::too_many_arguments)]
    pub fn publish(
        &mut self,
        tenant: &str,
        topic: &str,
        actor: &str,
        kind: &str,
        payload: Value,
        urgency: StreamUrgency,
        target_actor: Option<String>,
    ) -> Result<StreamEvent, StreamError> {
        let stream_key = Self::resolve_stream_key(tenant, topic)?;
        let token = self.next_token();
        let event = StreamEvent {
            id: format!("evt:{token:016x}"),
            stream_key: stream_key.clone(),
            ordering_token: token,
            actor: actor.to_string(),
            kind: kind.to_string(),
            payload,
            urgency,
            target_actor: target_actor.clone(),
            created_at: unix_ms() as u64,
        };

        // Append to the per-stream log (copy-on-write clone of the inner OrdMap).
        let mut log = self.streams.get(&stream_key).cloned().unwrap_or_default();
        log.insert(token, event.clone());
        self.streams.insert(stream_key.clone(), log);

        if urgency.is_ping() {
            if let Some(target) = target_actor {
                let target = target.trim();
                if !target.is_empty() {
                    let mut queue = self.pending_pings.get(target).cloned().unwrap_or_default();
                    queue.push_back((stream_key, token));
                    self.pending_pings.insert(target.to_string(), queue);
                }
            }
        }
        Ok(event)
    }

    fn cursor_for(&self, actor: &str, stream_key: &str) -> u64 {
        self.cursors
            .get(actor)
            .and_then(|streams| streams.get(stream_key))
            .copied()
            .unwrap_or(0)
    }

    fn set_cursor(&mut self, actor: &str, stream_key: &str, token: u64) {
        let mut actor_cursors = self.cursors.get(actor).cloned().unwrap_or_default();
        actor_cursors.insert(stream_key.to_string(), token);
        self.cursors.insert(actor.to_string(), actor_cursors);
    }

    /// Events strictly after `cursor` in one stream, ascending by token, capped
    /// at `limit`. Reuses the OrdMap range-after machinery (`crate::ordered`):
    /// the log is token-ordered, so a half-open range from `cursor` is exact and
    /// O(result + log n) -- nothing in the window is missed or duplicated.
    fn read_after(log: &OrdMap<u64, StreamEvent>, cursor: u64, limit: usize) -> Vec<StreamEvent> {
        log.range((Bound::Excluded(cursor), Bound::Unbounded))
            .take(limit)
            .map(|(_, event)| event.clone())
            .collect()
    }

    /// The actor's current subscription set, as resolved `stream_key`s.
    pub fn subscriptions_for(&self, actor: &str) -> Vec<String> {
        self.subscriptions
            .get(actor)
            .map(|set| set.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Pull events after the actor's cursor across `stream_keys` (or the actor's
    /// subscription set when `stream_keys` is empty -- selective attention).
    /// Advances the per-`(actor, stream)` cursor when `advance` is set. Returns
    /// the events in a single total order plus the resulting cursor per stream.
    pub fn read(
        &mut self,
        actor: &str,
        stream_keys: &[String],
        advance: bool,
        limit: usize,
    ) -> (Vec<StreamEvent>, OrdMap<String, u64>) {
        let limit = if limit == 0 { DEFAULT_READ_LIMIT } else { limit };
        let keys: Vec<String> = if stream_keys.is_empty() {
            self.subscriptions_for(actor)
        } else {
            stream_keys.to_vec()
        };

        let mut events = Vec::new();
        let mut new_cursors = OrdMap::new();
        for key in keys {
            let cursor = self.cursor_for(actor, &key);
            let delta = self
                .streams
                .get(&key)
                .map(|log| Self::read_after(log, cursor, limit))
                .unwrap_or_default();
            match delta.last() {
                Some(last) => {
                    let token = last.ordering_token;
                    if advance {
                        self.set_cursor(actor, &key, token);
                    }
                    new_cursors.insert(key.clone(), token);
                }
                None => {
                    // No new events: report the standing cursor, unchanged.
                    new_cursors.insert(key.clone(), cursor);
                }
            }
            events.extend(delta);
        }
        // Global ordering tokens give a single total order across streams with no
        // merge step.
        events.sort_by_key(|event| event.ordering_token);
        (events, new_cursors)
    }

    /// Add `stream_key` to the actor's subscription set; returns the new set.
    ///
    /// Opting in starts attention *now*: a first-time subscriber's cursor is
    /// initialized to the stream's current head, so the next read returns events
    /// published after the subscription rather than the full backlog. A
    /// re-subscribe keeps any existing cursor, resuming where the actor left off.
    /// (Backlog is still reachable by reading the stream explicitly.)
    pub fn subscribe(&mut self, actor: &str, stream_key: &str) -> Vec<String> {
        let mut set = self.subscriptions.get(actor).cloned().unwrap_or_default();
        set.insert(stream_key.to_string());
        self.subscriptions.insert(actor.to_string(), set);

        let has_cursor = self
            .cursors
            .get(actor)
            .map(|cursors| cursors.contains_key(stream_key))
            .unwrap_or(false);
        if !has_cursor {
            let head = self
                .streams
                .get(stream_key)
                .and_then(|log| log.iter().next_back().map(|(token, _)| *token))
                .unwrap_or(0);
            self.set_cursor(actor, stream_key, head);
        }
        self.subscriptions_for(actor)
    }

    /// Remove `stream_key` from the actor's subscription set; returns the new set.
    pub fn unsubscribe(&mut self, actor: &str, stream_key: &str) -> Vec<String> {
        if let Some(mut set) = self.subscriptions.get(actor).cloned() {
            set.remove(stream_key);
            if set.is_empty() {
                self.subscriptions.remove(actor);
            } else {
                self.subscriptions.insert(actor.to_string(), set);
            }
        }
        self.subscriptions_for(actor)
    }

    /// Drain (or peek) the target actor's pending pings, resolving each reference
    /// back to its event in arrival order. This is the seam the Stop-hook mention
    /// drain (warm head) and the spawn/courier wake (cold head) bridge onto. When
    /// `advance` is set the queue is cleared.
    pub fn drain_mentions(&mut self, actor: &str, advance: bool) -> Vec<StreamEvent> {
        let refs = match self.pending_pings.get(actor) {
            Some(refs) if !refs.is_empty() => refs.clone(),
            _ => return Vec::new(),
        };
        let mut events: Vec<StreamEvent> = refs
            .iter()
            .filter_map(|(stream_key, token)| self.event_at(stream_key, *token))
            .collect();
        events.sort_by_key(|event| event.ordering_token);
        if advance {
            self.pending_pings.remove(actor);
        }
        events
    }

    fn event_at(&self, stream_key: &str, token: u64) -> Option<StreamEvent> {
        self.streams
            .get(stream_key)
            .and_then(|log| log.get(&token))
            .cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn store() -> StreamStore {
        StreamStore::default()
    }

    fn tokens(events: &[StreamEvent]) -> Vec<u64> {
        events.iter().map(|e| e.ordering_token).collect()
    }

    // --- Acceptance criterion 1 -------------------------------------------
    // A head offline for N turns, on reconnect, pulls exactly the events on its
    // subscribed streams after its stored cursor, in order, cursor advancing;
    // nothing in that window is missed or duplicated.
    #[test]
    fn offline_head_pulls_exact_delta_after_cursor() {
        let mut s = store();
        let key = StreamStore::resolve_stream_key("acme", "room").unwrap();
        s.subscribe("alice", &key);

        for i in 0..3 {
            s.publish("acme", "room", "bob", "msg", json!({ "i": i }), StreamUrgency::Info, None)
                .unwrap();
        }
        let (first, cursors) = s.read("alice", &[], true, 0);
        assert_eq!(tokens(&first), vec![1, 2, 3]);
        assert_eq!(cursors[&key], 3);

        // Offline for N turns; two more events land.
        s.publish("acme", "room", "bob", "msg", json!({ "i": 3 }), StreamUrgency::Info, None)
            .unwrap();
        s.publish("acme", "room", "bob", "msg", json!({ "i": 4 }), StreamUrgency::Info, None)
            .unwrap();

        // On reconnect, EXACTLY the two events after the cursor, in order.
        let (delta, cursors) = s.read("alice", &[], true, 0);
        assert_eq!(tokens(&delta), vec![4, 5]);
        assert_eq!(cursors[&key], 5);

        // Idempotent re-read returns nothing -- no duplicates.
        let (empty, _) = s.read("alice", &[], true, 0);
        assert!(empty.is_empty());
    }

    // --- Acceptance criterion 2 -------------------------------------------
    // A ping (urgency ask|block + target) to a warm head appears in its next
    // mention drain; the same queue triggers a cold-head wake.
    #[test]
    fn ping_lands_in_targets_mention_drain() {
        let mut s = store();
        let event = s
            .publish(
                "acme",
                "room",
                "alice",
                "question",
                json!({ "q": "review?" }),
                StreamUrgency::Ask,
                Some("bob".to_string()),
            )
            .unwrap();

        let mentions = s.drain_mentions("bob", true);
        assert_eq!(mentions.len(), 1);
        assert_eq!(mentions[0].id, event.id);
        assert_eq!(mentions[0].urgency, StreamUrgency::Ask);

        // Drained.
        assert!(s.drain_mentions("bob", true).is_empty());

        // A `block` ping queues just the same; peek does not drain.
        s.publish("acme", "room", "alice", "blocker", json!({}), StreamUrgency::Block, Some("bob".to_string()))
            .unwrap();
        assert_eq!(s.drain_mentions("bob", false).len(), 1);
        assert_eq!(s.drain_mentions("bob", true).len(), 1);
        assert!(s.drain_mentions("bob", true).is_empty());
    }

    // --- Acceptance criterion 3 -------------------------------------------
    // Two heads publishing concurrently to one stream receive distinct ordering
    // tokens; both events are readable in a single total order with no merge.
    #[test]
    fn concurrent_publishers_get_distinct_tokens_in_total_order() {
        let mut s = store();
        let e1 = s
            .publish("acme", "room", "alice", "msg", json!({}), StreamUrgency::Info, None)
            .unwrap();
        let e2 = s
            .publish("acme", "room", "bob", "msg", json!({}), StreamUrgency::Info, None)
            .unwrap();
        assert_ne!(e1.ordering_token, e2.ordering_token);
        assert!(e1.ordering_token < e2.ordering_token);

        let key = StreamStore::resolve_stream_key("acme", "room").unwrap();
        let (events, _) = s.read("carol", &[key], false, 0);
        assert_eq!(tokens(&events), vec![e1.ordering_token, e2.ordering_token]);
    }

    // --- Acceptance criterion 4 -------------------------------------------
    // A publish and a read under the configured tenant share a stream; a call
    // with an empty tenant is rejected, not silently routed to a default.
    #[test]
    fn tenant_scope_shares_stream_and_rejects_empty() {
        let mut s = store();
        s.publish("acme", "room", "alice", "msg", json!({}), StreamUrgency::Info, None)
            .unwrap();
        let key = StreamStore::resolve_stream_key("acme", "room").unwrap();
        let (events, _) = s.read("bob", &[key], false, 0);
        assert_eq!(events.len(), 1);

        // Empty / whitespace-only tenant is rejected, never defaulted.
        assert_eq!(
            s.publish("", "room", "a", "msg", json!({}), StreamUrgency::Info, None)
                .unwrap_err()
                .code,
            "empty_tenant"
        );
        assert_eq!(
            StreamStore::resolve_stream_key("   ", "room").unwrap_err().code,
            "empty_tenant"
        );
        // An empty topic is rejected too.
        assert_eq!(
            StreamStore::resolve_stream_key("acme", "  ").unwrap_err().code,
            "empty_stream_topic"
        );
        // A non-empty tenant always resolves (the normalizer is total).
        assert!(StreamStore::resolve_stream_key("acme", "room").is_ok());
    }

    // --- Acceptance criterion 5 -------------------------------------------
    // Subscribing and unsubscribing changes which streams' deltas a read
    // returns; a ping still reaches an unsubscribed target.
    #[test]
    fn attention_controls_reads_but_ping_bypasses_it() {
        let mut s = store();
        let key = StreamStore::resolve_stream_key("acme", "room").unwrap();

        s.publish("acme", "room", "alice", "msg", json!({ "n": 0 }), StreamUrgency::Info, None)
            .unwrap();
        let (none, _) = s.read("bob", &[], true, 0);
        assert!(none.is_empty(), "no subscription => empty delta");

        s.subscribe("bob", &key);
        s.publish("acme", "room", "alice", "msg", json!({ "n": 1 }), StreamUrgency::Info, None)
            .unwrap();
        let (after_sub, _) = s.read("bob", &[], true, 0);
        assert_eq!(after_sub.len(), 1);

        s.unsubscribe("bob", &key);
        s.publish("acme", "room", "alice", "msg", json!({ "n": 2 }), StreamUrgency::Info, None)
            .unwrap();
        let (after_unsub, _) = s.read("bob", &[], true, 0);
        assert!(after_unsub.is_empty(), "unsubscribed => no delta");

        // A ping still reaches bob even though he is not subscribed.
        s.publish("acme", "room", "alice", "ping", json!({}), StreamUrgency::Block, Some("bob".to_string()))
            .unwrap();
        assert_eq!(s.drain_mentions("bob", true).len(), 1, "ping bypasses attention");
    }

    #[test]
    fn urgency_parses_and_rejects_unknown() {
        assert_eq!(StreamUrgency::parse(""), Some(StreamUrgency::Info));
        assert_eq!(StreamUrgency::parse("INFO"), Some(StreamUrgency::Info));
        assert_eq!(StreamUrgency::parse(" ask "), Some(StreamUrgency::Ask));
        assert_eq!(StreamUrgency::parse("block"), Some(StreamUrgency::Block));
        assert_eq!(StreamUrgency::parse("urgent"), None);
    }

    #[test]
    fn read_limit_caps_the_window_and_resumes() {
        let mut s = store();
        let key = StreamStore::resolve_stream_key("acme", "room").unwrap();
        for i in 0..10 {
            s.publish("acme", "room", "a", "msg", json!({ "i": i }), StreamUrgency::Info, None)
                .unwrap();
        }
        let (events, cursors) = s.read("alice", &[key.clone()], true, 4);
        assert_eq!(tokens(&events), vec![1, 2, 3, 4]);
        assert_eq!(cursors[&key], 4);
        let (next, _) = s.read("alice", &[key], true, 4);
        assert_eq!(tokens(&next), vec![5, 6, 7, 8]);
    }
}
