//! Differential parity gate for the eleven-stage epistemic filter.
//!
//! This is the Rust half of the joint Python-parity gate (the Python half is
//! specified in `docs/plans/multi-source-search/epistemic-parity-gate.md` and
//! runs the REAL Theseus `_fuse_score_and_threshold` over the same fixtures).
//!
//! It runs `apply_epistemic_filter` (with the `RrfFallbackScorer`, which matches
//! the Python `rrf_fallback` path) over the shared fixture set and:
//!   1. asserts the Rust output matches a committed Rust golden (a regression
//!      gate; regenerate with `UPDATE_GOLDEN=1 cargo test -p rustyred-web --test
//!      epistemic_parity`);
//!   2. when `tests/epistemic_parity_python_golden.json` is present (Codex's
//!      half), asserts the Rust output equals the Python golden value-for-value
//!      (the cross-language parity gate). Comparison is over parsed structs, not
//!      raw JSON text, so float formatting differences between the two json
//!      encoders never produce a false mismatch -- only a genuine value
//!      divergence (e.g. a banker's-rounding discrepancy) fails the gate.

use std::collections::HashMap;
use std::path::Path;

use rustyred_web::{
    apply_epistemic_filter, EpistemicFilterConfig, FusedCandidate, RrfFallbackScorer,
};
use serde::{Deserialize, Serialize};

fn default_top_n() -> usize {
    50
}
fn default_min_score() -> f64 {
    0.1
}
fn default_true() -> bool {
    true
}
fn default_epistemic_weight() -> f64 {
    1.0
}
fn default_accepted() -> String {
    "accepted".to_string()
}
fn default_prior() -> f64 {
    0.5
}

/// The fixture config shape. Defaults mirror `EpistemicFilterConfig::default`, so
/// a case only sets the knobs it exercises. The Python harness applies the same
/// defaults (documented in the fixtures `_comment`).
#[derive(Debug, Deserialize)]
struct ConfigJson {
    #[serde(default = "default_top_n")]
    top_n: usize,
    #[serde(default = "default_min_score")]
    min_score: f64,
    #[serde(default)]
    include_provisional: bool,
    #[serde(default)]
    include_contested: bool,
    #[serde(default = "default_true")]
    exclude_acquaintance: bool,
    #[serde(default)]
    include_code: bool,
    #[serde(default)]
    query_type: Option<String>,
}

impl From<ConfigJson> for EpistemicFilterConfig {
    fn from(cj: ConfigJson) -> Self {
        EpistemicFilterConfig {
            top_n: cj.top_n,
            min_score: cj.min_score,
            include_provisional: cj.include_provisional,
            include_contested: cj.include_contested,
            exclude_acquaintance: cj.exclude_acquaintance,
            include_code: cj.include_code,
            query_type: cj.query_type,
        }
    }
}

/// The fixture candidate shape. Defaults mirror `FusedCandidate::new`.
#[derive(Debug, Deserialize)]
struct FixtureCandidate {
    object_pk: String,
    rrf_score: f64,
    #[serde(default = "default_epistemic_weight")]
    epistemic_weight: f64,
    #[serde(default = "default_accepted")]
    acceptance_status: String,
    #[serde(default = "default_prior")]
    justification_prior: f64,
    #[serde(default)]
    slug: Option<String>,
    #[serde(default)]
    title: String,
    #[serde(default)]
    source_system: Option<String>,
    #[serde(default)]
    code_entity_type: Option<String>,
    #[serde(default)]
    signals: HashMap<String, f64>,
}

impl From<FixtureCandidate> for FusedCandidate {
    fn from(fc: FixtureCandidate) -> Self {
        FusedCandidate {
            object_pk: fc.object_pk,
            rrf_score: fc.rrf_score,
            epistemic_weight: fc.epistemic_weight,
            acceptance_status: fc.acceptance_status,
            justification_prior: fc.justification_prior,
            slug: fc.slug,
            title: fc.title,
            source_system: fc.source_system,
            code_entity_type: fc.code_entity_type,
            signals: fc.signals,
        }
    }
}

#[derive(Debug, Deserialize)]
struct Case {
    name: String,
    query: String,
    config: ConfigJson,
    candidates: Vec<FixtureCandidate>,
}

#[derive(Debug, Deserialize)]
struct Fixtures {
    cases: Vec<Case>,
}

/// The parity-relevant output of one result. This is the exact shape the Python
/// golden must emit (same field names, same order).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct ResultRow {
    object_pk: String,
    rrf_score: f64,
    learned_score: f64,
    scoring_method: String,
    epistemic_weight: f64,
    acceptance_status: String,
    title_overlap_bonus: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct CaseOutput {
    name: String,
    results: Vec<ResultRow>,
}

fn fixtures_path() -> String {
    format!(
        "{}/tests/epistemic_parity_fixtures.json",
        env!("CARGO_MANIFEST_DIR")
    )
}

fn rust_golden_path() -> String {
    format!(
        "{}/tests/epistemic_parity_rust_golden.json",
        env!("CARGO_MANIFEST_DIR")
    )
}

fn python_golden_path() -> String {
    format!(
        "{}/tests/epistemic_parity_python_golden.json",
        env!("CARGO_MANIFEST_DIR")
    )
}

/// Run every fixture case through the filter and collect the parity rows.
fn compute_outputs() -> Vec<CaseOutput> {
    let raw = std::fs::read_to_string(fixtures_path()).expect("read fixtures");
    let fixtures: Fixtures = serde_json::from_str(&raw).expect("parse fixtures");
    let scorer = RrfFallbackScorer;
    fixtures
        .cases
        .into_iter()
        .map(|case| {
            let config: EpistemicFilterConfig = case.config.into();
            let fused: Vec<FusedCandidate> = case
                .candidates
                .into_iter()
                .map(FusedCandidate::from)
                .collect();
            let results = apply_epistemic_filter(fused, &case.query, &config, &scorer)
                .into_iter()
                .map(|sr| ResultRow {
                    object_pk: sr.object_pk,
                    rrf_score: sr.rrf_score,
                    learned_score: sr.learned_score,
                    scoring_method: sr.scoring_method,
                    epistemic_weight: sr.epistemic_weight,
                    acceptance_status: sr.acceptance_status,
                    title_overlap_bonus: sr.title_overlap_bonus,
                })
                .collect();
            CaseOutput {
                name: case.name,
                results,
            }
        })
        .collect()
}

#[test]
fn rust_output_matches_committed_golden() {
    let outputs = compute_outputs();
    let golden_path = rust_golden_path();
    let regenerate = std::env::var("UPDATE_GOLDEN").is_ok();

    if regenerate || !Path::new(&golden_path).exists() {
        let json = serde_json::to_string_pretty(&outputs).expect("serialize golden");
        std::fs::write(&golden_path, format!("{json}\n")).expect("write golden");
        eprintln!("epistemic_parity: wrote Rust golden to {golden_path}");
        return;
    }

    let golden: Vec<CaseOutput> =
        serde_json::from_str(&std::fs::read_to_string(&golden_path).expect("read golden"))
            .expect("parse golden");
    assert_eq!(
        outputs, golden,
        "Rust filter output drifted from the committed golden; re-run with UPDATE_GOLDEN=1 if this change is intended"
    );
}

#[test]
fn python_golden_matches_rust_when_present() {
    let py_path = python_golden_path();
    if !Path::new(&py_path).exists() {
        eprintln!(
            "epistemic_parity: {py_path} not present yet (Codex's half of the parity gate); \
             cross-language assertion skipped until the Python golden lands"
        );
        return;
    }
    let outputs = compute_outputs();
    let python: Vec<CaseOutput> =
        serde_json::from_str(&std::fs::read_to_string(&py_path).expect("read python golden"))
            .expect("parse python golden");
    assert_eq!(
        outputs, python,
        "Rust apply_epistemic_filter diverged from the Python _fuse_score_and_threshold golden \
         (check banker's-rounding parity and stage ordering)"
    );
}
