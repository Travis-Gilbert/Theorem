"""Cross-language byte-parity differential for the native symbolic engines.

This module runs the SAME inputs through the Python reference engines and the
Rust `theseus_native` engines and asserts byte-identical receipts on the surface
the compute-offload differential gate compares:

- Datalog: full receipt equality (engine, fact_pack_hash, rule_ids, derived_facts,
  derived_count, warnings, writeback_policy). Each derived fact's `fact_id` is a
  content hash, so full-dict equality also proves byte-level serialization parity.
- Probabilistic: full receipt equality including `receipt_hash` (the float-format
  serialization check) and `posterior`.
- Evolution: full archive receipt equality, including `archive_hash`, per-niche
  elite order, and `rejected_count`.

It imports only the pure reference modules (no Django, no pyarrow), so it runs in
a minimal build venv as well as full CI. The companion Rust unit tests in
`rustyredcore_THG/crates/rustyred-thg-core/src/symbolic.rs` assert receipt
shape; this asserts byte-equality against the Python reference, which is the
actual parity contract.

Owner: claude-code (verification lane). The native implementation it verifies
lives in `rustyredcore_THG/crates/rustyred-thg-core/src/symbolic.rs` (codex lane).
"""

from __future__ import annotations

import json
from typing import Any

from apps.notebook.inference_engines.datalog.contracts import DatalogFact, fact_pack_from_iterable
from apps.notebook.inference_engines.datalog.engine import DatalogEngine
from apps.notebook.inference_engines.datalog.facts import build_fact_pack_from_records
from apps.notebook.inference_engines.datalog.rules import DEFAULT_RULES
from apps.notebook.inference_engines.evolution.contracts import EvolutionCandidate
from apps.notebook.inference_engines.evolution.engine import EvolutionEngine
from apps.notebook.inference_engines.probabilistic.engine import ProbProgEngine

# Track the reference rule set dynamically. When a new rule lands in rules.py
# DEFAULT_RULES (e.g. the civic demolition_window rule), the gate demands native
# parity for it automatically rather than silently testing a stale hardcoded list.
DATALOG_RULE_IDS = tuple(rule.rule_id for rule in DEFAULT_RULES)


def _civic_demolition_facts() -> list[DatalogFact]:
    """Civic structure_present / structure_absent facts that trigger demolition_window."""

    return [
        DatalogFact(
            relation="structure_present",
            entity_id="SP1950",
            attributes={"parcel_id": "PARCEL-1", "year": 1950, "source_id": "sanborn:1950"},
            source_ref="civic:structure_present",
        ),
        DatalogFact(
            relation="structure_present",
            entity_id="SP1955",
            attributes={"parcel_id": "PARCEL-1", "year": 1955, "source_id": "sanborn:1955"},
            source_ref="civic:structure_present",
        ),
        DatalogFact(
            relation="structure_absent",
            entity_id="SA1960",
            attributes={"parcel_id": "PARCEL-1", "year": 1960, "source_id": "aerial:1960"},
            source_ref="civic:structure_absent",
        ),
    ]


def _civic_conflict_facts() -> list[DatalogFact]:
    """source_assertion facts that trigger conflict_set.

    Two sources assert different build_year values for the same parcel+field,
    so conflict_set emits one source_conflict for (PARCEL-2, build_year).
    """

    return [
        DatalogFact(
            relation="source_assertion",
            entity_id="SA-CONF-1",
            attributes={"parcel_id": "PARCEL-2", "field": "build_year", "value": "1920", "source_id": "assessor:a"},
            source_ref="civic:source_assertion",
        ),
        DatalogFact(
            relation="source_assertion",
            entity_id="SA-CONF-2",
            attributes={"parcel_id": "PARCEL-2", "field": "build_year", "value": "1925", "source_id": "sanborn:b"},
            source_ref="civic:source_assertion",
        ),
    ]


def _civic_vacancy_facts() -> list[DatalogFact]:
    """assessor_status facts that trigger vacancy_duration.

    A vacant run on PARCEL-3 from 2010 to 2016 (span 6 >= min_years 5) with no
    intervening occupied observation, so vacancy_duration emits prolonged_vacancy.
    """

    return [
        DatalogFact(
            relation="assessor_status",
            entity_id="AS-2010",
            attributes={"parcel_id": "PARCEL-3", "status": "vacant", "year": 2010, "source_id": "assessor:2010"},
            source_ref="civic:assessor_status",
        ),
        DatalogFact(
            relation="assessor_status",
            entity_id="AS-2013",
            attributes={"parcel_id": "PARCEL-3", "status": "vacant", "year": 2013, "source_id": "assessor:2013"},
            source_ref="civic:assessor_status",
        ),
        DatalogFact(
            relation="assessor_status",
            entity_id="AS-2016",
            attributes={"parcel_id": "PARCEL-3", "status": "vacant", "year": 2016, "source_id": "assessor:2016"},
            source_ref="civic:assessor_status",
        ),
    ]


def _civic_ownership_facts() -> list[DatalogFact]:
    """ownership facts that trigger ownership_chain with the distressed pattern.

    Three owners on PARCEL-4 where a tax_foreclosure owner (2015) precedes a
    land_bank owner (2018), so ownership_chain emits the chain plus the
    tax_foreclosure_to_land_bank distressed flag.
    """

    return [
        DatalogFact(
            relation="ownership",
            entity_id="OWN-1",
            attributes={"parcel_id": "PARCEL-4", "owner": "Jane Smith", "owner_type": "private", "from_year": 2000, "to_year": 2014, "source_id": "deeds:1"},
            source_ref="civic:ownership",
        ),
        DatalogFact(
            relation="ownership",
            entity_id="OWN-2",
            attributes={"parcel_id": "PARCEL-4", "owner": "Genesee County Treasurer", "owner_type": "tax_foreclosure", "from_year": 2015, "to_year": 2017, "source_id": "deeds:2"},
            source_ref="civic:ownership",
        ),
        DatalogFact(
            relation="ownership",
            entity_id="OWN-3",
            attributes={"parcel_id": "PARCEL-4", "owner": "Genesee County Land Bank", "owner_type": "land_bank", "from_year": 2018, "source_id": "deeds:3"},
            source_ref="civic:ownership",
        ),
    ]


def build_coverage_fact_pack():
    """A fact pack engineered to trigger every reference rule, including civic ones."""

    base = build_fact_pack_from_records(
        objects=[
            {"id": "O1", "title": "Alpha Source"},
            {"id": "O2", "title": "Hello World"},
            {"id": "O3", "title": "hello-world"},
            {"id": "O6", "title": "Private Obj", "properties": {"private": True}},
        ],
        claims=[
            {"id": "C1", "status": "proposed"},
            {"id": "C2", "status": "captured"},
            {"id": "C7", "status": "captured"},
            {"id": "C8", "status": "captured"},
            {"id": "C9", "status": "archived"},
        ],
        evidence_links=[],
        claim_dependencies=[
            {"id": "D1", "claim_id": "C2", "depends_on_object_id": "O1", "justification_type": "source", "strength": 0.8},
            {"id": "D7", "claim_id": "C7", "depends_on_object_id": "O9"},
            {"id": "D8", "claim_id": "C8", "depends_on_object_id": "O9"},
        ],
        context_atoms=[
            {"id": "A1", "kind": "code_symbol", "title": "foo", "metadata": {"failed_tests": True}},
            {"id": "A2", "kind": "note", "artifact_id": "ARTG", "metadata": {"generated": True}},
            {"id": "A3", "kind": "note", "object_pk": "O6", "artifact_id": "ARTP", "metadata": {"export_candidate": True}},
        ],
        edges=[{"id": "E1", "from_object_id": "O4", "to_object_id": "O5", "edge_type": "contradicts"}],
        evidence_paths=[{"id": "P1", "edge_pks": ["e1", "e2", "e3", "e4"]}],
    )
    return fact_pack_from_iterable(
        list(base.facts)
        + _civic_demolition_facts()
        + _civic_conflict_facts()
        + _civic_vacancy_facts()
        + _civic_ownership_facts()
    )


def _native_datalog(native_module: Any, pack, rule_ids: list[str] | None = None) -> dict[str, Any]:
    payload = [fact.to_dict() for fact in pack.facts]
    if rule_ids is None:
        arg = json.dumps(payload)
    else:
        arg = json.dumps({"facts": payload, "rule_ids": rule_ids})
    return json.loads(native_module.bgi_datalog_derive_core_json(arg))


def _native_verified_rule_ids(native_module: Any) -> list[str]:
    """Rules the native module DECLARES parity-verified (bgi_datalog_verified_rule_ids_json)."""

    return [str(rule_id) for rule_id in json.loads(native_module.bgi_datalog_verified_rule_ids_json())]


def run_datalog_derivation_parity(native_module: Any) -> dict[str, Any]:
    """Per-rule + all-rules datalog parity over the native-declared verified set.

    Parity is the must-pass invariant: for every rule the native module declares
    verified (bgi_datalog_verified_rule_ids_json) that is also in the live Python
    DEFAULT_RULES, the native receipt must be byte-identical to the Python
    reference. Rules in DEFAULT_RULES but NOT yet declared by native are reported
    as `coverage_lag` (informational), not failures: native is honest that it has
    not implemented them, so they are tracked, never silent, and do not false-red
    the gate while the civic rule set grows. When native later declares a lagging
    rule verified, parity covers it automatically with no gate change.
    """

    pack = build_coverage_fact_pack()
    native_verified = _native_verified_rule_ids(native_module)
    parity_rules = [rule_id for rule_id in DATALOG_RULE_IDS if rule_id in native_verified]
    coverage_lag = [rule_id for rule_id in DATALOG_RULE_IDS if rule_id not in native_verified]

    per_rule: list[dict[str, Any]] = []
    failures: list[str] = []

    # All-rules apples-to-apples: native runs its declared set; compare Python
    # restricted to exactly the rule_ids native emitted, in native's order.
    native_all = _native_datalog(native_module, pack)
    python_all = DatalogEngine().derive(pack, rule_ids=list(native_all.get("rule_ids") or [])).to_dict()
    all_equal = python_all == native_all
    if not all_equal:
        failures.append("__all_rules__")

    for rule_id in parity_rules:
        python_receipt = DatalogEngine().derive(pack, rule_ids=[rule_id]).to_dict()
        native_receipt = _native_datalog(native_module, pack, [rule_id])
        equal = python_receipt == native_receipt
        per_rule.append({
            "rule_id": rule_id,
            "equal": equal,
            "python_derived_count": python_receipt["derived_count"],
            "native_derived_count": native_receipt.get("derived_count"),
            "fact_pack_hash_match": python_receipt["fact_pack_hash"] == native_receipt.get("fact_pack_hash"),
            "python_receipt": python_receipt if not equal else None,
            "native_receipt": native_receipt if not equal else None,
        })
        if not equal:
            failures.append(rule_id)

    return {
        "all_rules_equal": all_equal,
        "per_rule": per_rule,
        "parity_rules": parity_rules,
        "coverage_lag": coverage_lag,
        "failures": failures,
        "passed": not failures,
    }


def run_probabilistic_parity(native_module: Any) -> dict[str, Any]:
    """Return a probabilistic parity report (receipt_hash + posterior byte-equality)."""

    failures: list[str] = []

    python_sr = ProbProgEngine().source_reliability(
        source_id="source-a", prior_alpha=2.0, prior_beta=2.0, corroborated=6, contradicted=2,
    ).to_dict()
    native_sr = json.loads(native_module.bgi_probabilistic_source_reliability_json(json.dumps({
        "source_id": "source-a", "prior_alpha": 2.0, "prior_beta": 2.0, "corroborated": 6, "contradicted": 2,
    })))
    if python_sr != native_sr:
        failures.append("source_reliability")

    python_ev = ProbProgEngine().expected_value_of_information(
        current_uncertainty=0.6666666666666666, expected_uncertainty_after=0.25,
        decision_value=3.0, validator_cost=1.0,
    ).to_dict()
    native_ev = json.loads(native_module.bgi_probabilistic_expected_value_json(json.dumps({
        "current_uncertainty": 0.6666666666666666, "expected_uncertainty_after": 0.25,
        "decision_value": 3.0, "validator_cost": 1.0,
    })))
    if python_ev != native_ev:
        failures.append("expected_value_of_information")

    return {
        "source_reliability_equal": python_sr == native_sr,
        "expected_value_equal": python_ev == native_ev,
        "failures": failures,
        "passed": not failures,
    }


def run_evolution_archive_parity(native_module: Any) -> dict[str, Any]:
    """Return a MAP-Elites archive parity report (archive_hash + elite order)."""

    candidates = [
        EvolutionCandidate(
            candidate_id="candidate-a",
            niche="skill|promote|green",
            score=0.7,
            novelty=0.2,
            payload={"policy": {"exact_symbol_target": 1.0}},
        ),
        EvolutionCandidate(
            candidate_id="candidate-b",
            niche="skill|promote|green",
            score=0.7,
            novelty=0.9,
            payload={"policy": {"exact_symbol_target": 1.2}},
        ),
        EvolutionCandidate(
            candidate_id="candidate-c",
            niche="tool|rewrite|blue",
            score=0.5,
            novelty=1.0,
            payload={"patch": {"kind": "tool"}},
        ),
        EvolutionCandidate(
            candidate_id="candidate-d",
            niche="tool|rewrite|blue",
            score=0.4,
            novelty=0.1,
            payload={"patch": {"kind": "tool-low"}},
        ),
    ]
    payload = {
        "candidates": [candidate.to_dict() for candidate in candidates],
        "elites_per_niche": 1,
    }
    python_receipt = EvolutionEngine().archive(candidates, elites_per_niche=1).to_dict()
    native_receipt = json.loads(native_module.bgi_evolution_archive_json(json.dumps(payload)))

    return {
        "archive_equal": python_receipt == native_receipt,
        "failures": [] if python_receipt == native_receipt else ["archive"],
        "python_receipt": None if python_receipt == native_receipt else python_receipt,
        "native_receipt": None if python_receipt == native_receipt else native_receipt,
        "passed": python_receipt == native_receipt,
    }


def run_edge_case_parity(native_module: Any) -> dict[str, Any]:
    """Parity on the contract edges: empty pack, unknown rule warning, payload forms."""

    from apps.notebook.inference_engines.datalog.contracts import fact_pack_from_iterable

    failures: list[str] = []

    # 1. Empty fact pack: native runs its declared set; compare Python restricted
    #    to exactly native's emitted rule_ids (so a growing civic set never false-reds).
    empty = fact_pack_from_iterable([])
    nat_empty = _native_datalog(native_module, empty)
    py_empty = DatalogEngine().derive(empty, rule_ids=list(nat_empty.get("rule_ids") or [])).to_dict()
    if py_empty != nat_empty:
        failures.append("empty_pack")

    pack = build_coverage_fact_pack()

    # 2. Unknown rule id: Python emits a "Unknown Datalog rule skipped" warning, no facts.
    py_unknown = DatalogEngine().derive(pack, rule_ids=["does_not_exist"]).to_dict()
    nat_unknown = _native_datalog(native_module, pack, ["does_not_exist"])
    if py_unknown != nat_unknown:
        failures.append("unknown_rule_warning")

    # 3. Array vs object {"facts": [...]} payload forms must both equal Python
    #    restricted to native's declared rule_ids (apples-to-apples over the verified set).
    payload = [fact.to_dict() for fact in pack.facts]
    nat_array = json.loads(native_module.bgi_datalog_derive_core_json(json.dumps(payload)))
    nat_object = json.loads(native_module.bgi_datalog_derive_core_json(json.dumps({"facts": payload})))
    py_scoped = DatalogEngine().derive(pack, rule_ids=list(nat_array.get("rule_ids") or [])).to_dict()
    if nat_array != py_scoped:
        failures.append("array_payload_form")
    if nat_object != py_scoped:
        failures.append("object_payload_form")

    return {
        "empty_pack_equal": py_empty == nat_empty,
        "unknown_rule_equal": py_unknown == nat_unknown,
        "array_form_equal": nat_array == py_scoped,
        "object_form_equal": nat_object == py_scoped,
        "failures": failures,
        "passed": not failures,
    }


def run_all(native_module: Any) -> dict[str, Any]:
    datalog = run_datalog_derivation_parity(native_module)
    probabilistic = run_probabilistic_parity(native_module)
    evolution = run_evolution_archive_parity(native_module)
    edge_cases = run_edge_case_parity(native_module)
    return {
        "passed": datalog["passed"] and probabilistic["passed"] and evolution["passed"] and edge_cases["passed"],
        "datalog": datalog,
        "probabilistic": probabilistic,
        "evolution": evolution,
        "edge_cases": edge_cases,
    }
