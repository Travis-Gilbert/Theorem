use std::cmp::Ordering;
use std::collections::BTreeMap;

use imbl::{HashMap as ImHashMap, OrdMap};
use serde::{Deserialize, Serialize};

use crate::graph_store::{GraphStoreError, GraphStoreResult};

pub type OrderedMember = Vec<u8>;

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub struct OrderedScore(f64);

impl OrderedScore {
    pub fn new(score: f64) -> GraphStoreResult<Self> {
        if score.is_nan() {
            return Err(GraphStoreError::new(
                "invalid_ordered_score",
                "ordered scores must not be NaN".to_string(),
            ));
        }
        Ok(Self(score))
    }

    pub fn get(self) -> f64 {
        self.0
    }
}

impl Eq for OrderedScore {}

impl Ord for OrderedScore {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0.total_cmp(&other.0)
    }
}

impl PartialOrd for OrderedScore {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OrderedMode {
    Persistent,
    Transient,
}

impl Default for OrderedMode {
    fn default() -> Self {
        Self::Persistent
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct OrderedDesignation {
    pub label: String,
    pub property: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct OrderedIndex {
    by_member: ImHashMap<OrderedMember, f64>,
    by_score: OrdMap<(OrderedScore, OrderedMember), ()>,
    mode: OrderedMode,
}

impl Default for OrderedIndex {
    fn default() -> Self {
        Self::new(OrderedMode::Persistent)
    }
}

impl OrderedIndex {
    pub fn new(mode: OrderedMode) -> Self {
        Self {
            by_member: ImHashMap::new(),
            by_score: OrdMap::new(),
            mode,
        }
    }

    pub fn persistent() -> Self {
        Self::new(OrderedMode::Persistent)
    }

    pub fn transient() -> Self {
        Self::new(OrderedMode::Transient)
    }

    pub fn mode(&self) -> OrderedMode {
        self.mode
    }

    pub fn zadd(&mut self, member: impl Into<OrderedMember>, score: f64) -> GraphStoreResult<bool> {
        let member = member.into();
        if member.is_empty() {
            return Err(GraphStoreError::new(
                "invalid_ordered_member",
                "ordered member must not be empty".to_string(),
            ));
        }
        let ordered_score = OrderedScore::new(score)?;
        let inserted = !self.by_member.contains_key(&member);
        if let Some(previous) = self.by_member.insert(member.clone(), score) {
            let previous_score = OrderedScore::new(previous)?;
            self.by_score.remove(&(previous_score, member.clone()));
        }
        self.by_score.insert((ordered_score, member), ());
        Ok(inserted)
    }

    pub fn zscore(&self, member: &[u8]) -> Option<f64> {
        self.by_member.get(member).copied()
    }

    pub fn zpop_min(&mut self) -> Option<(OrderedMember, f64)> {
        let ((score, member), _) = self
            .by_score
            .iter()
            .next()
            .map(|(key, value)| (key, value))?;
        let score = score.get();
        let member = member.clone();
        self.by_score
            .remove(&(OrderedScore::new(score).ok()?, member.clone()));
        self.by_member.remove(&member);
        Some((member, score))
    }

    pub fn zpop_max(&mut self) -> Option<(OrderedMember, f64)> {
        // `next_back()` on the double-ended OrdMap iterator descends to the
        // largest key in O(log n). (`Iterator::last()` would walk every entry,
        // O(n) -- fatal for the crawl frontier's pop-the-next-URL hot path.)
        let ((score, member), _) = self.by_score.iter().next_back()?;
        let score = score.get();
        let member = member.clone();
        self.by_score
            .remove(&(OrderedScore::new(score).ok()?, member.clone()));
        self.by_member.remove(&member);
        Some((member, score))
    }

    pub fn zrange_by_score(
        &self,
        min: f64,
        max: f64,
        limit: Option<usize>,
    ) -> GraphStoreResult<Vec<(OrderedMember, f64)>> {
        let min = OrderedScore::new(min)?;
        let max = OrderedScore::new(max)?;
        if min > max {
            return Ok(Vec::new());
        }
        let cap = limit.unwrap_or(usize::MAX);
        let mut out = Vec::new();
        for ((score, member), _) in self.by_score.iter() {
            if *score < min || *score > max {
                continue;
            }
            out.push((member.clone(), score.get()));
            if out.len() >= cap {
                break;
            }
        }
        Ok(out)
    }

    /// Ascending members with `score <= max_score`, up to `limit` (0 = no cap).
    /// Early-stops at the first score above `max_score`: the index is
    /// score-ordered, so this is O(result + log n), NOT the O(n) filter-every-
    /// entry walk `zrange_by_score` does. This is the eviction frontier's
    /// "coldest k below the cutoff" read, the pop-not-scan property the storage
    /// spine depends on.
    pub fn range_to(&self, max_score: f64, limit: usize) -> Vec<(OrderedMember, f64)> {
        let mut out = Vec::new();
        for ((score, member), _) in self.by_score.iter() {
            if score.get() > max_score {
                break;
            }
            out.push((member.clone(), score.get()));
            if limit != 0 && out.len() >= limit {
                break;
            }
        }
        out
    }

    pub fn zrem(&mut self, member: &[u8]) -> bool {
        let Some(score) = self.by_member.remove(member) else {
            return false;
        };
        if let Ok(score) = OrderedScore::new(score) {
            self.by_score.remove(&(score, member.to_vec()));
        }
        true
    }

    pub fn zcard(&self) -> usize {
        self.by_member.len()
    }

    pub fn zrank(&self, member: &[u8]) -> Option<usize> {
        self.by_score
            .iter()
            .enumerate()
            .find_map(|(rank, ((_, candidate), _))| {
                (candidate.as_slice() == member).then_some(rank)
            })
    }

    pub fn entries(&self) -> Vec<(OrderedMember, f64)> {
        self.by_score
            .iter()
            .map(|((score, member), _)| (member.clone(), score.get()))
            .collect()
    }

    pub fn entries_desc(&self, limit: usize) -> Vec<(OrderedMember, f64)> {
        self.by_score
            .iter()
            .rev()
            .take(limit)
            .map(|((score, member), _)| (member.clone(), score.get()))
            .collect()
    }
}

#[derive(Clone, Debug, Default)]
pub struct OrderedIndexRegistry {
    indexes: BTreeMap<String, OrderedIndex>,
}

impl OrderedIndexRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn index_mut(&mut self, name: &str, mode: OrderedMode) -> &mut OrderedIndex {
        self.indexes
            .entry(name.to_string())
            .or_insert_with(|| OrderedIndex::new(mode))
    }

    pub fn index(&self, name: &str) -> Option<&OrderedIndex> {
        self.indexes.get(name)
    }
}

/// The eviction frontier (storage spine, cut 6): a persistent, per-scope
/// [`OrderedIndex`] over `last_accessed_ms`. It turns eviction from an O(n) walk
/// of every memory node into O(k log n) ordered-index work over just the coldest
/// k below a cutoff.
///
/// - `recall()` is the zadd site: it already writes `last_accessed_ms` on every
///   access, so it [`touch`](Self::touch)es the node here on each recall.
/// - `decay()` reads the coldest tail via [`coldest_below`](Self::coldest_below)
///   and [`forget`](Self::forget)s each node it actually evicts.
///
/// `ops()` is the cumulative ordered-index operation count -- the analogue of
/// the chunk-visit counter [`diff_graph_trees`](crate::versioned_graph::diff_graph_trees)
/// returns -- so an acceptance test can prove eviction is a pop, not a scan.
#[derive(Clone, Debug, Default)]
pub struct EvictionFrontier {
    scopes: BTreeMap<String, OrderedIndex>,
    ops: usize,
}

impl EvictionFrontier {
    pub fn new() -> Self {
        Self::default()
    }

    /// zadd: record or refresh a node's coldness key (`last_accessed_ms`) on
    /// access. Counts one ordered-index op.
    pub fn touch(&mut self, scope: &str, id: &str, last_accessed_ms: i64) -> GraphStoreResult<()> {
        self.ops += 1;
        self.scopes
            .entry(scope.to_string())
            .or_insert_with(OrderedIndex::persistent)
            .zadd(id.as_bytes().to_vec(), last_accessed_ms as f64)?;
        Ok(())
    }

    /// zrem: drop a node from the frontier (it was evicted to, or rehydrated
    /// out of, the cold tail). Counts one ordered-index op.
    pub fn forget(&mut self, scope: &str, id: &str) -> bool {
        self.ops += 1;
        self.scopes
            .get_mut(scope)
            .map(|index| index.zrem(id.as_bytes()))
            .unwrap_or(false)
    }

    /// The coldest members in `scope` whose `last_accessed_ms <= cutoff`, up to
    /// `limit` (0 = uncapped). O(result + log n) via an early-stopping ascending
    /// range -- never a full scan. Each visited entry counts as one op.
    pub fn coldest_below(&mut self, scope: &str, cutoff: i64, limit: usize) -> Vec<(String, f64)> {
        let Some(index) = self.scopes.get(scope) else {
            self.ops += 1;
            return Vec::new();
        };
        let entries = index.range_to(cutoff as f64, limit);
        self.ops += entries.len().max(1);
        entries
            .into_iter()
            .filter_map(|(member, score)| String::from_utf8(member).ok().map(|id| (id, score)))
            .collect()
    }

    /// Cumulative ordered-index operation count since the last [`reset_ops`](Self::reset_ops).
    pub fn ops(&self) -> usize {
        self.ops
    }

    pub fn reset_ops(&mut self) {
        self.ops = 0;
    }

    pub fn len(&self, scope: &str) -> usize {
        self.scopes.get(scope).map(OrderedIndex::zcard).unwrap_or(0)
    }

    pub fn is_empty(&self, scope: &str) -> bool {
        self.len(scope) == 0
    }

    pub fn score(&self, scope: &str, id: &str) -> Option<f64> {
        self.scopes
            .get(scope)
            .and_then(|index| index.zscore(id.as_bytes()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(member: &[u8]) -> String {
        String::from_utf8(member.to_vec()).unwrap()
    }

    #[test]
    fn zset_semantics_reject_nan_and_tie_break_by_member() {
        let mut index = OrderedIndex::transient();
        assert!(index.zadd(b"b".to_vec(), 1.0).unwrap());
        assert!(index.zadd(b"a".to_vec(), 1.0).unwrap());
        assert!(index.zadd(b"c".to_vec(), 2.0).unwrap());

        let entries = index
            .zrange_by_score(0.0, 10.0, None)
            .unwrap()
            .into_iter()
            .map(|(member, score)| (text(&member), score))
            .collect::<Vec<_>>();
        assert_eq!(
            entries,
            vec![
                ("a".to_string(), 1.0),
                ("b".to_string(), 1.0),
                ("c".to_string(), 2.0),
            ]
        );
        assert_eq!(index.zrank(b"a"), Some(0));
        assert_eq!(index.zrank(b"b"), Some(1));
        assert_eq!(
            index.zadd(b"nan".to_vec(), f64::NAN).unwrap_err().code,
            "invalid_ordered_score"
        );
    }

    #[test]
    fn zadd_update_moves_score_entry_and_pop_max_returns_highest() {
        let mut index = OrderedIndex::transient();
        assert!(index.zadd(b"url:1".to_vec(), 1.0).unwrap());
        assert!(!index.zadd(b"url:1".to_vec(), 5.0).unwrap());
        assert!(index.zadd(b"url:2".to_vec(), 3.0).unwrap());

        assert_eq!(index.zcard(), 2);
        assert_eq!(index.zscore(b"url:1"), Some(5.0));
        assert_eq!(index.zpop_max(), Some((b"url:1".to_vec(), 5.0)));
        assert_eq!(index.zpop_min(), Some((b"url:2".to_vec(), 3.0)));
        assert_eq!(index.zpop_min(), None);
    }

    #[test]
    fn entries_desc_limits_highest_score_side_with_member_tie_break() {
        let mut index = OrderedIndex::transient();
        assert!(index.zadd(b"a".to_vec(), 1.0).unwrap());
        assert!(index.zadd(b"c".to_vec(), 1.0).unwrap());
        assert!(index.zadd(b"b".to_vec(), 1.0).unwrap());
        assert!(index.zadd(b"top".to_vec(), 2.0).unwrap());

        let entries = index
            .entries_desc(3)
            .into_iter()
            .map(|(member, score)| (text(&member), score))
            .collect::<Vec<_>>();
        assert_eq!(
            entries,
            vec![
                ("top".to_string(), 2.0),
                ("c".to_string(), 1.0),
                ("b".to_string(), 1.0),
            ]
        );
    }
}
