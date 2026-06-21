//! The eleven-stage epistemic filter — the fusion-quality layer of the
//! multi-source search pipeline (`docs/plans/multi-source-search/HANDOFF.md`).
//!
//! This is the pure-Rust port of the post-fusion ranking/filter core of Theseus
//! `apps/notebook/search/retrieval.py::_fuse_score_and_threshold` (the
//! "eleven-stage epistemic filter" the handoff calls for). The handoff's lock is
//! "fusion is entirely Rust ... no Python hop in the search hot path", so the
//! stages that are pure ranking, thresholding, dedup, and ordering are ported
//! here byte-faithfully; the stages that are genuinely DB/ML/embedding work are
//! left as injected seams (see below), not inlined.
//!
//! ## What this module owns (the pure stages)
//!
//! The Python function is an imperative pipeline; its in-code labels are
//! `Stage 2/3/3.5/3.7/4` plus "Stage 11 of the Memgraph canonical arc" (the code
//! gate). There is no literal eleven-item list. The discrete pure operations, in
//! execution order, are:
//!
//!   3a. Candidate truncation to `2 * top_n` (before any rerank — a candidate
//!       below the cut can never be rescued by a later bonus).
//!   3b. Code-object exclusion gate (drop `source_system == "codebase"` or a
//!       `code_entity_type` in the 7-element code set, unless `include_code` or a
//!       `code_debug` query). This is the code's "Stage 11 of the Memgraph arc".
//!    5. Learned scoring via the injected `ConnectionScorer`; the default
//!       `RrfFallbackScorer` reproduces the Python `except: learned = rrf,
//!       method = "rrf_fallback"` path so the pure pipeline runs without the model.
//!    6. Epistemic-weight boosting: `learned *= epistemic_weight`, plus the
//!       `+0.25` code_debug bump; records acceptance_status + justification_prior.
//!    7. Slug deduplication: same slug keeps the higher learned score (strict `>`,
//!       index semantics preserved).
//!    8. Title-query overlap bonus: `min(0.3, overlap * 0.10)`.
//!    9. Sort by learned score desc (stable), drop `< min_score`, and drop
//!       acquaintance (`epistemic_weight <= 0.0`) when `exclude_acquaintance`.
//!   10. Acceptance-status filtering: always drop `retracted`; drop `provisional`
//!       / `contested` unless explicitly included.
//!   11. Final `top_n` slice.
//!
//! ## Seams (NOT ported here — injected by the caller)
//!
//! Per the reference's seam list, these leave pure logic and stay out of this
//! module: the RRF merge (Stage 2, owned by the provider fan-out in `search.rs`),
//! the learned ML scorer (Stage 5, the `ConnectionScorer` trait), world-scope
//! classification, object hydration from the graph store (Stage 4 — the caller
//! supplies `FusedCandidate`s already carrying the epistemic fields), the
//! `code_debug` DB re-admission (Stage 3c), and the temporal scope filter
//! (Stage 11, a DB seam — applied by the caller before the final slice when set).
//!
//! ## Byte-parity hazards handled here
//!
//! - Rounding is round-half-to-even (Python 3 `round`), applied at exactly the
//!   Python interleave points: `rrf_score` to 6 dp once, `learned_score` to 4 dp
//!   after the scorer, after `* epistemic_weight`, after the `+0.25` bump, and
//!   after the title bonus. `round_half_even` uses `f64::round_ties_even`.
//! - Ordering of equal-score ties relies on stable sort over an insertion-ordered
//!   `Vec`; nothing here iterates a `HashMap` to produce ranked output.
//! - The acquaintance filter (Stage 9) and the explicit `retracted` drop
//!   (Stage 10) intentionally double-cover retracted objects; both are kept.

use std::collections::{HashMap, HashSet};

use rustyred_thg_core::{apply_cascade, EpistemicGate, QueryContext, RankCandidate, RankingRule};
use serde::{Deserialize, Serialize};

/// Final result top-n (Python `top_n` default).
pub const DEFAULT_TOP_N: usize = 50;
/// Minimum learned score to survive Stage 9 (Python `min_score` default 0.1).
pub const DEFAULT_MIN_SCORE: f64 = 0.1;
/// Code_debug score bump added in Stage 6 (Python `+ 0.25`).
const CODE_DEBUG_BUMP: f64 = 0.25;
/// Per-overlapping-word title bonus (Python `overlap * 0.10`).
const TITLE_BONUS_PER_WORD: f64 = 0.10;
/// Cap on the title-overlap bonus (Python `min(0.3, ...)`).
const TITLE_BONUS_CAP: f64 = 0.3;
/// Signal key the code_debug re-admission tags candidates with (Python
/// `signals['code_boost'] = 0.3`); read in Stage 6 to apply the bump.
pub const SIGNAL_CODE_BOOST: &str = "code_boost";

/// The 7-element code-entity-type set that the code-exclusion gate drops. This is
/// the *second* (winning) definition in the Python source (line 357); the earlier
/// 4-element definition is shadowed before the gate ever runs. Porting the
/// 4-element set is the easy parity bug the reference flags.
const CODE_ENTITY_TYPES: [&str; 7] = [
    "code_file",
    "code_structure",
    "code_member",
    "code_process",
    "specification",
    "fix_pattern",
    "commit",
];

/// The graph `source_system` value marking a code-ingested object.
const CODE_SOURCE_SYSTEM: &str = "codebase";

/// Round half-to-even (banker's rounding) to `dp` decimal places, matching
/// Python 3's built-in `round`. Rust's `f64::round` is half-away-from-zero, which
/// would diverge on the `.5` boundary; `round_ties_even` is the correct primitive.
pub fn round_half_even(value: f64, dp: u32) -> f64 {
    let factor = 10f64.powi(dp as i32);
    (value * factor).round_ties_even() / factor
}

/// A fused candidate entering the epistemic filter: the output of the RRF merge
/// (Stage 2, owned elsewhere) after object hydration (Stage 4, a seam). Every
/// field here is one the pure stages read; the caller fills them from the graph
/// `Object` it hydrated. Defaults mirror the Python `getattr(obj, ..., default)`
/// fallbacks so a partially-populated candidate behaves like the Python object.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FusedCandidate {
    /// The object primary key / slug id, as a string so int pks and code slugs
    /// share one type (the Python `candidate_pks` mixes both).
    pub object_pk: String,
    /// The fused RRF score from Stage 2.
    pub rrf_score: f64,
    /// `Object.epistemic_weight` (Python default 1.0 when the attribute is
    /// missing): acquaintance 0.0, explanatory 1.2x, axiomatic 1.5x, etc.
    pub epistemic_weight: f64,
    /// `Object.acceptance_status` (Python `getattr` default "accepted").
    pub acceptance_status: String,
    /// `Object.justification_confidence_prior` (Python default 0.5).
    pub justification_prior: f64,
    /// `Object.slug` for Stage 7 dedup; `None` candidates are never deduped.
    pub slug: Option<String>,
    /// `Object.title` for the Stage 8 overlap bonus.
    pub title: String,
    /// `Object.source_system` for the Stage 3b code gate.
    pub source_system: Option<String>,
    /// `Object.properties.code_entity_type` for the Stage 3b code gate.
    pub code_entity_type: Option<String>,
    /// Bi-temporal valid-time start (ms), the temporal seam the `Recency` rule reads. `None` is
    /// unknown valid-time.
    #[serde(default)]
    pub valid_from_ms: Option<i64>,
    /// Whether a newer version supersedes this one (the supersession signal the temporal seam
    /// reads); `Recency` sends superseded results to the worst band.
    #[serde(default)]
    pub superseded: bool,
    /// Per-pk signal scores (keyed by score-key, e.g. `code_boost`). The pure
    /// stages read only `code_boost`; the rest ride through for the scorer seam.
    #[serde(default)]
    pub signals: HashMap<String, f64>,
}

impl FusedCandidate {
    /// A minimal candidate with the Python defaults; tests and callers set the
    /// epistemic fields they have and leave the rest at their faithful defaults.
    pub fn new(object_pk: impl Into<String>, rrf_score: f64) -> Self {
        Self {
            object_pk: object_pk.into(),
            rrf_score,
            epistemic_weight: 1.0,
            acceptance_status: "accepted".to_string(),
            justification_prior: 0.5,
            slug: None,
            title: String::new(),
            source_system: None,
            code_entity_type: None,
            valid_from_ms: None,
            superseded: false,
            signals: HashMap::new(),
        }
    }

    fn is_code_object(&self) -> bool {
        if self.source_system.as_deref() == Some(CODE_SOURCE_SYSTEM) {
            return true;
        }
        match self.code_entity_type.as_deref() {
            Some(kind) => CODE_ENTITY_TYPES.contains(&kind),
            None => false,
        }
    }
}

/// A scored result after the pipeline. Mirrors the Python result dict's
/// ranking-relevant keys (the keys downstream consumers and tests assert on).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScoredResult {
    pub object_pk: String,
    /// `rrf_score` rounded to 6 dp (Stage 5).
    pub rrf_score: f64,
    /// `learned_score` after the scorer and every boost, rounded to 4 dp.
    pub learned_score: f64,
    pub scoring_method: String,
    pub epistemic_weight: f64,
    pub acceptance_status: String,
    pub justification_prior: f64,
    /// The Stage 8 bonus when a title overlap fired; `None` otherwise (Python
    /// only sets the key when `overlap >= 1`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title_overlap_bonus: Option<f64>,
    pub signals: HashMap<String, f64>,
}

/// The learned-scorer seam (Python Stage 5 `score_connection`). The real
/// implementation calls the gradient-boosted / ML ranker over a feature vector;
/// the pure pipeline runs against `RrfFallbackScorer`, which reproduces the
/// Python exception path (`learned = rrf_score`, `method = "rrf_fallback"`).
pub trait ConnectionScorer {
    /// Score one candidate against the query; returns `(learned_score, method)`.
    fn score(&self, candidate: &FusedCandidate, query: &str) -> (f64, String);
}

/// The default scorer: the Python `except:` fallback. Always available, pure, and
/// deterministic, so the epistemic filter is fully testable without the ML model.
#[derive(Clone, Copy, Debug, Default)]
pub struct RrfFallbackScorer;

impl ConnectionScorer for RrfFallbackScorer {
    fn score(&self, candidate: &FusedCandidate, _query: &str) -> (f64, String) {
        (candidate.rrf_score, "rrf_fallback".to_string())
    }
}

/// Knobs for the epistemic filter, defaulting to the Python `unified_retrieve`
/// signature defaults.
#[derive(Clone, Debug, PartialEq)]
pub struct EpistemicFilterConfig {
    /// Final result count and the `2 * top_n` truncation base (Python 50).
    pub top_n: usize,
    /// Reject `learned_score < min_score` in Stage 9 (Python `min_score`, 0.1).
    pub min_score: f64,
    /// Keep `provisional` results (Python `include_provisional`, default false).
    pub include_provisional: bool,
    /// Keep `contested` results (Python `include_contested`, default false).
    pub include_contested: bool,
    /// Drop acquaintance (`epistemic_weight <= 0.0`) in Stage 9 (default true).
    pub exclude_acquaintance: bool,
    /// Keep code objects through Stage 3b (Python `include_code`, default false).
    pub include_code: bool,
    /// The query type; only the literal `"code_debug"` changes behavior (it skips
    /// the code gate and enables the `code_boost` bump).
    pub query_type: Option<String>,
}

impl Default for EpistemicFilterConfig {
    fn default() -> Self {
        Self {
            top_n: DEFAULT_TOP_N,
            min_score: DEFAULT_MIN_SCORE,
            include_provisional: false,
            include_contested: false,
            exclude_acquaintance: true,
            include_code: false,
            query_type: None,
        }
    }
}

impl EpistemicFilterConfig {
    fn is_code_debug(&self) -> bool {
        self.query_type.as_deref() == Some("code_debug")
    }
}

/// Apply the eleven-stage epistemic filter to a fused, hydrated candidate list.
///
/// `fused` must already be in RRF rank order (Stage 2 output). The function runs
/// Stages 3a, 3b, 5-11 from the reference and returns the surviving results,
/// ranked and truncated to `top_n`. The temporal seam (Stage 11's optional
/// `as_of`/`between` filter) is applied by the caller before the final slice when
/// set; with it unset, the slice is the whole of Stage 11.
pub fn apply_epistemic_filter(
    fused: Vec<FusedCandidate>,
    query: &str,
    config: &EpistemicFilterConfig,
    scorer: &dyn ConnectionScorer,
) -> Vec<ScoredResult> {
    run_pipeline(fused, query, config, scorer)
}

/// The web path's default cascade rule order (SPEC-RUSTYRED-RANKING-CASCADE, D4): trust ahead of
/// relevance, the differentiator. Epistemic standing dominates, then materialized source
/// reliability, then recency, then the learned relevance score. The order is data: callers pass any
/// order to [`apply_epistemic_cascade`].
//
// ponytail: default order is the high-leverage knob held for Travis; reorder here (or per query)
// to make relevance dominate and trust break ties instead.
pub fn web_cascade_rules() -> Vec<RankingRule> {
    vec![
        RankingRule::EpistemicStatus,
        RankingRule::SourceReliability,
        RankingRule::Recency,
        RankingRule::Text,
    ]
}

/// The web path's epistemic signals expressed as a ranking-rule cascade (D4): the same fused,
/// hydrated candidates ordered through [`apply_cascade`] instead of one blended `learned_score`.
///
/// `EpistemicStatus` subsumes Stage 10 (retracted excluded, contested/provisional gated by config)
/// and the acquaintance drop (`epistemic_weight <= 0`); `Recency` is the temporal seam (superseded
/// to the worst band); `SourceReliability` reads the materialized per-source value
/// (`justification_prior` for now); the learned `ConnectionScorer` stays a relevance input that
/// bucketizes into `Text` rather than a separate multiply (the title-overlap bonus folds into it).
/// Structural stages survive: 3a truncation, 3b code gate, slug dedup, and the final `top_n` slice.
pub fn apply_epistemic_cascade(
    fused: Vec<FusedCandidate>,
    query: &str,
    config: &EpistemicFilterConfig,
    scorer: &dyn ConnectionScorer,
    rules: &[RankingRule],
) -> Vec<ScoredResult> {
    // Stage 3a: truncate to 2 * top_n before scoring.
    let candidates: Vec<FusedCandidate> = fused
        .into_iter()
        .take(config.top_n.saturating_mul(2))
        .collect();
    if candidates.is_empty() {
        return Vec::new();
    }
    // Stage 3b: code-object exclusion gate.
    let code_debug = config.is_code_debug();
    let candidates: Vec<FusedCandidate> = if !config.include_code && !code_debug {
        candidates
            .into_iter()
            .filter(|candidate| !candidate.is_code_object())
            .collect()
    } else {
        candidates
    };

    // The cascade query context: terms from the query; the admission gate mirrors the Stage 10
    // include flags. The acquaintance drop is the `epistemic_weight <= 0` exclusion inside the rule.
    let context = QueryContext {
        epistemic: EpistemicGate {
            allow_provisional: config.include_provisional,
            allow_contested: config.include_contested,
        },
        ..QueryContext::from_query(query)
    };
    let query_words: HashSet<String> = query
        .to_lowercase()
        .split_whitespace()
        .map(str::to_string)
        .collect();

    // Score each candidate (learned relevance + title nudge -> the Text band) and build both the
    // result record and the rank candidate carrying the epistemic/temporal/reliability signals.
    let mut results: Vec<ScoredResult> = Vec::with_capacity(candidates.len());
    let mut rank_candidates: Vec<RankCandidate> = Vec::with_capacity(candidates.len());
    let mut slug_by_pk: HashMap<String, String> = HashMap::new();
    for candidate in candidates {
        let (learned, method) = scorer.score(&candidate, query);
        let mut learned = round_half_even(learned, 4);
        let title_bonus = title_overlap_bonus(&query_words, &candidate.title);
        if let Some(bonus) = title_bonus {
            learned = round_half_even(learned + bonus, 4);
        }
        if let Some(slug) = candidate.slug.as_deref().filter(|slug| !slug.is_empty()) {
            slug_by_pk.insert(candidate.object_pk.clone(), slug.to_string());
        }

        let mut rank = RankCandidate::new(candidate.object_pk.clone());
        rank.bm25_score = Some(learned as f32);
        rank.source_reliability = Some(candidate.justification_prior as f32);
        rank.valid_from_ms = candidate.valid_from_ms;
        rank.superseded = candidate.superseded;
        rank.epistemic_weight = candidate.epistemic_weight as f32;
        rank.acceptance_status = candidate.acceptance_status.clone();
        rank_candidates.push(rank);

        results.push(ScoredResult {
            object_pk: candidate.object_pk,
            rrf_score: round_half_even(candidate.rrf_score, 6),
            learned_score: learned,
            scoring_method: method,
            epistemic_weight: candidate.epistemic_weight,
            acceptance_status: candidate.acceptance_status,
            justification_prior: candidate.justification_prior,
            title_overlap_bonus: title_bonus,
            signals: candidate.signals,
        });
    }

    // The cascade orders and excludes (retracted / acquaintance / gated status).
    let outcome = apply_cascade(rank_candidates, rules, &context);
    let mut by_pk: HashMap<String, ScoredResult> =
        results.into_iter().map(|r| (r.object_pk.clone(), r)).collect();
    let mut ordered: Vec<ScoredResult> = outcome
        .ranked
        .into_iter()
        .filter_map(|ranked| by_pk.remove(&ranked.row_id))
        .collect();

    // Stage 7: slug dedup -- in cascade order the first occurrence of a slug is the best-ranked.
    let mut seen_slugs: HashSet<String> = HashSet::new();
    ordered.retain(|result| match slug_by_pk.get(&result.object_pk) {
        Some(slug) => seen_slugs.insert(slug.clone()),
        None => true,
    });

    // Stage 11: final top_n slice.
    ordered.truncate(config.top_n);
    ordered
}

/// Stage 6: epistemic-weight boosting for one result, given its source weight and
/// whether a code_boost signal is present under a code_debug query.
fn boost_epistemic(result: &mut ScoredResult, is_code_debug: bool) {
    // learned *= epistemic_weight (default 1.0 already on the result).
    result.learned_score = round_half_even(result.learned_score * result.epistemic_weight, 4);
    // +0.25 for code_debug results carrying a code_boost signal.
    if is_code_debug && result.signals.contains_key(SIGNAL_CODE_BOOST) {
        result.learned_score = round_half_even(result.learned_score + CODE_DEBUG_BUMP, 4);
    }
}

/// Stage 8: title-query overlap bonus for one result given the query word set and
/// the candidate's title.
fn apply_title_overlap_bonus(
    result: &mut ScoredResult,
    query_words: &HashSet<String>,
    title: &str,
) {
    if let Some(bonus) = title_overlap_bonus(query_words, title) {
        result.learned_score = round_half_even(result.learned_score + bonus, 4);
        result.title_overlap_bonus = Some(bonus);
    }
}

/// The Stage 8 title-overlap bonus as a pure value (capped), or `None` when no word overlaps.
/// Shared by the blended pipeline and the cascade path, which folds it into the relevance input.
fn title_overlap_bonus(query_words: &HashSet<String>, title: &str) -> Option<f64> {
    let title_words: HashSet<String> = title
        .to_lowercase()
        .split_whitespace()
        .map(str::to_string)
        .collect();
    let overlap = query_words.intersection(&title_words).count();
    (overlap >= 1).then(|| (overlap as f64 * TITLE_BONUS_PER_WORD).min(TITLE_BONUS_CAP))
}

/// The real implementation, carrying the slug/title metadata the public result
/// struct does not expose. The public `apply_epistemic_filter` is a thin shim.
fn run_pipeline(
    fused: Vec<FusedCandidate>,
    query: &str,
    config: &EpistemicFilterConfig,
    scorer: &dyn ConnectionScorer,
) -> Vec<ScoredResult> {
    // ---- Stage 3a: truncate to 2 * top_n before any rerank.
    let cutoff = config.top_n.saturating_mul(2);
    let candidates: Vec<FusedCandidate> = fused.into_iter().take(cutoff).collect();
    if candidates.is_empty() {
        return Vec::new();
    }

    // ---- Stage 3b: code-object exclusion gate.
    let code_debug = config.is_code_debug();
    let candidates: Vec<FusedCandidate> = if !config.include_code && !code_debug {
        candidates
            .into_iter()
            .filter(|candidate| !candidate.is_code_object())
            .collect()
    } else {
        candidates
    };

    // Parallel slug/title views preserve faithful order across the moves below.
    let slugs: Vec<Option<String>> = candidates.iter().map(|c| c.slug.clone()).collect();
    let titles: Vec<String> = candidates.iter().map(|c| c.title.clone()).collect();

    // ---- Stage 5: learned scoring (scorer seam). rrf to 6 dp, learned to 4 dp.
    let mut results: Vec<ScoredResult> = candidates
        .into_iter()
        .map(|candidate| {
            let (learned, method) = scorer.score(&candidate, query);
            ScoredResult {
                object_pk: candidate.object_pk,
                rrf_score: round_half_even(candidate.rrf_score, 6),
                learned_score: round_half_even(learned, 4),
                scoring_method: method,
                epistemic_weight: candidate.epistemic_weight,
                acceptance_status: candidate.acceptance_status,
                justification_prior: candidate.justification_prior,
                title_overlap_bonus: None,
                signals: candidate.signals,
            }
        })
        .collect();

    // ---- Stage 6: epistemic-weight boosting (+ code_debug bump).
    for result in &mut results {
        boost_epistemic(result, code_debug);
    }

    // ---- Stage 7: slug dedup. Same slug keeps the higher learned score; ties
    // and the first-seen index follow the Python `>`/index semantics exactly.
    let mut seen_slugs: HashMap<String, usize> = HashMap::new();
    let mut deduped: HashSet<usize> = HashSet::new();
    for (i, result) in results.iter().enumerate() {
        let Some(slug) = slugs[i].as_deref() else {
            continue;
        };
        if slug.is_empty() {
            continue;
        }
        if let Some(&prev_idx) = seen_slugs.get(slug) {
            if result.learned_score > results[prev_idx].learned_score {
                deduped.insert(prev_idx);
                seen_slugs.insert(slug.to_string(), i);
            } else {
                deduped.insert(i);
            }
        } else {
            seen_slugs.insert(slug.to_string(), i);
        }
    }
    let (mut results, titles): (Vec<ScoredResult>, Vec<String>) = if deduped.is_empty() {
        (results, titles)
    } else {
        results
            .into_iter()
            .zip(titles)
            .enumerate()
            .filter(|(i, _)| !deduped.contains(i))
            .map(|(_, pair)| pair)
            .unzip()
    };

    // ---- Stage 8: title-query overlap bonus.
    let query_words: HashSet<String> = query
        .to_lowercase()
        .split_whitespace()
        .map(str::to_string)
        .collect();
    for (result, title) in results.iter_mut().zip(titles.iter()) {
        apply_title_overlap_bonus(result, &query_words, title);
    }

    // ---- Stage 9: sort by learned score desc (stable), min-score, acquaintance.
    results.sort_by(|a, b| {
        b.learned_score
            .partial_cmp(&a.learned_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    results.retain(|result| result.learned_score >= config.min_score);
    if config.exclude_acquaintance {
        results.retain(|result| result.epistemic_weight > 0.0);
    }

    // ---- Stage 10: acceptance-status filtering. Retracted always drops (it is
    // also covered by the acquaintance filter; both are kept on purpose).
    results.retain(|result| result.acceptance_status != "retracted");
    if !config.include_provisional {
        results.retain(|result| result.acceptance_status != "provisional");
    }
    if !config.include_contested {
        results.retain(|result| result.acceptance_status != "contested");
    }

    // ---- Stage 11: final top_n slice. (The temporal seam, when set, is applied
    // by the caller on this list before the slice.)
    results.truncate(config.top_n);
    results
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(pk: &str, rrf: f64) -> FusedCandidate {
        FusedCandidate::new(pk, rrf)
    }

    #[test]
    fn round_half_even_matches_python_bankers_rounding() {
        // Half-to-even at 0 dp on values exactly representable in f64.
        assert_eq!(round_half_even(2.5, 0), 2.0);
        assert_eq!(round_half_even(3.5, 0), 4.0);
        assert_eq!(round_half_even(0.5, 0), 0.0);
        assert_eq!(round_half_even(1.5, 0), 2.0);
        // Exact binary eighths at 2 dp: 0.125 -> 0.12, 0.375 -> 0.38 (ties to the
        // even neighbour), matching Python round() on the identical doubles.
        assert_eq!(round_half_even(0.125, 2), 0.12);
        assert_eq!(round_half_even(0.375, 2), 0.38);
        // The 6 dp truncation point the pipeline uses. Note: 0.12345 at 4 dp is
        // NOT a clean tie -- the nearest f64 is just below 0.12345, so both Python
        // round() and this round it to 0.1234 (float representation, not the mode).
        assert_eq!(round_half_even(0.123456789, 6), 0.123457);
        assert_eq!(round_half_even(1.0 / 61.0, 6), 0.016393);
    }

    #[test]
    fn empty_input_returns_empty() {
        let out = apply_epistemic_filter_for_test(vec![], "q", &EpistemicFilterConfig::default());
        assert!(out.is_empty());
    }

    #[test]
    fn rrf_fallback_scorer_carries_rrf_into_learned_score() {
        let fused = vec![candidate("a", 0.5), candidate("b", 0.25)];
        let out =
            apply_epistemic_filter_for_test(fused, "anything", &EpistemicFilterConfig::default());
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].object_pk, "a");
        assert_eq!(out[0].scoring_method, "rrf_fallback");
        // epistemic_weight defaults to 1.0, so learned == rrf after Stage 6.
        assert_eq!(out[0].learned_score, 0.5);
        assert_eq!(out[1].learned_score, 0.25);
    }

    #[test]
    fn stage_3a_truncates_to_two_top_n_before_rerank() {
        // top_n = 1 => keep only the top 2 fused candidates; the 3rd is gone even
        // though it could otherwise earn a title bonus.
        let config = EpistemicFilterConfig {
            top_n: 1,
            ..EpistemicFilterConfig::default()
        };
        let mut third = candidate("third", 0.05);
        third.title = "title query".to_string(); // would-be title overlap
        let fused = vec![candidate("first", 0.9), candidate("second", 0.8), third];
        let out = apply_epistemic_filter_for_test(fused, "query", &config);
        // Stage 11 slices to top_n = 1, but truncation already dropped "third".
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].object_pk, "first");
    }

    #[test]
    fn stage_3b_drops_code_objects_unless_included() {
        let mut code = candidate("code", 0.9);
        code.source_system = Some("codebase".to_string());
        let mut spec = candidate("spec", 0.8);
        spec.code_entity_type = Some("specification".to_string()); // 7-element set
        let prose = candidate("prose", 0.7);

        let default = EpistemicFilterConfig::default();
        let out = apply_epistemic_filter_for_test(
            vec![code.clone(), spec.clone(), prose.clone()],
            "q",
            &default,
        );
        let pks: Vec<&str> = out.iter().map(|r| r.object_pk.as_str()).collect();
        assert_eq!(pks, vec!["prose"], "code objects are gated out by default");

        let with_code = EpistemicFilterConfig {
            include_code: true,
            ..EpistemicFilterConfig::default()
        };
        let out = apply_epistemic_filter_for_test(vec![code, spec, prose], "q", &with_code);
        assert_eq!(out.len(), 3, "include_code keeps them");
    }

    #[test]
    fn stage_6_multiplies_by_epistemic_weight() {
        let mut axiomatic = candidate("ax", 0.4);
        axiomatic.epistemic_weight = 1.5; // axiomatic boost
        let mut hypothetical = candidate("hy", 0.4);
        hypothetical.epistemic_weight = 0.5; // halved
        let out = apply_epistemic_filter_for_test(
            vec![axiomatic, hypothetical],
            "q",
            &EpistemicFilterConfig::default(),
        );
        // 0.4 * 1.5 = 0.6 ; 0.4 * 0.5 = 0.2. Sorted desc.
        assert_eq!(out[0].object_pk, "ax");
        assert_eq!(out[0].learned_score, 0.6);
        assert_eq!(out[1].learned_score, 0.2);
    }

    #[test]
    fn stage_6_code_debug_bump_only_with_signal_and_query_type() {
        let mut boosted = candidate("boost", 0.4);
        boosted.signals.insert(SIGNAL_CODE_BOOST.to_string(), 0.3);
        let plain = candidate("plain", 0.4);
        let config = EpistemicFilterConfig {
            query_type: Some("code_debug".to_string()),
            ..EpistemicFilterConfig::default()
        };
        let out = apply_epistemic_filter_for_test(vec![boosted, plain], "q", &config);
        // boost: 0.4 * 1.0 = 0.4, + 0.25 = 0.65; plain stays 0.4.
        let boost = out.iter().find(|r| r.object_pk == "boost").unwrap();
        let plain = out.iter().find(|r| r.object_pk == "plain").unwrap();
        assert_eq!(boost.learned_score, 0.65);
        assert_eq!(plain.learned_score, 0.4);
    }

    #[test]
    fn stage_7_slug_dedup_keeps_higher_learned_score() {
        let mut high = candidate("high", 0.9);
        high.slug = Some("shared".to_string());
        let mut low = candidate("low", 0.3);
        low.slug = Some("shared".to_string());
        let other = {
            let mut c = candidate("other", 0.5);
            c.slug = Some("unique".to_string());
            c
        };
        // Order: low first, high second -> the earlier (low) is the one removed.
        let out = apply_epistemic_filter_for_test(
            vec![low, high, other],
            "q",
            &EpistemicFilterConfig::default(),
        );
        let pks: Vec<&str> = out.iter().map(|r| r.object_pk.as_str()).collect();
        assert!(
            pks.contains(&"high"),
            "the higher-scored duplicate survives"
        );
        assert!(
            !pks.contains(&"low"),
            "the lower-scored duplicate is dropped"
        );
        assert!(pks.contains(&"other"));
    }

    #[test]
    fn stage_8_title_overlap_bonus_is_capped() {
        let mut c = candidate("t", 0.2);
        // 4 overlapping words * 0.10 = 0.40, capped to 0.30.
        c.title = "alpha beta gamma delta".to_string();
        let out = apply_epistemic_filter_for_test(
            vec![c],
            "alpha beta gamma delta",
            &EpistemicFilterConfig::default(),
        );
        assert_eq!(out[0].title_overlap_bonus, Some(0.3));
        // 0.2 + 0.3 (capped) = 0.5.
        assert_eq!(out[0].learned_score, 0.5);
    }

    #[test]
    fn stage_9_min_score_threshold_drops_weak_results() {
        let strong = candidate("strong", 0.5);
        let weak = candidate("weak", 0.05); // below default min_score 0.1
        let out = apply_epistemic_filter_for_test(
            vec![strong, weak],
            "q",
            &EpistemicFilterConfig::default(),
        );
        let pks: Vec<&str> = out.iter().map(|r| r.object_pk.as_str()).collect();
        assert_eq!(pks, vec!["strong"]);
    }

    #[test]
    fn stage_9_acquaintance_filter_drops_zero_weight() {
        let mut acquaintance = candidate("acq", 0.9);
        acquaintance.epistemic_weight = 0.0; // acquaintance
        let normal = candidate("normal", 0.5);
        let out = apply_epistemic_filter_for_test(
            vec![acquaintance, normal],
            "q",
            &EpistemicFilterConfig::default(),
        );
        // Acquaintance is dropped even though its rrf was highest (0.9 * 0.0 = 0.0,
        // below min_score and also caught by the acquaintance filter).
        let pks: Vec<&str> = out.iter().map(|r| r.object_pk.as_str()).collect();
        assert_eq!(pks, vec!["normal"]);
    }

    #[test]
    fn stage_10_acceptance_status_filtering() {
        let accepted = candidate("ok", 0.5);
        let mut retracted = candidate("retracted", 0.9);
        retracted.acceptance_status = "retracted".to_string();
        let mut provisional = candidate("prov", 0.8);
        provisional.acceptance_status = "provisional".to_string();
        let mut contested = candidate("cont", 0.7);
        contested.acceptance_status = "contested".to_string();

        // Default: only accepted survives.
        let out = apply_epistemic_filter_for_test(
            vec![
                accepted.clone(),
                retracted.clone(),
                provisional.clone(),
                contested.clone(),
            ],
            "q",
            &EpistemicFilterConfig::default(),
        );
        let pks: Vec<&str> = out.iter().map(|r| r.object_pk.as_str()).collect();
        assert_eq!(pks, vec!["ok"]);

        // Opt-in keeps provisional + contested, but retracted is always dropped.
        let permissive = EpistemicFilterConfig {
            include_provisional: true,
            include_contested: true,
            ..EpistemicFilterConfig::default()
        };
        let out = apply_epistemic_filter_for_test(
            vec![accepted, retracted, provisional, contested],
            "q",
            &permissive,
        );
        let mut pks: Vec<&str> = out.iter().map(|r| r.object_pk.as_str()).collect();
        pks.sort();
        assert_eq!(pks, vec!["cont", "ok", "prov"]);
    }

    #[test]
    fn stage_11_slices_to_top_n() {
        let config = EpistemicFilterConfig {
            top_n: 2,
            ..EpistemicFilterConfig::default()
        };
        // 4 candidates, all survive scoring; top_n=2 keeps the 2 highest. The
        // 2*top_n truncation (=4) admits all four, so the slice is the binding cut.
        let fused = vec![
            candidate("a", 0.9),
            candidate("b", 0.7),
            candidate("c", 0.5),
            candidate("d", 0.3),
        ];
        let out = apply_epistemic_filter_for_test(fused, "q", &config);
        let pks: Vec<&str> = out.iter().map(|r| r.object_pk.as_str()).collect();
        assert_eq!(pks, vec!["a", "b"]);
    }

    #[test]
    fn full_pipeline_is_deterministic() {
        let build = || {
            vec![
                {
                    let mut c = candidate("p1", 0.6);
                    c.title = "shared topic".to_string();
                    c.slug = Some("p1".to_string());
                    c
                },
                {
                    let mut c = candidate("p2", 0.5);
                    c.epistemic_weight = 1.2;
                    c.slug = Some("p2".to_string());
                    c
                },
            ]
        };
        let first = apply_epistemic_filter_for_test(
            build(),
            "shared topic",
            &EpistemicFilterConfig::default(),
        );
        let second = apply_epistemic_filter_for_test(
            build(),
            "shared topic",
            &EpistemicFilterConfig::default(),
        );
        assert_eq!(first, second);
    }

    /// Test shim: the public entry takes a `&dyn ConnectionScorer`; tests use the
    /// default fallback scorer.
    fn apply_epistemic_filter_for_test(
        fused: Vec<FusedCandidate>,
        query: &str,
        config: &EpistemicFilterConfig,
    ) -> Vec<ScoredResult> {
        apply_epistemic_filter(fused, query, config, &RrfFallbackScorer)
    }

    fn cascade(fused: Vec<FusedCandidate>, query: &str, config: &EpistemicFilterConfig, rules: &[RankingRule]) -> Vec<String> {
        apply_epistemic_cascade(fused, query, config, &RrfFallbackScorer, rules)
            .into_iter()
            .map(|r| r.object_pk)
            .collect()
    }

    // D4 acceptance: the cascade ordering supersedes the old blended `learned_score` order. A
    // better-warranted but weaker-relevance result ranks first under the cascade, where it ranked
    // second under the blended `learned *= epistemic_weight`.
    #[test]
    fn cascade_ordering_supersedes_blended_learned_score() {
        let mut trusted = candidate("trusted", 0.4);
        trusted.epistemic_weight = 1.5; // axiomatic warrant, weak relevance
        let weak_trust = candidate("weak_trust", 0.9); // strong relevance, ordinary warrant (1.0)

        // Blended: relevance * weight, the strong-relevance doc wins (0.9 > 0.4 * 1.5 = 0.6).
        let blended: Vec<String> = apply_epistemic_filter_for_test(
            vec![trusted.clone(), weak_trust.clone()],
            "q",
            &EpistemicFilterConfig::default(),
        )
        .into_iter()
        .map(|r| r.object_pk)
        .collect();
        assert_eq!(blended, vec!["weak_trust", "trusted"]);

        // Cascade, trust ahead of relevance: the better-warranted doc ranks first.
        assert_eq!(
            cascade(
                vec![trusted, weak_trust],
                "q",
                &EpistemicFilterConfig::default(),
                &web_cascade_rules(),
            ),
            vec!["trusted", "weak_trust"]
        );
    }

    // D4 acceptance: retracted always drops and provisional/contested stay gated, now through the
    // EpistemicStatus rule rather than Stage 10.
    #[test]
    fn cascade_epistemic_status_drops_retracted_and_gates_provisional_contested() {
        let accepted = candidate("ok", 0.5);
        let mut retracted = candidate("retracted", 0.9);
        retracted.acceptance_status = "retracted".to_string();
        let mut provisional = candidate("prov", 0.8);
        provisional.acceptance_status = "provisional".to_string();
        let mut contested = candidate("cont", 0.7);
        contested.acceptance_status = "contested".to_string();
        let all = || vec![accepted.clone(), retracted.clone(), provisional.clone(), contested.clone()];

        // Default: only accepted survives.
        assert_eq!(
            cascade(all(), "q", &EpistemicFilterConfig::default(), &web_cascade_rules()),
            vec!["ok"]
        );

        // Opt-in keeps provisional + contested; retracted is always dropped.
        let permissive = EpistemicFilterConfig {
            include_provisional: true,
            include_contested: true,
            ..EpistemicFilterConfig::default()
        };
        let mut pks = cascade(all(), "q", &permissive, &web_cascade_rules());
        pks.sort();
        assert_eq!(pks, vec!["cont", "ok", "prov"]);
    }

    // D4 acceptance: of two candidates, the one carrying a superseding newer valid-time sinks the
    // superseded one to last under the Recency rule, even with stronger relevance.
    #[test]
    fn cascade_recency_sinks_superseded_below_current() {
        let mut newer = candidate("newer", 0.3); // weaker relevance, current
        newer.valid_from_ms = Some(2_000);
        let mut superseded = candidate("superseded", 0.9); // stronger relevance, but superseded
        superseded.valid_from_ms = Some(1_000);
        superseded.superseded = true;

        assert_eq!(
            cascade(
                vec![superseded, newer],
                "q",
                &EpistemicFilterConfig::default(),
                &[RankingRule::Recency, RankingRule::Text],
            ),
            vec!["newer", "superseded"]
        );
    }
}
