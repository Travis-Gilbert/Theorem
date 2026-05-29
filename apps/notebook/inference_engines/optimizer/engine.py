"""Deterministic optimizer fallback for context and validator scheduling."""

from __future__ import annotations

from .contracts import OptimizationCandidate, OptimizationProblem, OptimizationResult


class OptimizerEngine:
    engine = 'python-deterministic-optimizer'

    def optimize(self, problem: OptimizationProblem) -> OptimizationResult:
        selected: list[OptimizationCandidate] = []
        remaining = float(problem.budget)

        hard_required = sorted(
            [candidate for candidate in problem.candidates if candidate.hard_required],
            key=lambda item: item.cost,
        )
        for candidate in hard_required:
            if candidate.cost > remaining:
                return OptimizationResult(
                    engine=self.engine,
                    problem_hash=problem.problem_hash,
                    status='infeasible',
                    selected=tuple(selected),
                    total_value=sum(item.value for item in selected),
                    total_cost=sum(item.cost for item in selected),
                    reason=f'Hard-required candidate exceeds remaining budget: {candidate.candidate_id}',
                )
            selected.append(candidate)
            remaining -= candidate.cost

        covered = {tag for candidate in selected for tag in candidate.tags}
        for tag in problem.min_tag_coverage:
            if tag in covered:
                continue
            candidates = [
                item for item in problem.candidates
                if item not in selected and tag in item.tags and item.cost <= remaining
            ]
            if not candidates:
                return OptimizationResult(
                    engine=self.engine,
                    problem_hash=problem.problem_hash,
                    status='infeasible',
                    selected=tuple(selected),
                    total_value=sum(item.value for item in selected),
                    total_cost=sum(item.cost for item in selected),
                    reason=f'No affordable candidate can cover required tag: {tag}',
                )
            best = max(candidates, key=lambda item: (item.value / max(item.cost, 0.0001), item.value))
            selected.append(best)
            remaining -= best.cost
            covered.update(best.tags)

        optional = [
            item for item in problem.candidates
            if item not in selected and item.cost <= remaining
        ]
        for candidate in sorted(optional, key=lambda item: (item.value / max(item.cost, 0.0001), item.value), reverse=True):
            if candidate.cost <= remaining:
                selected.append(candidate)
                remaining -= candidate.cost

        return OptimizationResult(
            engine=self.engine,
            problem_hash=problem.problem_hash,
            status='feasible',
            selected=tuple(selected),
            total_value=round(sum(item.value for item in selected), 6),
            total_cost=round(sum(item.cost for item in selected), 6),
            reason='Selected hard requirements, tag coverage, then best value-per-cost candidates.',
        )

    def schedule_validators(self, validators: list[dict], *, budget: float) -> OptimizationResult:
        candidates = tuple(
            OptimizationCandidate(
                candidate_id=str(item['id']),
                value=float(item.get('expected_value', 0.0)),
                cost=float(item.get('cost', 1.0)),
                tags=tuple(item.get('tags', ())),
                hard_required=bool(item.get('hard_required', False)),
                metadata=dict(item),
            )
            for item in validators
        )
        return self.optimize(OptimizationProblem(
            problem_id='validator-schedule',
            objective='max_expected_validation_value',
            candidates=candidates,
            budget=budget,
        ))

