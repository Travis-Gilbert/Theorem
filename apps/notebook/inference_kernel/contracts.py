"""Typed contracts for inference kernel inventory.

These are intentionally small and immutable so routing surfaces can reason
about capabilities without loading heavy engine implementations.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any


KNOWN_EPISTEMIC_JOBS = (
    'ingest',
    'structure',
    'relate',
    'evaluate',
    'revise',
    'express',
    'act',
    'validate',
)


KNOWN_INFERENCE_FAMILIES = (
    'lexical',
    'neural',
    'graph',
    'deductive',
    'constraint',
    'egraph',
    'causal',
    'probabilistic',
    'planner',
    'simulator',
    'optimizer',
    'evolutionary',
    'proof',
    'expression',
)


KNOWN_CONSUMES_VIEWS = (
    'text',
    'graph',
    'claims',
    'code AST',
    'CPG',
    'RTL',
    'netlist',
    'constraints',
    'egraph expression',
    'trace',
)


KNOWN_PRODUCES = (
    'edge',
    'claim',
    'proof',
    'counterexample',
    'plan',
    'score',
    'posterior',
    'action',
    'artifact',
    'scene',
)


KNOWN_TRUTH_TYPES = (
    'relevance',
    'plausibility',
    'consequence',
    'feasibility',
    'equivalence',
    'causality',
    'probability',
    'empirical result',
    'proof',
)


KNOWN_VALIDATORS = (
    'test',
    'proof',
    'simulation',
    'benchmark',
    'source corroboration',
    'human review',
    'none',
)


KNOWN_WRITEBACK_POLICIES = (
    'read-only',
    'proposal-only',
    'review-required',
    'direct-write',
)


class ConstrainedChoiceError(ValueError):
    """Raised when contract field constraints are violated."""


def _normalize_tuple(values: tuple[str, ...] | list[str], *, allowed: tuple[str, ...], field_name: str) -> tuple[str, ...]:
    normalized = tuple(str(value).strip() for value in (values or ()))
    for value in normalized:
        if value and value not in allowed:
            raise ConstrainedChoiceError(f"invalid {field_name}: {value}")
    return tuple(dict.fromkeys(normalized))


def _normalize_choice(value: str, *, allowed: tuple[str, ...], field_name: str) -> str:
    normalized = str(value or '').strip()
    if normalized not in allowed:
        raise ConstrainedChoiceError(f"invalid {field_name}: {normalized}")
    return normalized


def _drop_empty(payload: dict[str, Any]) -> dict[str, Any]:
    return {
        key: value
        for key, value in payload.items()
        if value not in (None, '', [], (), {}, '')
    }


@dataclass(frozen=True, slots=True)
class InferenceKernelContract:
    """Non-executable contract for a single inference kernel capability."""

    kernel_id: str
    epistemic_job: str
    inference_family: str
    consumes_view: tuple[str, ...] = ()
    produces: tuple[str, ...] = ()
    truth_type: str = 'relevance'
    validator: str = 'none'
    writeback_policy: str = 'read-only'
    source_module: str = ''
    owner: str = 'unknown'
    description: str = ''
    source: str = ''
    tags: tuple[str, ...] = ()
    metadata: dict[str, Any] = field(default_factory=dict)

    def __post_init__(self) -> None:
        object.__setattr__(self, 'kernel_id', str(self.kernel_id).strip())
        object.__setattr__(self, 'consumes_view', _normalize_tuple(
            self.consumes_view,
            allowed=KNOWN_CONSUMES_VIEWS,
            field_name='consumes_view',
        ))
        object.__setattr__(self, 'produces', _normalize_tuple(
            self.produces,
            allowed=KNOWN_PRODUCES,
            field_name='produces',
        ))
        object.__setattr__(self, 'epistemic_job', _normalize_choice(
            self.epistemic_job,
            allowed=KNOWN_EPISTEMIC_JOBS,
            field_name='epistemic_job',
        ))
        object.__setattr__(self, 'inference_family', _normalize_choice(
            self.inference_family,
            allowed=KNOWN_INFERENCE_FAMILIES,
            field_name='inference_family',
        ))
        object.__setattr__(self, 'truth_type', _normalize_choice(
            self.truth_type,
            allowed=KNOWN_TRUTH_TYPES,
            field_name='truth_type',
        ))
        object.__setattr__(self, 'validator', _normalize_choice(
            self.validator,
            allowed=KNOWN_VALIDATORS,
            field_name='validator',
        ))
        object.__setattr__(self, 'writeback_policy', _normalize_choice(
            self.writeback_policy,
            allowed=KNOWN_WRITEBACK_POLICIES,
            field_name='writeback_policy',
        ))
        object.__setattr__(self, 'tags', tuple(dict.fromkeys(
            str(tag).strip() for tag in (self.tags or ()) if str(tag).strip()
        )))
        if not self.kernel_id:
            raise ConstrainedChoiceError('kernel_id is required')

    def to_dict(self) -> dict[str, Any]:
        return _drop_empty({
            'kernel_id': self.kernel_id,
            'epistemic_job': self.epistemic_job,
            'inference_family': self.inference_family,
            'consumes_view': list(self.consumes_view),
            'produces': list(self.produces),
            'truth_type': self.truth_type,
            'validator': self.validator,
            'writeback_policy': self.writeback_policy,
            'source_module': self.source_module,
            'owner': self.owner,
            'description': self.description,
            'source': self.source,
            'tags': list(self.tags),
            'metadata': dict(self.metadata),
        })
