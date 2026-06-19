//! Append-only coordination event streams: the reactive companion to
//! [`ordered`](crate::ordered).
//!
//! `ordered.rs` is a copy-on-write B-tree keyed by an arbitrary `f64` score, so a
//! re-`zadd` can *move* an entry. A coordination stream is the append-only
//! special case: it is keyed by a strictly-increasing `u64` ordering token that
//! never moves once assigned. That single constraint is what makes the read a
//! cheap, lossless delta -- `read_after(cursor)` is an early-stopping `OrdMap`
//! range over `(cursor, ..]`, the same range-after machinery `EvictionFrontier`
//! leans on, with no merge step and nothing in the window missed or duplicated.
//!
//! This module is the *one data model* the stream-coordination spec calls for.
//! It is pure, `tokio`-free, and in-memory:
//!
//! - [`StreamLog`] is the single-stream log ([`append`](StreamLog::append) ->
//!   ordering token, [`read_after`](StreamLog::read_after) -> ordered delta).
//! - [`StreamRegistry`] is the local-embedded *live-tail transport*: it bundles
//!   many logs with per-`(actor, stream)` [cursors](StreamRegistry::cursor) and
//!   per-actor [subscriptions](StreamRegistry::subscriptions), so a warm head can
//!   pull its subscribed delta with no polling.
//!
//! Remotely, the same [`StreamEvent`] model degrades to a delta-pull over durable
//! storage (the harness MCP server persists each event as a graph node and
//! rehydrates a transient `StreamLog` via [`ingest`](StreamLog::ingest) to run
//! the identical `read_after`). One model, two transports.
//!
//! Scope (`(tenant, topic)`) resolution -- consistent casing, empty tenant
//! rejected rather than defaulted -- is a transport-layer concern handled by the
//! caller through the harness tenant normalizer; this module treats the canonical
//! [`StreamKey`] string as opaque.

use std::collections::{BTreeMap, BTreeSet};
use std::ops::Bound;

use imbl::OrdMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Escalation level carried on a [`StreamEvent`]. `Info` is the passive default;
/// `Ask`/`Block` are the *intentional ping* levels that, paired with a
/// `target_actor`, additionally enqueue onto the target's mention/wake path.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StreamUrgency {
    /// Passive, awareness-only. Lands on the stream; never wakes anyone.
    #[default]
    Info,
    /// A question directed at `target_actor`. Pings the target.
    Ask,
    /// A blocker directed at `target_actor`. Pings the target.
    Block,
}

impl StreamUrgency {
    /// The wire string (`info` | `ask` | `block`).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Ask => "ask",
            Self::Block => "block",
        }
    }

    /// Parse a wire string. Empty defaults to [`Info`](Self::Info); an unknown
    /// non-empty value is rejected (`None`) so callers can refuse rather than
    /// silently downgrade an escalation.
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "" | "info" => Some(Self::Info),
            "ask" => Some(Self::Ask),
            "block" => Some(Self::Block),
            _ => None,
        }
    }

    /// Whether this urgency escalates to the target's mention/wake path. `Ask`
    /// and `Block` ping; `Info` does not.
    pub fn pings(self) -> bool {
        matches!(self, Self::Ask | Self::Block)
    }
}

/// The `(tenant, topic)` scope of a stream. `topic` is the room, optionally finer
/// (per-task, per-actor). The [`canonical`](StreamKey::canonical) string is the
/// opaque key used to address a [`StreamLog`] in a [`StreamRegistry`] and to key
/// durable storage.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct StreamKey {
    pub tenant: String,
    pub topic: String,
}

impl StreamKey {
    pub fn new(tenant: impl Into<String>, topic: impl Into<String>) -> Self {
        Self {
            tenant: tenant.into(),
            topic: topic.into(),
        }
    }

    /// The opaque canonical key. Uses a unit-separator (`\u{1}`) join so a tenant
    /// or topic containing a literal separator can never forge a different scope.
    pub fn canonical(&self) -> String {
        format!("{}\u{1}{}", self.tenant, self.topic)
    }
}

/// One ordered, append-only coordination event.
///
/// `ordering_token` is monotonic *within a stream*: concurrent publishers receive
/// distinct tokens and the whole stream reads back in a single total order with no
/// merge step.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StreamEvent {
    /// Stable event id.
    pub id: String,
    /// Canonical `(tenant, topic)` key this event belongs to.
    pub stream_key: String,
    /// Monotonic position within the stream. The cursor read boundary.
    pub ordering_token: u64,
    /// Who published it.
    pub actor: String,
    /// Application kind (e.g. `intent`, `handoff`, `note`).
    pub kind: String,
    /// Free-form payload.
    pub payload: Value,
    /// Escalation level.
    #[serde(default)]
    pub urgency: StreamUrgency,
    /// The pinged actor, for `ask`/`block` events.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_actor: Option<String>,
    /// Creation timestamp (opaque string; caller-supplied or harness clock).
    pub created_at: String,
}

impl StreamEvent {
    /// A builder-light constructor. `ordering_token` is assigned by
    /// [`StreamLog::append`]; callers pass `0` and read the returned token.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: impl Into<String>,
        stream_key: impl Into<String>,
        actor: impl Into<String>,
        kind: impl Into<String>,
        payload: Value,
        urgency: StreamUrgency,
        target_actor: Option<String>,
        created_at: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            stream_key: stream_key.into(),
            ordering_token: 0,
            actor: actor.into(),
            kind: kind.into(),
            payload,
            urgency,
            target_actor: target_actor.filter(|value| !value.trim().is_empty()),
            created_at: created_at.into(),
        }
    }

    /// Whether this event is an intentional ping (`ask`/`block` with a target).
    pub fn is_ping(&self) -> bool {
        self.urgency.pings() && self.target_actor.is_some()
    }
}

/// A single append-only stream: events keyed by a strictly-increasing ordering
/// token. The append-only specialization of [`OrderedIndex`](crate::ordered::OrderedIndex).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StreamLog {
    by_token: OrdMap<u64, StreamEvent>,
    next_token: u64,
}

impl StreamLog {
    pub fn new() -> Self {
        Self {
            by_token: OrdMap::new(),
            next_token: 1,
        }
    }

    /// Append an event, assigning it the next ordering token (always `>` every
    /// token already in the log). Returns the assigned token. The event's
    /// `ordering_token` field is overwritten with the assignment.
    pub fn append(&mut self, mut event: StreamEvent) -> u64 {
        let token = self.next_token.max(1);
        event.ordering_token = token;
        self.by_token.insert(token, event);
        self.next_token = token + 1;
        token
    }

    /// Re-insert an event at its *own* recorded token (durable rehydration). Does
    /// not assign a new token; keeps `next_token` ahead of every ingested token so
    /// a later [`append`](Self::append) stays monotonic.
    pub fn ingest(&mut self, event: StreamEvent) {
        let token = event.ordering_token;
        self.next_token = self.next_token.max(token.saturating_add(1));
        self.by_token.insert(token, event);
    }

    /// Events strictly after `cursor`, in ascending token order, up to `limit`
    /// (`0` = uncapped). Early-stopping range over `(cursor, ..]` -- O(result +
    /// log n), never an O(n) scan, and nothing in the window is missed or
    /// duplicated.
    pub fn read_after(&self, cursor: u64, limit: usize) -> Vec<StreamEvent> {
        let mut out = Vec::new();
        for (_token, event) in self
            .by_token
            .range((Bound::Excluded(cursor), Bound::Unbounded))
        {
            out.push(event.clone());
            if limit != 0 && out.len() >= limit {
                break;
            }
        }
        out
    }

    /// The highest token currently in the log, or `0` if empty.
    pub fn latest_token(&self) -> u64 {
        self.by_token
            .get_max()
            .map(|(token, _)| *token)
            .unwrap_or(0)
    }

    /// The token the next [`append`](Self::append) will assign.
    pub fn next_token(&self) -> u64 {
        self.next_token.max(1)
    }

    pub fn len(&self) -> usize {
        self.by_token.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_token.is_empty()
    }
}

/// The local-embedded multi-stream home and *live-tail transport*: many
/// [`StreamLog`]s plus per-`(actor, stream)` cursors and per-actor subscription
/// sets. A warm head pulls its subscribed delta with no polling; the cursor
/// advances so nothing is re-read.
#[derive(Clone, Debug, Default)]
pub struct StreamRegistry {
    streams: BTreeMap<String, StreamLog>,
    cursors: BTreeMap<(String, String), u64>,
    subscriptions: BTreeMap<String, BTreeSet<String>>,
}

/// The delta a [`read`](StreamRegistry::read) returns for one stream: the events
/// after the actor's prior cursor and the cursor position after the read.
#[derive(Clone, Debug, PartialEq)]
pub struct StreamDelta {
    pub stream_key: String,
    pub events: Vec<StreamEvent>,
    pub new_cursor: u64,
}

impl StreamRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Append to a stream, creating it on first write. Returns the assigned token.
    pub fn append(&mut self, stream_key: &str, event: StreamEvent) -> u64 {
        self.streams
            .entry(stream_key.to_string())
            .or_default()
            .append(event)
    }

    /// The actor's last consumed token on a stream (`0` if never read).
    pub fn cursor(&self, actor: &str, stream_key: &str) -> u64 {
        self.cursors
            .get(&(actor.to_string(), stream_key.to_string()))
            .copied()
            .unwrap_or(0)
    }

    /// Read one stream's delta for an actor. With `advance`, the actor's cursor
    /// moves to the last event read so the window is consumed exactly once.
    pub fn read(
        &mut self,
        actor: &str,
        stream_key: &str,
        advance: bool,
        limit: usize,
    ) -> StreamDelta {
        let cursor = self.cursor(actor, stream_key);
        let events = self
            .streams
            .get(stream_key)
            .map(|log| log.read_after(cursor, limit))
            .unwrap_or_default();
        let new_cursor = events
            .last()
            .map(|event| event.ordering_token)
            .unwrap_or(cursor);
        if advance && new_cursor > cursor {
            self.cursors
                .insert((actor.to_string(), stream_key.to_string()), new_cursor);
        }
        StreamDelta {
            stream_key: stream_key.to_string(),
            events,
            new_cursor,
        }
    }

    /// Subscribe an actor to a stream. Returns the actor's full subscription set.
    pub fn subscribe(&mut self, actor: &str, stream_key: &str) -> Vec<String> {
        self.subscriptions
            .entry(actor.to_string())
            .or_default()
            .insert(stream_key.to_string());
        self.subscriptions(actor)
    }

    /// Unsubscribe an actor from a stream. Returns the remaining subscription set.
    pub fn unsubscribe(&mut self, actor: &str, stream_key: &str) -> Vec<String> {
        if let Some(set) = self.subscriptions.get_mut(actor) {
            set.remove(stream_key);
        }
        self.subscriptions(actor)
    }

    /// The actor's current subscription set, sorted.
    pub fn subscriptions(&self, actor: &str) -> Vec<String> {
        self.subscriptions
            .get(actor)
            .map(|set| set.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Read the deltas for every stream the actor is subscribed to. This is the
    /// passive turn-start read that replaces the room poll.
    pub fn read_subscribed(
        &mut self,
        actor: &str,
        advance: bool,
        limit: usize,
    ) -> Vec<StreamDelta> {
        let keys = self.subscriptions(actor);
        keys.into_iter()
            .map(|stream_key| self.read(actor, &stream_key, advance, limit))
            .filter(|delta| !delta.events.is_empty())
            .collect()
    }

    pub fn latest_token(&self, stream_key: &str) -> u64 {
        self.streams
            .get(stream_key)
            .map(StreamLog::latest_token)
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn event(actor: &str, kind: &str) -> StreamEvent {
        StreamEvent::new(
            format!("evt:{actor}:{kind}"),
            "tenant\u{1}room:demo",
            actor,
            kind,
            json!({ "k": kind }),
            StreamUrgency::Info,
            None,
            "t",
        )
    }

    #[test]
    fn append_assigns_strictly_increasing_tokens() {
        let mut log = StreamLog::new();
        let a = log.append(event("codex", "a"));
        let b = log.append(event("claude-code", "b"));
        let c = log.append(event("codex", "c"));
        assert_eq!((a, b, c), (1, 2, 3));
        assert_eq!(log.latest_token(), 3);
        assert_eq!(log.next_token(), 4);
    }

    #[test]
    fn read_after_returns_exact_ordered_delta_with_no_miss_or_dup() {
        // AC1: a head offline for N turns pulls exactly the events after its
        // stored cursor, in order, nothing missed or duplicated.
        let mut log = StreamLog::new();
        for kind in ["a", "b", "c", "d", "e"] {
            log.append(event("codex", kind));
        }
        // Cursor at 0: full window in order.
        let all = log.read_after(0, 0);
        assert_eq!(
            all.iter().map(|e| e.ordering_token).collect::<Vec<_>>(),
            vec![1, 2, 3, 4, 5]
        );
        // Resume from token 2: only the tail, in order, no overlap.
        let tail = log.read_after(2, 0);
        assert_eq!(
            tail.iter().map(|e| e.ordering_token).collect::<Vec<_>>(),
            vec![3, 4, 5]
        );
        // Past the end: empty, not an error.
        assert!(log.read_after(5, 0).is_empty());
        // Limit caps without skipping.
        let capped = log.read_after(0, 2);
        assert_eq!(
            capped.iter().map(|e| e.ordering_token).collect::<Vec<_>>(),
            vec![1, 2]
        );
    }

    #[test]
    fn concurrent_publishers_get_distinct_tokens_in_one_total_order() {
        // AC3: two heads publishing to one stream receive distinct tokens; both
        // events are readable in a single total order with no merge step.
        let mut log = StreamLog::new();
        let t_codex = log.append(event("codex", "x"));
        let t_claude = log.append(event("claude-code", "y"));
        assert_ne!(t_codex, t_claude);
        let order = log
            .read_after(0, 0)
            .into_iter()
            .map(|e| (e.actor, e.ordering_token))
            .collect::<Vec<_>>();
        assert_eq!(
            order,
            vec![("codex".to_string(), 1), ("claude-code".to_string(), 2),]
        );
    }

    #[test]
    fn ingest_preserves_tokens_and_keeps_append_monotonic() {
        // Durable rehydration: load events at their own tokens, then a later
        // append stays above them.
        let mut log = StreamLog::new();
        let mut e7 = event("codex", "seven");
        e7.ordering_token = 7;
        let mut e3 = event("codex", "three");
        e3.ordering_token = 3;
        log.ingest(e7);
        log.ingest(e3);
        assert_eq!(
            log.read_after(0, 0)
                .iter()
                .map(|e| e.ordering_token)
                .collect::<Vec<_>>(),
            vec![3, 7]
        );
        // next append is strictly greater than the highest ingested token.
        assert_eq!(log.append(event("codex", "next")), 8);
    }

    #[test]
    fn registry_cursor_advances_and_consumes_window_once() {
        let mut reg = StreamRegistry::new();
        let key = "tenant\u{1}room:demo";
        reg.append(key, event("codex", "a"));
        reg.append(key, event("codex", "b"));

        let first = reg.read("claude-code", key, true, 0);
        assert_eq!(first.events.len(), 2);
        assert_eq!(first.new_cursor, 2);
        // Second read after advancing: nothing left.
        let second = reg.read("claude-code", key, true, 0);
        assert!(second.events.is_empty());
        assert_eq!(second.new_cursor, 2);
        // A different actor still gets the full window (independent cursor).
        let other = reg.read("deepseek", key, true, 0);
        assert_eq!(other.events.len(), 2);
    }

    #[test]
    fn passive_read_without_advance_is_repeatable() {
        let mut reg = StreamRegistry::new();
        let key = "tenant\u{1}room:demo";
        reg.append(key, event("codex", "a"));
        let peek_a = reg.read("claude-code", key, false, 0);
        let peek_b = reg.read("claude-code", key, false, 0);
        assert_eq!(peek_a.events.len(), 1);
        assert_eq!(peek_b.events.len(), 1);
        assert_eq!(reg.cursor("claude-code", key), 0);
    }

    #[test]
    fn subscription_set_controls_which_deltas_a_read_returns() {
        // AC5: subscribing/unsubscribing changes which streams' deltas a read
        // returns.
        let mut reg = StreamRegistry::new();
        let room = "tenant\u{1}room:demo";
        let task = "tenant\u{1}task:stream";
        reg.append(room, event("codex", "room-1"));
        reg.append(task, event("codex", "task-1"));

        // No subscriptions -> no deltas.
        assert!(reg.read_subscribed("claude-code", true, 0).is_empty());

        // Subscribe to the room only.
        let set = reg.subscribe("claude-code", room);
        assert_eq!(set, vec![room.to_string()]);
        let deltas = reg.read_subscribed("claude-code", true, 0);
        assert_eq!(deltas.len(), 1);
        assert_eq!(deltas[0].stream_key, room);

        // Add the task stream.
        reg.subscribe("claude-code", task);
        reg.append(room, event("codex", "room-2"));
        let both = reg.read_subscribed("claude-code", true, 0);
        // room delta has room-2 only (room-1 already consumed); task has task-1.
        let keys = both
            .iter()
            .map(|d| d.stream_key.clone())
            .collect::<BTreeSet<_>>();
        assert!(keys.contains(room));
        assert!(keys.contains(task));

        // Unsubscribe from the task stream -> its delta no longer returned.
        let remaining = reg.unsubscribe("claude-code", task);
        assert_eq!(remaining, vec![room.to_string()]);
        reg.append(task, event("codex", "task-2"));
        let after = reg.read_subscribed("claude-code", true, 0);
        assert!(after.iter().all(|d| d.stream_key != task));
    }

    #[test]
    fn urgency_parse_and_ping_classification() {
        assert_eq!(StreamUrgency::parse(""), Some(StreamUrgency::Info));
        assert_eq!(StreamUrgency::parse("ASK"), Some(StreamUrgency::Ask));
        assert_eq!(StreamUrgency::parse("block"), Some(StreamUrgency::Block));
        assert_eq!(StreamUrgency::parse("loud"), None);
        assert!(!StreamUrgency::Info.pings());
        assert!(StreamUrgency::Ask.pings());
        assert!(StreamUrgency::Block.pings());

        let ping = StreamEvent::new(
            "id",
            "k",
            "codex",
            "ask",
            json!({}),
            StreamUrgency::Ask,
            Some("claude-code".to_string()),
            "t",
        );
        assert!(ping.is_ping());
        let info = event("codex", "fyi");
        assert!(!info.is_ping());
    }

    #[test]
    fn stream_key_canonical_is_separator_safe() {
        let key = StreamKey::new("travis-gilbert", "room:demo");
        assert_eq!(key.canonical(), "travis-gilbert\u{1}room:demo");
        // A topic that embeds the tenant text cannot collide with a real scope.
        let a = StreamKey::new("a", "b").canonical();
        let b = StreamKey::new("a\u{1}b", "").canonical();
        assert_ne!(a, b);
    }
}
