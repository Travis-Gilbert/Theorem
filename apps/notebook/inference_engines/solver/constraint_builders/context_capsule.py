"""Constraint builders for ContextArtifact capsule safety."""

from __future__ import annotations

from typing import Any

from apps.notebook.inference_engines.solver.contracts import SolverConstraint, SolverProblem

from .schema import as_dict_list, nested_get, truthy


PRIVILEGED_CHANNELS = (
    'system_invariants',
    'team_policy',
    'trusted_repo_memory',
    'user_task',
)


def _iter_channel_items(capsule: dict[str, Any], channel: str) -> list[dict[str, Any]]:
    value = capsule.get(channel, [])
    if isinstance(value, dict):
        return [value]
    if isinstance(value, str):
        return [{'text': value}]
    return as_dict_list(value)


def _estimated_tokens(capsule: dict[str, Any]) -> int:
    total = 0
    for value in capsule.values():
        if isinstance(value, str):
            total += max(1, len(value) // 4)
        elif isinstance(value, list):
            total += sum(max(1, len(str(item)) // 4) for item in value)
        elif isinstance(value, dict):
            total += max(1, len(str(value)) // 4)
    return total


def build_context_capsule_problem(
    *,
    capsule: dict[str, Any],
    budget_tokens: int,
    token_ledger: dict[str, Any] | None = None,
    atoms: list[dict[str, Any]] | None = None,
    exports: dict[str, Any] | None = None,
    input_view_refs: tuple[str, ...] = (),
) -> SolverProblem:
    """Build violation constraints for context capsule safety."""
    ledger = dict(token_ledger or {})
    atom_rows = as_dict_list(atoms or [])
    export_payload = dict(exports or {})
    constraints: list[SolverConstraint] = []

    external_in_privileged: list[dict[str, Any]] = []
    for channel in PRIVILEGED_CHANNELS:
        for item in _iter_channel_items(capsule, channel):
            origin = str(item.get('source_channel', '') or item.get('origin', '') or item.get('channel', '')).lower()
            if origin == 'external_content' or item.get('kind') == 'external':
                external_in_privileged.append({'channel': channel, 'item': item})
    constraints.append(SolverConstraint(
        constraint_id='external_content_not_instruction_channel',
        description='External content cannot enter instruction or trusted-memory channels.',
        violated=bool(external_in_privileged),
        counterexample={'placements': external_in_privileged},
    ))

    capsule_tokens = int(
        ledger.get('capsuleTokens')
        or ledger.get('capsule_tokens')
        or ledger.get('total')
        or _estimated_tokens(capsule)
        or 0
    )
    constraints.append(SolverConstraint(
        constraint_id='capsule_within_budget',
        description='Context capsule must fit within budget after pinned requirements.',
        violated=budget_tokens > 0 and capsule_tokens > budget_tokens,
        counterexample={'capsule_tokens': capsule_tokens, 'budget_tokens': budget_tokens},
    ))

    muted_included = []
    for atom in atom_rows:
        metadata = dict(atom.get('metadata') or {})
        is_muted = truthy(atom.get('muted')) or truthy(metadata.get('muted'))
        required = truthy(atom.get('hard_required')) or truthy(metadata.get('hard_required')) or truthy(metadata.get('pinned_required'))
        if truthy(atom.get('included')) and is_muted and not required:
            muted_included.append(atom)
    constraints.append(SolverConstraint(
        constraint_id='muted_node_requires_hard_requirement',
        description='Muted nodes cannot be included unless a hard requirement exists.',
        violated=bool(muted_included),
        counterexample={'atoms': muted_included},
    ))

    public_export = (
        truthy(export_payload.get('public'))
        or str(export_payload.get('visibility', '')).lower() == 'public'
        or truthy(nested_get(export_payload, 'signed_json', 'public', default=False))
    )
    private_atoms = []
    for atom in atom_rows:
        metadata = dict(atom.get('metadata') or {})
        private = (
            truthy(atom.get('private'))
            or truthy(metadata.get('private'))
            or str(metadata.get('source_visibility', '')).lower() == 'private'
        )
        if public_export and private:
            private_atoms.append(atom)
    constraints.append(SolverConstraint(
        constraint_id='private_source_not_exported',
        description='Private sources cannot be included in public exports.',
        violated=bool(private_atoms),
        counterexample={'private_atoms': private_atoms, 'exports': export_payload},
    ))

    return SolverProblem(
        target='context_capsule_safety',
        constraints=tuple(constraints),
        input_view_refs=input_view_refs,
        metadata={'budget_tokens': budget_tokens, 'capsule_tokens': capsule_tokens},
    )


def build_context_capsule_problem_from_artifact(artifact, *, input_view_refs: tuple[str, ...] = ()) -> SolverProblem:
    return build_context_capsule_problem(
        capsule=dict(getattr(artifact, 'capsule', {}) or {}),
        budget_tokens=int(getattr(artifact, 'budget_tokens', 0) or 0),
        token_ledger=dict(getattr(artifact, 'token_ledger', {}) or {}),
        atoms=list(getattr(artifact, 'atoms', []) or []),
        exports=dict(getattr(artifact, 'exports', {}) or {}),
        input_view_refs=input_view_refs or (f'context_artifact:{getattr(artifact, "pk", "")}',),
    )

