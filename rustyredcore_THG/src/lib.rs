// The capital-THG suffix in `rustyredcore_THG` is intentional protocol
// marker preservation. See PRINCIPLES.md P-001 and the module-level
// rationale below.
#![allow(non_snake_case)]
#![allow(unexpected_cfgs)]

//! rustyredcore_THG: PyO3 entry point.
//!
//! RustyRedCore-THG is the Theseus-customized fork: RustyRed Core
//! substrate with THG protocol native support. Distinct from the
//! standalone RustyRed-GraphDB OSS repo — this is the merger upgrade
//! of the original Theseus Hot Graph atop the RustyRed substrate.
//!
//! Exports PyO3 accelerators including `push_ppr`, matching the live
//! Python signature in `apps/notebook/sparse_ppr.py:push_ppr` exactly:
//!
//!     push_ppr(
//!         adjacency: dict[int, list[tuple[int, float]]],
//!         seeds: dict[int, float],
//!         *,
//!         alpha: float = 0.15,
//!         epsilon: float = 1e-4,
//!         max_pushes: int = 200_000,
//!     ) -> dict[int, float]
//!
//! `alpha`, `epsilon`, `max_pushes` are keyword-only (PyO3 `*` separator).
//! `adjacency` keys and node IDs are Python `int` (not contiguous indices)
//! because Theseus PKs are arbitrary integers.

mod adapters;
mod bgi;
mod cmh;
mod graph_export;
mod push_ppr;
mod search_kernel;
mod thg;

use pyo3::prelude::*;

// The capital-THG suffix is intentional — it marks the protocol family
// (Theseus Hot Graph). The lowercase `thg` convention would erase that
// signal and reintroduce the merge-vs-original ambiguity this rename
// exists to remove. See PRINCIPLES.md P-002 if/when added.
//
// PyO3 module name override: the Python-visible module is `theseus_native`
// (matches `[project].name` + `[tool.maturin].module-name` in pyproject.toml).
// Without `#[pyo3(name = ...)]` the generated symbol would be
// `PyInit_rustyredcore_THG` (derived from the Rust fn name) and Python's
// import would fail because the .so file is named `theseus_native.abi3.so`
// — Python looks for `PyInit_theseus_native`. The name override makes the
// symbol match the file name so dlopen + dlsym succeed.
//
// This defect was silent in production until the PT-004 native-dispatch
// probe (apps/notebook/apps.py) started logging FALLBACK warnings at boot.
// See docs/plans/mcp-server-consolidation/phase-0-execution-report.md.
#[allow(non_snake_case)]
#[pymodule]
#[pyo3(name = "theseus_native")]
fn rustyredcore_THG(_py: Python<'_>, m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(push_ppr::push_ppr, m)?)?;
    m.add_function(wrap_pyfunction!(push_ppr::push_ppr_filtered, m)?)?;
    m.add_function(wrap_pyfunction!(cmh::cmh_body_hash, m)?)?;
    m.add_function(wrap_pyfunction!(cmh::cmh_atom_id_v1, m)?)?;
    m.add_function(wrap_pyfunction!(cmh::cmh_handoff_state_hash_v1, m)?)?;
    m.add_function(wrap_pyfunction!(bgi::bgi_stable_hash_json, m)?)?;
    m.add_function(wrap_pyfunction!(bgi::bgi_fact_pack_hash_rows_json, m)?)?;
    m.add_function(wrap_pyfunction!(bgi::bgi_egraph_receipt_summary_json, m)?)?;
    m.add_function(wrap_pyfunction!(
        bgi::bgi_egraph_extract_context_pack_json,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(bgi::bgi_datalog_receipt_summary_json, m)?)?;
    m.add_function(wrap_pyfunction!(
        bgi::bgi_datalog_verified_rule_ids_json,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(bgi::bgi_datalog_derive_core_json, m)?)?;
    m.add_function(wrap_pyfunction!(
        bgi::bgi_probabilistic_source_reliability_json,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        bgi::bgi_probabilistic_expected_value_json,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(bgi::bgi_evolution_archive_json, m)?)?;
    m.add_function(wrap_pyfunction!(bgi::bgi_compact_receipts_json, m)?)?;
    m.add_function(wrap_pyfunction!(
        search_kernel::search_normalize_urls_batch,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        search_kernel::search_score_frontier_batch,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        search_kernel::search_fuse_scores_batch,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(search_kernel::search_cosine_topk, m)?)?;
    m.add_function(wrap_pyfunction!(graph_export::graph_remap_ids_batch, m)?)?;
    m.add_function(wrap_pyfunction!(graph_export::graph_pack_edges_batch, m)?)?;
    m.add_function(wrap_pyfunction!(thg::rustyred_thg_expand_bounded, m)?)?;
    m.add_function(wrap_pyfunction!(thg::rustyred_thg_paths_shortest, m)?)?;
    m.add_class::<thg::RustyredThgCoreExecutor>()?;
    adapters::register(_py, m)?;
    Ok(())
}
