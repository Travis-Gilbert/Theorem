use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use url::Url;

use crate::SubstrateSearch;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub enum CrawlDial {
    #[default]
    Conservative,
    Broad,
}

/// The single named dial that governs how readily similarity-search reaches the web.
///
/// The gate reaches the web when the local mean graph score falls below this value, so a
/// HIGHER threshold reaches the web more readily (it trusts the substrate only when local
/// evidence is strong). The default is tuned so search reaches out on moderate -- not only
/// near-empty -- local evidence, which is the addendum's "lower the web-call threshold so
/// similarity-search reaches the web more readily": the practical barrier to a web call is
/// lowered. Override it at config construction with [`WEB_REACH_THRESHOLD_ENV`].
///
/// This is the web-CALL threshold (when to go out), kept deliberately distinct from the
/// rerank admission gate (what to keep) governed by SPEC-SEARCH-RERANK-GATE-1.0.
pub const DEFAULT_WEB_REACH_THRESHOLD: f64 = 0.2;

/// Environment variable that overrides [`DEFAULT_WEB_REACH_THRESHOLD`] at construction via
/// [`TriggerGateConfig::with_env_overrides`]. The single configurable web-reach value.
pub const WEB_REACH_THRESHOLD_ENV: &str = "RUSTYRED_WEB_REACH_THRESHOLD";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TriggerGateConfig {
    pub dial: CrawlDial,
    pub min_hits: usize,
    pub min_distinct_domains: usize,
    /// The single web-reach dial: reach the web when the local mean graph score is below
    /// this. Higher reaches the web more readily. See [`DEFAULT_WEB_REACH_THRESHOLD`].
    pub web_reach_threshold: f64,
    pub max_score_variance: f64,
    pub max_crawl_seeds: usize,
}

impl TriggerGateConfig {
    pub fn conservative() -> Self {
        Self {
            dial: CrawlDial::Conservative,
            min_hits: 1,
            min_distinct_domains: 1,
            web_reach_threshold: DEFAULT_WEB_REACH_THRESHOLD,
            max_score_variance: 1.0,
            max_crawl_seeds: 3,
        }
    }

    pub fn broad() -> Self {
        Self {
            dial: CrawlDial::Broad,
            min_hits: 3,
            min_distinct_domains: 2,
            // The broad dial reaches the web even more readily than the default.
            web_reach_threshold: 0.35,
            max_score_variance: 0.5,
            max_crawl_seeds: 8,
        }
    }

    /// Apply the [`WEB_REACH_THRESHOLD_ENV`] override (when set to a finite, non-negative
    /// number) to the single web-reach dial. Call this when building the gate from runtime
    /// configuration so an operator can tune web reach without a code change.
    pub fn with_env_overrides(mut self) -> Self {
        if let Some(value) = std::env::var(WEB_REACH_THRESHOLD_ENV)
            .ok()
            .and_then(|raw| raw.trim().parse::<f64>().ok())
            .filter(|value| value.is_finite() && *value >= 0.0)
        {
            self.web_reach_threshold = value;
        }
        self
    }
}

impl Default for TriggerGateConfig {
    fn default() -> Self {
        Self::conservative()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TriggerGateDecision {
    pub should_crawl: bool,
    pub reason: String,
    pub hit_count: usize,
    pub distinct_domains: usize,
    pub mean_score: f64,
    pub score_variance: f64,
    pub max_crawl_seeds: usize,
}

pub fn evaluate_trigger_gate(
    search: &SubstrateSearch,
    config: &TriggerGateConfig,
) -> TriggerGateDecision {
    let stats = SearchStats::from_search(search);
    let (should_crawl, reason) = if search.query.trim().is_empty() {
        (false, "browse mode does not trigger crawl")
    } else if stats.hit_count == 0 {
        (true, "empty results")
    } else if stats.hit_count < config.min_hits {
        (true, "too few results")
    } else if stats.distinct_domains < config.min_distinct_domains {
        (true, "too little domain coverage")
    } else if stats.mean_score < config.web_reach_threshold {
        (true, "local mean score below web-reach threshold")
    } else if stats.score_variance > config.max_score_variance {
        (true, "score variance is too high")
    } else {
        (false, "local substrate evidence is sufficient")
    };

    TriggerGateDecision {
        should_crawl,
        reason: reason.to_string(),
        hit_count: stats.hit_count,
        distinct_domains: stats.distinct_domains,
        mean_score: stats.mean_score,
        score_variance: stats.score_variance,
        max_crawl_seeds: config.max_crawl_seeds,
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct SearchStats {
    hit_count: usize,
    distinct_domains: usize,
    mean_score: f64,
    score_variance: f64,
}

impl SearchStats {
    fn from_search(search: &SubstrateSearch) -> Self {
        if search.hits.is_empty() {
            return Self::default();
        }

        let mut domains = BTreeSet::new();
        let mut score_sum = 0.0;
        for hit in &search.hits {
            if let Ok(url) = Url::parse(&hit.url) {
                if let Some(host) = url.host_str() {
                    domains.insert(host.to_ascii_lowercase());
                }
            }
            score_sum += hit.match_score;
        }
        let hit_count = search.hits.len();
        let mean_score = score_sum / hit_count as f64;
        let variance = search
            .hits
            .iter()
            .map(|hit| {
                let delta = hit.match_score - mean_score;
                delta * delta
            })
            .sum::<f64>()
            / hit_count as f64;

        Self {
            hit_count,
            distinct_domains: domains.len(),
            mean_score,
            score_variance: variance,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{SearchHit, SearchLink};

    fn hit(url: &str, score: f64) -> SearchHit {
        SearchHit {
            node_id: url.to_string(),
            url: url.to_string(),
            title: url.to_string(),
            snippet: String::new(),
            ring: 0,
            ring_label: "match".to_string(),
            match_score: score,
            // empty snippet => past the frontier, by provenance_of's rule.
            provenance: "frontier".to_string(),
        }
    }

    fn search(query: &str, hits: Vec<SearchHit>) -> SubstrateSearch {
        SubstrateSearch {
            query: query.to_string(),
            hits,
            links: Vec::<SearchLink>::new(),
            matched_count: 0,
            kept_count: 0,
        }
    }

    #[test]
    fn empty_results_trigger_crawl() {
        let decision =
            evaluate_trigger_gate(&search("rust", vec![]), &TriggerGateConfig::default());
        assert!(decision.should_crawl);
        assert_eq!(decision.reason, "empty results");
    }

    #[test]
    fn conservative_gate_accepts_one_strong_result() {
        let decision = evaluate_trigger_gate(
            &search("rust", vec![hit("https://example.com/rust", 0.3)]),
            &TriggerGateConfig::conservative(),
        );
        assert!(!decision.should_crawl);
    }

    #[test]
    fn broad_gate_requires_more_coverage() {
        let decision = evaluate_trigger_gate(
            &search("rust", vec![hit("https://example.com/rust", 0.3)]),
            &TriggerGateConfig::broad(),
        );
        assert!(decision.should_crawl);
        assert_eq!(decision.reason, "too few results");
        assert_eq!(decision.max_crawl_seeds, 8);
    }

    // T8 (search-reach threshold): a single moderate-confidence local hit (score 0.1).
    // Under the prior conservative gate (mean-score floor 0.01) this stayed graph-local;
    // the single, raised web-reach dial now reaches the web because 0.1 < 0.2.
    #[test]
    fn lowered_web_reach_threshold_reaches_web_on_moderate_evidence() {
        let moderate = search("rust ownership", vec![hit("https://example.com/a", 0.1)]);
        let decision = evaluate_trigger_gate(&moderate, &TriggerGateConfig::conservative());
        assert!(
            decision.should_crawl,
            "moderate local evidence now reaches the web: {decision:?}"
        );
        assert_eq!(decision.reason, "local mean score below web-reach threshold");

        // Single configurable value: under the prior 0.01 floor the same query stays local,
        // proving `web_reach_threshold` is the one dial that governs web reach.
        let prior = TriggerGateConfig {
            web_reach_threshold: 0.01,
            ..TriggerGateConfig::conservative()
        };
        assert!(
            !evaluate_trigger_gate(&moderate, &prior).should_crawl,
            "lowering the single web-reach dial keeps the query graph-local"
        );
    }

    #[test]
    fn env_override_tunes_the_single_web_reach_dial() {
        // The override is applied at construction; assert the parse/clamp without relying on
        // a specific process-global env value (env mutation races other tests).
        let base = TriggerGateConfig::conservative();
        assert_eq!(base.web_reach_threshold, DEFAULT_WEB_REACH_THRESHOLD);
        let tuned = TriggerGateConfig {
            web_reach_threshold: 0.5,
            ..TriggerGateConfig::conservative()
        };
        // A strong-looking local result (0.3) that stays local by default now reaches the
        // web once the single dial is raised to 0.5.
        let strong = search("rust", vec![hit("https://example.com/x", 0.3)]);
        assert!(!evaluate_trigger_gate(&strong, &base).should_crawl);
        assert!(evaluate_trigger_gate(&strong, &tuned).should_crawl);
    }
}
