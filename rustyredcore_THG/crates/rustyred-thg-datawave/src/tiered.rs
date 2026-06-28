//! The cardinality-tiered global + field index (the read-side complement to
//! materialization), with boolean pushdown.
//!
//! North Star: "A small global index maps value plus field to fragment, and a
//! per-fragment field index resolves the rest above a cardinality threshold ...
//! lets queries intersect billions of field-level facts in bounded time without
//! the index growing combinatorially." DATAWAVE shape: the global index narrows
//! a value+field predicate to the fragments that contain it; the per-fragment
//! field index resolves the matching events within those fragments.
//!
//! The cardinality threshold keeps the global index small: a field whose distinct
//! normalized-value count exceeds the threshold is NOT globally indexed (its
//! values would bloat the global tier), so its predicates are resolved by the
//! per-fragment field index directly. Low-cardinality fields get cheap global
//! posting lists that prune fragments before the field index is touched.
//!
//! ponytail: posting lists are `BTreeSet<String>`; roaring bitmaps over interned
//! ids are the upgrade when a corpus's posting lists get large (the core already
//! vendors `roaring`). The structure here is in-memory and rebuildable from the
//! materialized `FieldFact` nodes.

use std::collections::{BTreeMap, BTreeSet};

use crate::field::NormalizedField;

type FieldValue = (String, String);

/// A tiered index over field-facts, partitioned into fragments (shards).
#[derive(Clone, Debug)]
pub struct TieredIndex {
    threshold: usize,
    /// Per-fragment field index: fragment -> (field, value) -> events.
    fragments: BTreeMap<String, BTreeMap<FieldValue, BTreeSet<String>>>,
    /// Distinct normalized values seen per field (its cardinality).
    field_values: BTreeMap<String, BTreeSet<String>>,
    /// The small global index: (field, value) -> fragments, for globally-indexed
    /// (low-cardinality) fields only. Built by `finalize`.
    global: BTreeMap<FieldValue, BTreeSet<String>>,
    globally_indexed: BTreeSet<String>,
    dirty: bool,
}

impl TieredIndex {
    /// `threshold` is the max distinct-value count for a field to be globally
    /// indexed. Above it, the field is resolved per-fragment.
    pub fn new(threshold: usize) -> Self {
        Self {
            threshold,
            fragments: BTreeMap::new(),
            field_values: BTreeMap::new(),
            global: BTreeMap::new(),
            globally_indexed: BTreeSet::new(),
            dirty: false,
        }
    }

    /// Index one event's field-facts into a fragment (shard).
    pub fn index_event(&mut self, fragment: &str, event_id: &str, fields: &[NormalizedField]) {
        let shard = self.fragments.entry(fragment.to_string()).or_default();
        for f in fields {
            let key = (f.field.clone(), f.normalized.clone());
            shard.entry(key).or_default().insert(event_id.to_string());
            self.field_values
                .entry(f.field.clone())
                .or_default()
                .insert(f.normalized.clone());
        }
        self.dirty = true;
    }

    /// (Re)build the global tier from the current cardinalities. Call after a
    /// batch of `index_event`s and before querying.
    pub fn finalize(&mut self) {
        self.global.clear();
        self.globally_indexed.clear();
        for (field, values) in &self.field_values {
            if values.len() <= self.threshold {
                self.globally_indexed.insert(field.clone());
            }
        }
        for (fragment, index) in &self.fragments {
            for (field, value) in index.keys() {
                if self.globally_indexed.contains(field) {
                    self.global
                        .entry((field.clone(), value.clone()))
                        .or_default()
                        .insert(fragment.clone());
                }
            }
        }
        self.dirty = false;
    }

    pub fn cardinality(&self, field: &str) -> usize {
        self.field_values.get(field).map(BTreeSet::len).unwrap_or(0)
    }

    pub fn is_globally_indexed(&self, field: &str) -> bool {
        self.globally_indexed.contains(field)
    }

    /// Number of entries in the small global index (its boundedness is the point:
    /// it does not grow with high-cardinality fields).
    pub fn global_entries(&self) -> usize {
        self.global.len()
    }

    /// Events matching `field == value`. Globally-indexed fields prune to their
    /// fragments first; high-cardinality fields scan the per-fragment field index.
    pub fn lookup(&self, field: &str, value: &str) -> BTreeSet<String> {
        debug_assert!(!self.dirty, "call finalize() before querying the tiered index");
        let key = (field.to_string(), value.to_string());
        let mut events = BTreeSet::new();
        // If the global tier is stale (index_event called after finalize, or never
        // finalized), fall back to the always-correct full scan rather than pruning
        // against an out-of-date fragment set. Correctness over speed when misused.
        if !self.dirty && self.globally_indexed.contains(field) {
            if let Some(fragments) = self.global.get(&key) {
                for fragment in fragments {
                    if let Some(index) = self.fragments.get(fragment) {
                        if let Some(hits) = index.get(&key) {
                            events.extend(hits.iter().cloned());
                        }
                    }
                }
            }
        } else {
            for index in self.fragments.values() {
                if let Some(hits) = index.get(&key) {
                    events.extend(hits.iter().cloned());
                }
            }
        }
        events
    }

    /// Boolean AND pushdown: events matching every `(field, value)` predicate.
    /// Intersects the smallest posting list first.
    pub fn intersect(&self, predicates: &[(&str, &str)]) -> BTreeSet<String> {
        if predicates.is_empty() {
            return BTreeSet::new();
        }
        let mut postings: Vec<BTreeSet<String>> =
            predicates.iter().map(|(f, v)| self.lookup(f, v)).collect();
        postings.sort_by_key(BTreeSet::len);
        let mut iter = postings.into_iter();
        let mut acc = iter.next().unwrap_or_default();
        for next in iter {
            acc.retain(|e| next.contains(e));
            if acc.is_empty() {
                break;
            }
        }
        acc
    }

    /// Boolean OR: events matching any `(field, value)` predicate.
    pub fn union(&self, predicates: &[(&str, &str)]) -> BTreeSet<String> {
        let mut acc = BTreeSet::new();
        for (f, v) in predicates {
            acc.extend(self.lookup(f, v));
        }
        acc
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::field::{FieldOrigin, FieldType, IndexPolicy};

    fn fact(field: &str, value: &str) -> NormalizedField {
        NormalizedField {
            field: field.to_string(),
            raw_value: value.to_string(),
            normalized: value.to_string(),
            group: None,
            visibility: None,
            masked: None,
            policy: IndexPolicy::INDEXED,
            field_type: FieldType::Text,
            origin: FieldOrigin::Extracted,
        }
    }

    fn sample_index() -> TieredIndex {
        let mut idx = TieredIndex::new(3);
        idx.index_event("f0", "e1", &[fact("proto", "tcp"), fact("port", "80"), fact("host", "a")]);
        idx.index_event("f0", "e2", &[fact("proto", "udp"), fact("port", "53"), fact("host", "b")]);
        idx.index_event("f1", "e3", &[fact("proto", "tcp"), fact("port", "443"), fact("host", "c")]);
        idx.index_event("f1", "e4", &[fact("proto", "tcp"), fact("port", "80"), fact("host", "d")]);
        idx.finalize();
        idx
    }

    #[test]
    fn cardinality_threshold_decides_global_indexing() {
        let idx = sample_index();
        // proto {tcp,udp}=2 and port {80,53,443}=3 are <= threshold 3: globally indexed.
        assert!(idx.is_globally_indexed("proto"));
        assert!(idx.is_globally_indexed("port"));
        // host {a,b,c,d}=4 exceeds the threshold: NOT globally indexed.
        assert!(!idx.is_globally_indexed("host"));
        assert_eq!(idx.cardinality("host"), 4);
    }

    #[test]
    fn lookup_resolves_both_tiers() {
        let idx = sample_index();
        // Globally-indexed field: pruned via the global tier.
        assert_eq!(idx.lookup("proto", "tcp"), set(["e1", "e3", "e4"]));
        // High-cardinality field: resolved by the per-fragment field index.
        assert_eq!(idx.lookup("host", "a"), set(["e1"]));
        assert!(idx.lookup("proto", "sctp").is_empty());
    }

    #[test]
    fn intersect_and_union_push_down_booleans() {
        let idx = sample_index();
        // tcp AND port 80 -> e1, e4 (not e3 which is 443, not e2 which is udp).
        assert_eq!(idx.intersect(&[("proto", "tcp"), ("port", "80")]), set(["e1", "e4"]));
        // udp OR port 443 -> e2 (udp) and e3 (443).
        assert_eq!(idx.union(&[("proto", "udp"), ("port", "443")]), set(["e2", "e3"]));
    }

    #[test]
    fn global_index_stays_bounded_under_high_cardinality() {
        // Many distinct host values must NOT enter the global tier.
        let mut idx = TieredIndex::new(3);
        for i in 0..1000 {
            idx.index_event("f0", &format!("e{i}"), &[fact("proto", "tcp"), fact("host", &format!("h{i}"))]);
        }
        idx.finalize();
        // proto (card 1) contributes one global entry; host (card 1000) contributes none.
        assert!(!idx.is_globally_indexed("host"));
        assert_eq!(idx.global_entries(), 1, "only proto=tcp is globally indexed");
    }

    fn set<const N: usize>(items: [&str; N]) -> BTreeSet<String> {
        items.iter().map(|s| s.to_string()).collect()
    }
}
