//! The workspace presence registry for the agent co-presence layer (spec:
//! `~/Downloads/HANDOFF-AGENT-COEDIT-PRESENCE-LAYER.md`).
//!
//! Concurrent agent processes (Claude Code, Codex) edit the same repo. Instead of
//! probing the filesystem (mtime / `pgrep`) or claiming file locks, each agent
//! ambiently announces *where it is* (cursor) and *what it is about to change*
//! (a pending-edit footprint) over the one local instance, and reads peers' state
//! before writing. Coordination shifts from a mutex (file ownership) to presence
//! (concurrent); the residual coordination role narrows to semantic ownership.
//!
//! # Awareness only -- bytes still merge via git
//!
//! This layer NEVER writes file bytes. The `theorem-copresence` code surface
//! already decided [`CodeContentStrategy::GitMergeOnly`]: source bytes merge
//! through W2 git (`apps/rustyred-git`), not through this presence layer. We reuse
//! that crate's public value types ([`FileRange`], [`CodeEditFootprint`],
//! [`CodePresenceSnapshot`], [`PresenceKind`]) so the model is not reinvented; the
//! registry is the cross-process aggregation + overlap query the public surface
//! does not have (its `CodeSurfaceAdapter` is per-file and in-process).
//!
//! # Cross-process sharing
//!
//! The registry is in-memory in THIS process. Cross-process sharing works because
//! the agents are HTTP clients of this one local instance (the control endpoint):
//! each agent's hook announces over `POST /v1/presence`, sets a footprint over
//! `POST /v1/presence/footprint`, and tests its intended range over
//! `POST /v1/presence/would-overlap` -- all against the same registry. The remote
//! harness room being offline does not matter; the local instance is the hub.
//!
//! # The key query
//!
//! [`PresenceRegistry::would_overlap`] takes an `(path, intended: FileRange)` and
//! returns the peers' pending-edit footprints that overlap the intended range on
//! the same path, EXCLUDING the caller's own footprint. A PreToolUse(Edit) hook
//! calls this before any write: a non-empty result means a peer is about to touch
//! overlapping lines, so the hook can warn (bytes still merge via git) or serialize.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use rustyred_thg_core::ActorId;
use serde::{Deserialize, Serialize};

pub use theorem_copresence::{CodeEditFootprint, CodePresenceSnapshot, FileRange, PresenceKind};

/// An agent's announced presence in the workspace: which file it is focused on and
/// where its cursor sits, plus a human-facing label and whether it is a human or
/// an agent. One agent can be present in multiple files at once (keyed by path), so
/// presences are stored per `(actor, path)`.
///
/// This mirrors [`CodePresenceSnapshot`]'s shape (we reuse that type for the wire
/// surface) but carries the registry's own `updated_at_ms` for last-writer-wins on
/// re-announce.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AgentPresence {
    /// The announcing actor (an opaque id; an agent derives it from a stable label
    /// like "claude-code" / "codex").
    pub actor: ActorId,
    /// The file path the agent is present in (workspace-relative, the agent's own
    /// convention; the registry treats it as an opaque key).
    pub path: String,
    /// Cursor line (1-based by convention; the registry does not interpret it).
    pub line: u32,
    /// Cursor column.
    pub col: u32,
    /// Human-facing label for the presence (e.g. "Claude Code").
    pub label: String,
    /// Whether this is a human or an agent.
    pub kind: PresenceKind,
    /// When this presence was last announced/updated (epoch ms), for LWW.
    pub updated_at_ms: i64,
}

impl AgentPresence {
    /// Project to the `theorem-copresence` wire snapshot type. The `cursor` field
    /// there is the crate's monotonic per-actor cursor token; this registry does
    /// not maintain HLC cursors (it is the local hub, not a CRDT peer), so we
    /// surface `updated_at_ms` as the ordering token, which is monotonic per actor
    /// under last-writer-wins and lets a consumer sort presences by recency.
    pub fn to_snapshot(&self) -> CodePresenceSnapshot {
        CodePresenceSnapshot {
            actor: self.actor,
            path: self.path.clone(),
            line: self.line,
            col: self.col,
            label: self.label.clone(),
            kind: self.kind.clone(),
            cursor: self.updated_at_ms.max(0) as u64,
        }
    }
}

/// A new presence announcement (the registry stamps `updated_at_ms`).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PresenceAnnouncement {
    pub actor: ActorId,
    pub path: String,
    pub line: u32,
    pub col: u32,
    pub label: String,
    pub kind: PresenceKind,
}

/// A pending-edit footprint announcement: the range an agent is about to edit on a
/// path, with an optional summary. The registry stores it keyed by `(actor, path)`
/// and stamps it for staleness reasoning by the caller.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct FootprintAnnouncement {
    pub actor: ActorId,
    pub path: String,
    pub range: FileRange,
    #[serde(default)]
    pub summary: Option<String>,
}

/// A stored pending-edit footprint plus its stamp.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct StoredFootprint {
    range: FileRange,
    summary: Option<String>,
    updated_at_ms: i64,
}

/// Composite key for both presences and footprints: an `(actor, path)` pair, so
/// multiple agents and multiple files coexist and a per-agent-per-file entry is
/// replaced (not duplicated) on re-announce.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct ActorPath {
    actor: ActorId,
    path: String,
}

/// Shared registry internals behind an `Arc` so the registry is a cheap cloneable
/// handle the HTTP layer shares across requests (same pattern as
/// [`crate::runs::RunRegistry`]).
struct PresenceInner {
    presences: Mutex<BTreeMap<ActorPath, AgentPresence>>,
    footprints: Mutex<BTreeMap<ActorPath, StoredFootprint>>,
}

/// The workspace presence registry: announce/update presence, set/clear a
/// pending-edit footprint, list current presences + footprints, and the key
/// [`would_overlap`](Self::would_overlap) query.
///
/// Cloneable handle (state is `Arc`-backed) so the control router shares one
/// registry across handlers, and all agent processes that talk to this instance
/// see the same workspace presence.
#[derive(Clone)]
pub struct PresenceRegistry {
    inner: Arc<PresenceInner>,
}

impl Default for PresenceRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl PresenceRegistry {
    /// Build an empty registry.
    pub fn new() -> Self {
        Self {
            inner: Arc::new(PresenceInner {
                presences: Mutex::new(BTreeMap::new()),
                footprints: Mutex::new(BTreeMap::new()),
            }),
        }
    }

    /// Announce (or update) an agent's presence on a path. Last-writer-wins per
    /// `(actor, path)`: a re-announce replaces the prior cursor for that agent in
    /// that file rather than accumulating. Returns the stored presence (with the
    /// stamp the registry assigned).
    pub fn announce(&self, announcement: PresenceAnnouncement) -> AgentPresence {
        let presence = AgentPresence {
            actor: announcement.actor,
            path: announcement.path.clone(),
            line: announcement.line,
            col: announcement.col,
            label: announcement.label,
            kind: announcement.kind,
            updated_at_ms: now_ms(),
        };
        let key = ActorPath {
            actor: announcement.actor,
            path: announcement.path,
        };
        self.inner
            .presences
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(key, presence.clone());
        presence
    }

    /// Clear an agent's presence on a path (e.g. when it closes the file). Returns
    /// whether a presence existed and was removed. Idempotent.
    pub fn clear_presence(&self, actor: ActorId, path: &str) -> bool {
        let key = ActorPath {
            actor,
            path: path.to_string(),
        };
        self.inner
            .presences
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&key)
            .is_some()
    }

    /// Set (or replace) an agent's pending-edit footprint on a path. Last-writer-
    /// wins per `(actor, path)`. Returns the footprint as the wire type.
    pub fn set_footprint(&self, announcement: FootprintAnnouncement) -> CodeEditFootprint {
        let key = ActorPath {
            actor: announcement.actor,
            path: announcement.path.clone(),
        };
        let stored = StoredFootprint {
            range: announcement.range.clone(),
            summary: announcement.summary.clone(),
            updated_at_ms: now_ms(),
        };
        self.inner
            .footprints
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(key, stored);
        CodeEditFootprint {
            actor: announcement.actor,
            path: announcement.path,
            range: announcement.range,
            summary: announcement.summary,
        }
    }

    /// Clear an agent's pending-edit footprint on a path (PostToolUse(Edit) calls
    /// this once the write is done). Returns whether a footprint existed and was
    /// removed. Idempotent, so a hook that clears with no prior set is harmless.
    pub fn clear_footprint(&self, actor: ActorId, path: &str) -> bool {
        let key = ActorPath {
            actor,
            path: path.to_string(),
        };
        self.inner
            .footprints
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&key)
            .is_some()
    }

    /// List all current presences across every agent and file, ordered by
    /// `(actor, path)`.
    pub fn list_presences(&self) -> Vec<AgentPresence> {
        self.inner
            .presences
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .values()
            .cloned()
            .collect()
    }

    /// List all current pending-edit footprints across every agent and file, as the
    /// `theorem-copresence` wire type, ordered by `(actor, path)`.
    pub fn list_footprints(&self) -> Vec<CodeEditFootprint> {
        self.inner
            .footprints
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .map(|(key, stored)| CodeEditFootprint {
                actor: key.actor,
                path: key.path.clone(),
                range: stored.range.clone(),
                summary: stored.summary.clone(),
            })
            .collect()
    }

    /// The key query: peers' pending-edit footprints that overlap `intended` on
    /// `path`, EXCLUDING the caller's own footprint.
    ///
    /// `caller` is the actor about to edit; its own footprint on `path` (if any) is
    /// excluded so re-running the query after setting your own intended footprint
    /// does not flag yourself. Overlap is line/col range intersection on the same
    /// path (see [`ranges_overlap`]).
    ///
    /// A non-empty result means another agent has announced an intent to edit
    /// overlapping lines: the caller's hook can warn (bytes still merge via git +
    /// the freshness guard) or serialize. Results are ordered by `(actor, path)`.
    pub fn would_overlap(
        &self,
        caller: ActorId,
        path: &str,
        intended: &FileRange,
    ) -> Vec<CodeEditFootprint> {
        self.inner
            .footprints
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .filter(|(key, _)| key.path == path && key.actor != caller)
            .filter(|(_, stored)| ranges_overlap(&stored.range, intended))
            .map(|(key, stored)| CodeEditFootprint {
                actor: key.actor,
                path: key.path.clone(),
                range: stored.range.clone(),
                summary: stored.summary.clone(),
            })
            .collect()
    }
}

/// Whether two file ranges overlap, treating each as a closed `[start, end]`
/// span over the linear `(line, col)` ordering. Two edits overlap when neither
/// ends strictly before the other begins.
///
/// The comparison is on the flat (line, col) position so a multi-line range
/// intersects correctly: range A `[ (a.start_line, a.start_col), (a.end_line,
/// a.end_col) ]` overlaps range B unless A ends before B starts or B ends before
/// A starts. A zero-width range (start == end, an insertion point) still overlaps
/// a span that contains it, which is the conservative ("warn") choice for an
/// awareness layer.
pub fn ranges_overlap(a: &FileRange, b: &FileRange) -> bool {
    let a_start = (a.start_line, a.start_col);
    let a_end = (a.end_line, a.end_col);
    let b_start = (b.start_line, b.start_col);
    let b_end = (b.end_line, b.end_col);
    // Disjoint iff one ends strictly before the other starts.
    !(a_end < b_start || b_end < a_start)
}

/// Current wall-clock in epoch ms, reusing the core clock so stamps line up with
/// the rest of the runtime's provenance.
fn now_ms() -> i64 {
    rustyred_thg_core::now_ms()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn range(sl: u32, sc: u32, el: u32, ec: u32) -> FileRange {
        FileRange::new(sl, sc, el, ec)
    }

    fn announce(actor: &str, path: &str, line: u32, col: u32) -> PresenceAnnouncement {
        PresenceAnnouncement {
            actor: ActorId::from_label(actor),
            path: path.to_string(),
            line,
            col,
            label: actor.to_string(),
            kind: PresenceKind::Agent,
        }
    }

    fn footprint(actor: &str, path: &str, r: FileRange, summary: &str) -> FootprintAnnouncement {
        FootprintAnnouncement {
            actor: ActorId::from_label(actor),
            path: path.to_string(),
            range: r,
            summary: Some(summary.to_string()),
        }
    }

    #[test]
    fn ranges_overlap_detects_intersection_and_disjoint() {
        // Same lines, overlapping columns.
        assert!(ranges_overlap(&range(10, 0, 10, 20), &range(10, 15, 10, 30)));
        // Adjacent-but-touching (A ends exactly where B starts) -> overlap (closed).
        assert!(ranges_overlap(&range(10, 0, 10, 15), &range(10, 15, 10, 30)));
        // Disjoint on the same line (gap between them).
        assert!(!ranges_overlap(&range(10, 0, 10, 10), &range(10, 11, 10, 20)));
        // Multi-line containment.
        assert!(ranges_overlap(&range(5, 0, 20, 0), &range(10, 0, 10, 5)));
        // Disjoint across lines.
        assert!(!ranges_overlap(&range(1, 0, 5, 0), &range(6, 0, 9, 0)));
        // Zero-width insertion point inside a span.
        assert!(ranges_overlap(&range(8, 4, 8, 4), &range(5, 0, 12, 0)));
    }

    #[test]
    fn two_agents_overlapping_pending_edits_flagged_excluding_self() {
        let registry = PresenceRegistry::new();
        // Both agents announce presence on the same file.
        registry.announce(announce("claude-code", "src/lib.rs", 10, 0));
        registry.announce(announce("codex", "src/lib.rs", 40, 0));

        // Both set a pending-edit footprint; the ranges overlap (lines 10-20 vs 15-25).
        registry.set_footprint(footprint(
            "claude-code",
            "src/lib.rs",
            range(10, 0, 20, 0),
            "refactor fn a",
        ));
        registry.set_footprint(footprint(
            "codex",
            "src/lib.rs",
            range(15, 0, 25, 0),
            "rename fn b",
        ));

        // Claude Code, about to edit lines 10-20, queries: it must see CODEX's
        // overlapping footprint and NOT its own.
        let claude = ActorId::from_label("claude-code");
        let overlaps = registry.would_overlap(claude, "src/lib.rs", &range(10, 0, 20, 0));
        assert_eq!(overlaps.len(), 1, "exactly the peer's footprint is flagged");
        assert_eq!(
            overlaps[0].actor,
            ActorId::from_label("codex"),
            "the flagged footprint is the peer's, not the caller's own"
        );
        assert_eq!(overlaps[0].summary.as_deref(), Some("rename fn b"));
    }

    #[test]
    fn non_overlapping_ranges_return_empty() {
        let registry = PresenceRegistry::new();
        registry.set_footprint(footprint(
            "codex",
            "src/lib.rs",
            range(40, 0, 50, 0),
            "edit tail",
        ));
        // Claude intends to edit lines 10-20; the peer's footprint is at 40-50.
        let claude = ActorId::from_label("claude-code");
        let overlaps = registry.would_overlap(claude, "src/lib.rs", &range(10, 0, 20, 0));
        assert!(
            overlaps.is_empty(),
            "non-overlapping ranges on the same path do not flag"
        );
    }

    #[test]
    fn overlap_is_scoped_to_the_same_path() {
        let registry = PresenceRegistry::new();
        // Peer's footprint overlaps line-wise but is on a DIFFERENT file.
        registry.set_footprint(footprint(
            "codex",
            "src/other.rs",
            range(10, 0, 20, 0),
            "other file",
        ));
        let claude = ActorId::from_label("claude-code");
        let overlaps = registry.would_overlap(claude, "src/lib.rs", &range(10, 0, 20, 0));
        assert!(
            overlaps.is_empty(),
            "a same-range footprint on another path does not flag"
        );
    }

    #[test]
    fn presence_list_reflects_announces_and_clears() {
        let registry = PresenceRegistry::new();
        registry.announce(announce("claude-code", "src/lib.rs", 10, 0));
        registry.announce(announce("codex", "src/main.rs", 5, 2));
        // Same agent in a second file is a distinct presence (keyed by path).
        registry.announce(announce("claude-code", "src/main.rs", 99, 0));

        let presences = registry.list_presences();
        assert_eq!(presences.len(), 3, "three distinct (actor, path) presences");

        // Re-announce on an existing (actor, path) replaces, not duplicates.
        registry.announce(announce("claude-code", "src/lib.rs", 12, 4));
        let after = registry.list_presences();
        assert_eq!(after.len(), 3, "re-announce on the same file replaces in place");
        let lib_presence = after
            .iter()
            .find(|p| p.actor == ActorId::from_label("claude-code") && p.path == "src/lib.rs")
            .expect("claude-code presence on src/lib.rs");
        assert_eq!((lib_presence.line, lib_presence.col), (12, 4), "cursor updated");

        // Clearing removes exactly that one.
        assert!(registry.clear_presence(ActorId::from_label("codex"), "src/main.rs"));
        assert_eq!(registry.list_presences().len(), 2);
        // Clearing again is idempotent (false, no panic).
        assert!(!registry.clear_presence(ActorId::from_label("codex"), "src/main.rs"));
    }

    #[test]
    fn clearing_a_footprint_removes_it_from_overlap() {
        let registry = PresenceRegistry::new();
        registry.set_footprint(footprint(
            "codex",
            "src/lib.rs",
            range(10, 0, 20, 0),
            "pending",
        ));
        let claude = ActorId::from_label("claude-code");
        assert_eq!(
            registry
                .would_overlap(claude, "src/lib.rs", &range(12, 0, 14, 0))
                .len(),
            1,
            "the peer footprint is live before clear"
        );

        // PostToolUse(Edit): the peer clears its footprint.
        assert!(registry.clear_footprint(ActorId::from_label("codex"), "src/lib.rs"));
        assert!(
            registry
                .would_overlap(claude, "src/lib.rs", &range(12, 0, 14, 0))
                .is_empty(),
            "a cleared footprint no longer overlaps"
        );
        // list_footprints reflects the clear.
        assert!(registry.list_footprints().is_empty());
    }
}
