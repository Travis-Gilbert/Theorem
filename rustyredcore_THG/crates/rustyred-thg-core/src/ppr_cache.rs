//! Scoped cache-aside helpers for structural PPR vectors.
//!
//! The cache key carries the graph version, so graph writes naturally miss old
//! structural priors. Callers still apply volatile multipliers such as fitness
//! and activation after PPR.

use std::collections::{HashMap, VecDeque};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::graph::personalized_pagerank;
use crate::state::stable_hash;

const DEFAULT_SCOPED_PPR_CACHE_CAPACITY: usize = 256;
const DEFAULT_SCOPED_PPR_CACHE_TTL: Duration = Duration::from_secs(300);

#[derive(Clone, Debug)]
struct CachedPprEntry {
    scores: HashMap<String, f64>,
    inserted_at: Instant,
}

#[derive(Debug)]
struct ScopedPprCache {
    entries: HashMap<String, CachedPprEntry>,
    order: VecDeque<String>,
    capacity: usize,
    ttl: Duration,
}

impl Default for ScopedPprCache {
    fn default() -> Self {
        Self {
            entries: HashMap::new(),
            order: VecDeque::new(),
            capacity: DEFAULT_SCOPED_PPR_CACHE_CAPACITY,
            ttl: DEFAULT_SCOPED_PPR_CACHE_TTL,
        }
    }
}

impl ScopedPprCache {
    fn get(&mut self, key: &str) -> Option<HashMap<String, f64>> {
        let Some(entry) = self.entries.get(key) else {
            return None;
        };
        if entry.inserted_at.elapsed() > self.ttl {
            self.entries.remove(key);
            return None;
        }
        Some(entry.scores.clone())
    }

    fn insert(&mut self, key: String, value: HashMap<String, f64>) {
        if self.entries.contains_key(&key) {
            self.entries.insert(
                key,
                CachedPprEntry {
                    scores: value,
                    inserted_at: Instant::now(),
                },
            );
            return;
        }
        self.order.push_back(key.clone());
        self.entries.insert(
            key,
            CachedPprEntry {
                scores: value,
                inserted_at: Instant::now(),
            },
        );
        while self.entries.len() > self.capacity {
            let Some(evicted) = self.order.pop_front() else {
                break;
            };
            self.entries.remove(&evicted);
        }
    }
}

static SCOPED_PPR_CACHE: OnceLock<Mutex<ScopedPprCache>> = OnceLock::new();

fn cache() -> &'static Mutex<ScopedPprCache> {
    SCOPED_PPR_CACHE.get_or_init(|| Mutex::new(ScopedPprCache::default()))
}

/// Run PPR through the process-local scoped cache.
///
/// The key is `scope + graph_version + PPR params + seed set`. The adjacency is
/// intentionally not hashed; callers must pass the store snapshot version that
/// changes whenever the structural graph changes.
pub fn cached_personalized_pagerank(
    scope: &str,
    graph_version: u64,
    adjacency: &HashMap<String, Vec<(String, f64)>>,
    seeds: &HashMap<String, f64>,
    alpha: f64,
    epsilon: f64,
    max_pushes: usize,
) -> HashMap<String, f64> {
    if seeds.is_empty() {
        return HashMap::new();
    }
    let key = scoped_ppr_cache_key(scope, graph_version, seeds, alpha, epsilon, max_pushes);
    if let Ok(mut guard) = cache().lock() {
        if let Some(cached) = guard.get(&key) {
            return cached;
        }
    }

    let scores = personalized_pagerank(adjacency, seeds, alpha, epsilon, max_pushes);
    if let Ok(mut guard) = cache().lock() {
        guard.insert(key, scores.clone());
    }
    scores
}

/// Cache a unit single-seed PPR vector, then scale it for the requested weight.
///
/// This is the stable-seed decomposition used by task-type and project-anchor
/// priors. Dynamic query seeds can be computed fresh and added to this vector.
pub fn cached_single_seed_personalized_pagerank(
    scope: &str,
    graph_version: u64,
    adjacency: &HashMap<String, Vec<(String, f64)>>,
    seed: &str,
    weight: f64,
    alpha: f64,
    epsilon: f64,
    max_pushes: usize,
) -> HashMap<String, f64> {
    if seed.trim().is_empty() || weight <= 0.0 {
        return HashMap::new();
    }
    let mut seeds = HashMap::new();
    seeds.insert(seed.to_string(), 1.0);
    let mut scores = cached_personalized_pagerank(
        scope,
        graph_version,
        adjacency,
        &seeds,
        alpha,
        epsilon,
        max_pushes,
    );
    if (weight - 1.0).abs() > f64::EPSILON {
        for score in scores.values_mut() {
            *score *= weight;
        }
    }
    scores
}

/// Add `source` scores into `target`.
pub fn merge_ppr_scores(target: &mut HashMap<String, f64>, source: HashMap<String, f64>) {
    for (node, score) in source {
        *target.entry(node).or_insert(0.0) += score;
    }
}

pub fn clear_scoped_ppr_cache() {
    if let Ok(mut guard) = cache().lock() {
        guard.entries.clear();
        guard.order.clear();
    }
}

pub fn scoped_ppr_cache_len() -> usize {
    cache()
        .lock()
        .map(|guard| guard.entries.len())
        .unwrap_or_default()
}

fn scoped_ppr_cache_key(
    scope: &str,
    graph_version: u64,
    seeds: &HashMap<String, f64>,
    alpha: f64,
    epsilon: f64,
    max_pushes: usize,
) -> String {
    let mut normalized = seeds
        .iter()
        .map(|(node, weight)| (node.as_str(), normalized_f64_bits(*weight)))
        .collect::<Vec<_>>();
    normalized.sort_by(|left, right| left.0.cmp(right.0));
    stable_hash((
        "scoped-ppr-cache-v1",
        scope,
        graph_version,
        normalized,
        normalized_f64_bits(alpha),
        normalized_f64_bits(epsilon),
        max_pushes,
    ))
}

fn normalized_f64_bits(value: f64) -> u64 {
    if !value.is_finite() || value == 0.0 {
        0.0_f64.to_bits()
    } else {
        value.to_bits()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    static TEST_CACHE_GUARD: OnceLock<Mutex<()>> = OnceLock::new();

    fn cache_test_guard() -> std::sync::MutexGuard<'static, ()> {
        TEST_CACHE_GUARD
            .get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap()
    }

    fn chain() -> HashMap<String, Vec<(String, f64)>> {
        HashMap::from([
            ("a".to_string(), vec![("b".to_string(), 1.0)]),
            ("b".to_string(), vec![("c".to_string(), 1.0)]),
        ])
    }

    #[test]
    fn scoped_cache_reuses_identical_structural_query() {
        let _guard = cache_test_guard();
        clear_scoped_ppr_cache();
        let adjacency = chain();
        let seeds = HashMap::from([("a".to_string(), 1.0)]);

        let first = cached_personalized_pagerank("scope", 7, &adjacency, &seeds, 0.15, 1e-5, 100);
        let second = cached_personalized_pagerank("scope", 7, &adjacency, &seeds, 0.15, 1e-5, 100);

        assert_eq!(first, second);
        assert_eq!(scoped_ppr_cache_len(), 1);

        let _ = cached_personalized_pagerank("scope", 8, &adjacency, &seeds, 0.15, 1e-5, 100);
        assert_eq!(scoped_ppr_cache_len(), 2);
    }

    #[test]
    fn single_seed_cache_scales_weight_after_lookup() {
        let _guard = cache_test_guard();
        clear_scoped_ppr_cache();
        let adjacency = chain();

        let unit = cached_single_seed_personalized_pagerank(
            "single", 1, &adjacency, "a", 1.0, 0.15, 1e-5, 100,
        );
        let doubled = cached_single_seed_personalized_pagerank(
            "single", 1, &adjacency, "a", 2.0, 0.15, 1e-5, 100,
        );

        assert_eq!(scoped_ppr_cache_len(), 1);
        assert!((doubled["a"] - unit["a"] * 2.0).abs() < 1e-9);
    }
}
