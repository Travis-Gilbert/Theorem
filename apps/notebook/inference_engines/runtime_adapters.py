"""Adapters that feed BGI engines from real runtime-shaped records."""

from __future__ import annotations

from statistics import mean
from typing import Any, Iterable

from .causal.engine import CausalEngine
from .optimizer.engine import OptimizerEngine
from .probabilistic.engine import ProbProgEngine


POSITIVE_EVIDENCE_STATUSES = {'accepted', 'corroborated', 'supported', 'supports', 'passed'}
NEGATIVE_EVIDENCE_STATUSES = {'contradicted', 'rejected', 'refuted', 'failed'}


def _status(value: Any) -> str:
    return str(value or '').strip().lower()


def source_reliability_from_records(
    *,
    source_id: str,
    evidence_records: Iterable[dict[str, Any]],
    prior_alpha: float = 1.0,
    prior_beta: float = 1.0,
    engine: ProbProgEngine | None = None,
) -> dict[str, Any]:
    """Estimate source reliability from evidence/claim status rows."""

    records = [dict(record) for record in evidence_records]
    corroborated = sum(1 for record in records if _status(record.get('status')) in POSITIVE_EVIDENCE_STATUSES)
    contradicted = sum(1 for record in records if _status(record.get('status')) in NEGATIVE_EVIDENCE_STATUSES)
    receipt = (engine or ProbProgEngine()).source_reliability(
        source_id=source_id,
        prior_alpha=prior_alpha,
        prior_beta=prior_beta,
        corroborated=corroborated,
        contradicted=contradicted,
    )
    payload = receipt.to_dict()
    payload['metadata']['record_count'] = len(records)
    payload['metadata']['input_shape'] = 'evidence_records'
    return payload


def expected_value_from_validator_records(
    *,
    validator_records: Iterable[dict[str, Any]],
    decision_value: float = 1.0,
    engine: ProbProgEngine | None = None,
) -> dict[str, Any]:
    """Compute EVI from validator history shaped like runtime receipts."""

    records = [dict(record) for record in validator_records]
    if not records:
        current_uncertainty = 1.0
        expected_uncertainty_after = 0.8
        cost = 0.0
    else:
        pass_rate = sum(1 for record in records if _status(record.get('status')) == 'passed') / len(records)
        current_uncertainty = 1.0 - abs(pass_rate - 0.5) * 2.0
        expected_uncertainty_after = max(0.0, current_uncertainty * 0.6)
        cost = mean(float(record.get('cost', record.get('duration_ms', 1.0)) or 1.0) for record in records)
    receipt = (engine or ProbProgEngine()).expected_value_of_information(
        current_uncertainty=current_uncertainty,
        expected_uncertainty_after=expected_uncertainty_after,
        decision_value=decision_value,
        validator_cost=cost,
    )
    payload = receipt.to_dict()
    payload['metadata']['record_count'] = len(records)
    payload['metadata']['input_shape'] = 'validator_records'
    return payload


def causal_effect_from_observation_groups(
    *,
    question_id: str,
    treatment: str,
    outcome: str,
    treated_records: Iterable[dict[str, Any]],
    control_records: Iterable[dict[str, Any]],
    assumptions: Iterable[str] = (),
    confounders: Iterable[str] = (),
) -> dict[str, Any]:
    """Estimate a causal effect from treated/control outcome summaries."""

    treated = [float(record.get('outcome_value', record.get(outcome, 0.0)) or 0.0) for record in treated_records]
    control = [float(record.get('outcome_value', record.get(outcome, 0.0)) or 0.0) for record in control_records]
    receipt = CausalEngine().intervention_effect(
        question_id=question_id,
        treatment=treatment,
        outcome=outcome,
        treated_mean=mean(treated) if treated else None,
        control_mean=mean(control) if control else None,
        assumptions=tuple(str(item) for item in assumptions),
        confounders=tuple(str(item) for item in confounders),
    )
    payload = receipt.to_dict()
    payload['metadata']['treated_count'] = len(treated)
    payload['metadata']['control_count'] = len(control)
    payload['metadata']['input_shape'] = 'treated_control_records'
    return payload


def validator_schedule_from_records(
    *,
    validator_records: Iterable[dict[str, Any]],
    budget: float,
) -> dict[str, Any]:
    """Select validators from runtime candidate records."""

    result = OptimizerEngine().schedule_validators(
        [dict(record) for record in validator_records],
        budget=budget,
    )
    payload = result.to_dict()
    payload['input_shape'] = 'validator_records'
    return payload
