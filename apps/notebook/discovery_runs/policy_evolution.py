"""Context packing policy evolution skeleton (BGI3 phase 6).

Implements an OpenEvolve-style MAP-Elites loop for ContextAtom packing policies.
A "policy" is a small bundle of ranking weights that the packer would consult
when scoring evidence atoms. The loop:

1. Sample synthetic packing tasks (each with a target evidence set, distractors,
   and a budget).
2. Mutate a parent policy into a candidate child policy.
3. Score each candidate against the synthetic task pool using a deterministic
   evaluator (token economy + evidence coverage + diversity penalty).
4. Archive elites per niche via the native-aware
   ``apps.notebook.inference_engines.evolution.native.NativeEvolutionEngine``.

We deliberately do NOT depend on the upstream `openevolve` package or on any
LLM call: this skeleton runs offline, deterministically, and gives the rest of
the system a measurable feedback hook before real workloads exist.

The output contract is a list of elite ``EvolutionCandidate`` rows that
``apps.notebook.discovery_runs.archives.archive_candidates`` already persists
into ``DiscoveryCandidateArchiveEntry`` as proposal-only rows.
"""

from __future__ import annotations

import math
import random
from collections.abc import Callable, Sequence
from dataclasses import asdict, dataclass, field, replace
from typing import Any

from apps.notebook.inference_engines.evolution.contracts import EvolutionCandidate
from apps.notebook.inference_engines.evolution.native import NativeEvolutionEngine


@dataclass(frozen=True, slots=True)
class PackingPolicy:
    """Bundle of ranking weights consulted by the packer.

    Each field is a multiplier applied to a single signal in the ranking
    pipeline. Defaults reproduce the "no policy applied" state. Values are
    bounded to [0, 2] during evolution to avoid runaway weights.
    """

    policy_id: str
    exact_symbol_target: float = 1.0
    data_flow_risk: float = 1.0
    caller_proximity: float = 1.0
    source_span_confidence: float = 1.0
    doc_freshness: float = 1.0
    diversity_penalty: float = 0.5
    token_penalty: float = 0.001
    postmortem_bonus: float = 0.1
    tension_coverage_requirement: float = 0.0
    metadata: dict[str, Any] = field(default_factory=dict)

    def to_dict(self) -> dict[str, Any]:
        return asdict(self)


@dataclass(frozen=True, slots=True)
class SyntheticTask:
    """A fixture task used to score packing policies."""

    task_id: str
    niche: str
    budget_tokens: int
    target_atom_ids: tuple[str, ...]
    distractor_atom_ids: tuple[str, ...]
    atoms: tuple[dict[str, Any], ...]

    def to_dict(self) -> dict[str, Any]:
        return {
            'task_id': self.task_id,
            'niche': self.niche,
            'budget_tokens': self.budget_tokens,
            'target_atom_ids': list(self.target_atom_ids),
            'distractor_atom_ids': list(self.distractor_atom_ids),
            'atoms': [dict(atom) for atom in self.atoms],
        }


@dataclass(frozen=True, slots=True)
class PolicyScore:
    """Score breakdown for one (policy, task) pair."""

    task_id: str
    coverage: float
    diversity: float
    token_efficiency: float
    composite: float

    def to_dict(self) -> dict[str, Any]:
        return asdict(self)


@dataclass(frozen=True, slots=True)
class PolicyEvaluation:
    policy: PackingPolicy
    per_task: tuple[PolicyScore, ...]
    average_score: float
    behaviour_descriptor: tuple[float, float]

    def to_dict(self) -> dict[str, Any]:
        return {
            'policy': self.policy.to_dict(),
            'per_task': [score.to_dict() for score in self.per_task],
            'average_score': self.average_score,
            'behaviour_descriptor': list(self.behaviour_descriptor),
        }


def _atom_score(policy: PackingPolicy, atom: dict[str, Any]) -> float:
    """Apply the policy's weights to one atom's signals."""
    base = float(atom.get('score') or 0.0)
    signals = atom.get('signals') if isinstance(atom.get('signals'), dict) else {}
    boost = 0.0
    boost += policy.exact_symbol_target * float(signals.get('exact_symbol_target') or 0.0)
    boost += policy.data_flow_risk * float(signals.get('data_flow_risk') or 0.0)
    boost += policy.caller_proximity * float(signals.get('caller_proximity') or 0.0)
    boost += (policy.source_span_confidence - 1.0) * float(signals.get('source_span_confidence') or 1.0)
    boost += (policy.doc_freshness - 1.0) * float(signals.get('doc_freshness') or 1.0)
    if atom.get('kind') == 'postmortem':
        boost += policy.postmortem_bonus
    return max(0.0, base + boost)


def _pack_with_policy(
    policy: PackingPolicy,
    task: SyntheticTask,
) -> tuple[list[dict[str, Any]], int]:
    """Greedy fill a capsule under the policy. Returns (included_atoms, tokens_used)."""
    scored = sorted(
        ((_atom_score(policy, atom), atom) for atom in task.atoms),
        key=lambda pair: pair[0],
        reverse=True,
    )
    remaining = task.budget_tokens
    included: list[dict[str, Any]] = []
    used_kinds: set[str] = set()
    for score, atom in scored:
        cost = int(atom.get('token_cost') or 100)
        diversity_factor = 1.0
        if policy.diversity_penalty > 0 and atom.get('kind') in used_kinds:
            diversity_factor = max(0.0, 1.0 - policy.diversity_penalty)
        if score * diversity_factor <= 0.0:
            continue
        if cost > remaining:
            continue
        cost_with_penalty = cost + int(round(policy.token_penalty * cost * 100))
        if cost_with_penalty > remaining:
            continue
        included.append(dict(atom))
        used_kinds.add(str(atom.get('kind') or ''))
        remaining -= cost_with_penalty
    return included, task.budget_tokens - remaining


def evaluate_policy(
    policy: PackingPolicy,
    tasks: Sequence[SyntheticTask],
) -> PolicyEvaluation:
    """Score one policy over the fixture pool."""
    per_task: list[PolicyScore] = []
    avg_acc = 0.0
    diversity_acc = 0.0
    token_acc = 0.0
    for task in tasks:
        included, tokens_used = _pack_with_policy(policy, task)
        included_ids = {atom.get('atom_id') for atom in included}
        target_count = max(1, len(task.target_atom_ids))
        coverage = sum(1 for tid in task.target_atom_ids if tid in included_ids) / target_count
        kinds = {atom.get('kind') for atom in included}
        diversity = 0.0 if not included else len(kinds) / max(1, len(included))
        token_efficiency = 1.0 if task.budget_tokens <= 0 else 1.0 - (tokens_used / max(1, task.budget_tokens))
        token_efficiency = max(0.0, token_efficiency)
        composite = (0.6 * coverage) + (0.2 * diversity) + (0.2 * token_efficiency)
        per_task.append(PolicyScore(
            task_id=task.task_id,
            coverage=coverage,
            diversity=diversity,
            token_efficiency=token_efficiency,
            composite=composite,
        ))
        avg_acc += composite
        diversity_acc += diversity
        token_acc += token_efficiency
    n = max(1, len(tasks))
    avg = avg_acc / n
    descriptor = (
        round(diversity_acc / n, 4),
        round(token_acc / n, 4),
    )
    return PolicyEvaluation(
        policy=policy,
        per_task=tuple(per_task),
        average_score=round(avg, 4),
        behaviour_descriptor=descriptor,
    )


def mutate_policy(parent: PackingPolicy, *, rng: random.Random, sigma: float = 0.15) -> PackingPolicy:
    """Gaussian mutation on the policy's weights, clamped to [0, 2]."""

    def jitter(value: float, *, lo: float = 0.0, hi: float = 2.0) -> float:
        nudged = value + rng.gauss(0.0, sigma)
        return max(lo, min(hi, nudged))

    new_id = f'{parent.policy_id}-m{rng.randrange(1_000_000):06d}'
    return replace(
        parent,
        policy_id=new_id,
        exact_symbol_target=jitter(parent.exact_symbol_target),
        data_flow_risk=jitter(parent.data_flow_risk),
        caller_proximity=jitter(parent.caller_proximity),
        source_span_confidence=jitter(parent.source_span_confidence, lo=0.5, hi=1.5),
        doc_freshness=jitter(parent.doc_freshness, lo=0.5, hi=1.5),
        diversity_penalty=jitter(parent.diversity_penalty, lo=0.0, hi=1.0),
        token_penalty=jitter(parent.token_penalty, lo=0.0, hi=0.05),
        postmortem_bonus=jitter(parent.postmortem_bonus, lo=0.0, hi=0.5),
        tension_coverage_requirement=jitter(parent.tension_coverage_requirement, lo=0.0, hi=1.0),
        metadata={'parent_id': parent.policy_id},
    )


def _evaluation_to_evolution_candidate(
    evaluation: PolicyEvaluation,
    *,
    niche: str,
    novelty: float,
) -> EvolutionCandidate:
    return EvolutionCandidate(
        candidate_id=evaluation.policy.policy_id,
        niche=niche,
        score=evaluation.average_score,
        novelty=novelty,
        payload={
            'policy': evaluation.policy.to_dict(),
            'per_task': [s.to_dict() for s in evaluation.per_task],
            'behaviour_descriptor': list(evaluation.behaviour_descriptor),
        },
    )


def _novelty_against_archive(
    descriptor: tuple[float, float],
    archive_descriptors: Sequence[tuple[float, float]],
) -> float:
    if not archive_descriptors:
        return 1.0
    distances = [
        math.sqrt((descriptor[0] - d[0]) ** 2 + (descriptor[1] - d[1]) ** 2)
        for d in archive_descriptors
    ]
    return round(min(1.0, sum(distances) / len(distances)), 4)


def evolve_packing_policies(
    *,
    seed_policies: Sequence[PackingPolicy],
    tasks: Sequence[SyntheticTask],
    generations: int = 6,
    population_size: int = 8,
    elites_per_niche: int = 2,
    seed: int = 1729,
    niche_resolver: Callable[[PolicyEvaluation], str] | None = None,
) -> dict[str, Any]:
    """Run the evolution loop and return the archive receipt + best policy.

    The loop is deterministic given ``seed`` and ``tasks``. ``niche_resolver``
    lets callers bin policies by behaviour (e.g. by task type or descriptor
    bucket); the default uses the first task's niche.
    """
    if not seed_policies:
        raise ValueError('evolve_packing_policies requires at least one seed_policy')
    if not tasks:
        raise ValueError('evolve_packing_policies requires at least one task')

    rng = random.Random(seed)
    if niche_resolver is None:
        default_niche = tasks[0].niche

        def niche_resolver(_: PolicyEvaluation) -> str:
            return default_niche

    population: list[PolicyEvaluation] = [evaluate_policy(p, tasks) for p in seed_policies]

    history: list[dict[str, Any]] = []
    for generation in range(generations):
        offspring: list[PolicyEvaluation] = []
        # parents: top half by composite score
        ranked = sorted(population, key=lambda e: e.average_score, reverse=True)
        parents = ranked[: max(1, len(ranked) // 2)]
        while len(offspring) < population_size:
            parent = rng.choice(parents).policy
            child = mutate_policy(parent, rng=rng)
            offspring.append(evaluate_policy(child, tasks))
        # Combine, keep top population_size for next generation, but always
        # carry the best of the parents to avoid regression.
        combined = sorted(parents + offspring, key=lambda e: e.average_score, reverse=True)
        population = combined[:population_size]
        history.append({
            'generation': generation,
            'best_score': population[0].average_score,
            'best_policy_id': population[0].policy.policy_id,
            'population_size': len(population),
        })

    candidates: list[EvolutionCandidate] = []
    archive_descriptors: list[tuple[float, float]] = []
    for evaluation in population:
        novelty = _novelty_against_archive(evaluation.behaviour_descriptor, archive_descriptors)
        archive_descriptors.append(evaluation.behaviour_descriptor)
        candidates.append(_evaluation_to_evolution_candidate(
            evaluation,
            niche=niche_resolver(evaluation),
            novelty=novelty,
        ))

    archive_receipt = NativeEvolutionEngine().archive(
        candidates,
        elites_per_niche=elites_per_niche,
    )

    best = max(population, key=lambda e: e.average_score)
    return {
        'best_policy': best.policy.to_dict(),
        'best_score': best.average_score,
        'history': history,
        'population': [evaluation.to_dict() for evaluation in population],
        'archive': archive_receipt.to_dict(),
    }


# ---------------------------------------------------------------------------
# Synthetic task seeds
#
# Lightweight fixture pool useful for offline runs and tests. Real workloads
# should produce task seeds from genuine ContextArtifact + DiscoveryRun traces;
# the synthetic pool keeps the loop deterministic and keeps tests honest.
# ---------------------------------------------------------------------------


def make_synthetic_task_pool(*, seed: int = 7) -> list[SyntheticTask]:
    rng = random.Random(seed)
    tasks: list[SyntheticTask] = []
    kinds = ('code_symbol', 'doc_section', 'code_flow', 'claim', 'postmortem', 'table')
    niches = ('code-review', 'research', 'summary', 'q-and-a')
    for idx in range(8):
        niche = niches[idx % len(niches)]
        atoms: list[dict[str, Any]] = []
        target_ids: list[str] = []
        distractor_ids: list[str] = []
        # 3 target atoms with strong signals
        for t in range(3):
            atom_id = f'task{idx}-target{t}'
            target_ids.append(atom_id)
            atoms.append({
                'atom_id': atom_id,
                'kind': kinds[t % len(kinds)],
                'score': 0.7 + 0.05 * t,
                'token_cost': 120 + 10 * t,
                'signals': {
                    'exact_symbol_target': 0.25 if t == 0 else 0.0,
                    'data_flow_risk': 0.2 if kinds[t % len(kinds)] == 'code_flow' else 0.0,
                    'caller_proximity': 0.1 if kinds[t % len(kinds)] == 'code_symbol' else 0.0,
                    'source_span_confidence': 0.95,
                    'doc_freshness': 1.0,
                },
            })
        # 5 distractors with weaker signals
        for d in range(5):
            atom_id = f'task{idx}-noise{d}'
            distractor_ids.append(atom_id)
            atoms.append({
                'atom_id': atom_id,
                'kind': kinds[(d + 2) % len(kinds)],
                'score': rng.uniform(0.2, 0.5),
                'token_cost': rng.randint(80, 200),
                'signals': {
                    'exact_symbol_target': 0.0,
                    'data_flow_risk': 0.0,
                    'caller_proximity': 0.0,
                    'source_span_confidence': rng.uniform(0.6, 0.9),
                    'doc_freshness': rng.uniform(0.6, 1.0),
                },
            })
        tasks.append(SyntheticTask(
            task_id=f'task{idx:02d}',
            niche=niche,
            budget_tokens=900,
            target_atom_ids=tuple(target_ids),
            distractor_atom_ids=tuple(distractor_ids),
            atoms=tuple(atoms),
        ))
    return tasks


def default_seed_policies() -> list[PackingPolicy]:
    return [
        PackingPolicy(policy_id='seed:identity'),
        PackingPolicy(
            policy_id='seed:safety-aware',
            data_flow_risk=1.5,
            postmortem_bonus=0.3,
            diversity_penalty=0.4,
        ),
        PackingPolicy(
            policy_id='seed:doc-heavy',
            doc_freshness=1.4,
            source_span_confidence=1.2,
            token_penalty=0.005,
        ),
    ]
