"""Causal engine fallback that keeps assumptions explicit."""

from __future__ import annotations

from .contracts import CausalReceipt


class CausalEngine:
    engine = 'assumption-bound-causal-fallback'

    def intervention_effect(
        self,
        *,
        question_id: str,
        treatment: str,
        outcome: str,
        treated_mean: float | None = None,
        control_mean: float | None = None,
        assumptions: tuple[str, ...] = (),
        confounders: tuple[str, ...] = (),
    ) -> CausalReceipt:
        required = (
            'exchangeability',
            'positivity',
            'consistency',
        )
        all_assumptions = tuple(dict.fromkeys((*assumptions, *required)))
        if treated_mean is None or control_mean is None:
            return CausalReceipt(
                engine=self.engine,
                question_id=question_id,
                assumptions=all_assumptions,
                identifiability_status='unknown',
                recommendation='Collect treated and control outcome summaries before estimating intervention effect.',
                metadata={'treatment': treatment, 'outcome': outcome, 'confounders': list(confounders)},
            )
        estimate = float(treated_mean) - float(control_mean)
        return CausalReceipt(
            engine=self.engine,
            question_id=question_id,
            assumptions=all_assumptions,
            identifiability_status='identified_under_assumptions',
            estimate=estimate,
            recommendation='Treat this as an assumption-bound causal estimate, not proof.',
            metadata={'treatment': treatment, 'outcome': outcome, 'confounders': list(confounders)},
        )

    def recommend_experiment(
        self,
        *,
        question_id: str,
        candidate_controls: tuple[str, ...],
        uncertainty: float,
    ) -> CausalReceipt:
        controls = ', '.join(candidate_controls[:3]) if candidate_controls else 'predefined controls'
        return CausalReceipt(
            engine=self.engine,
            question_id=question_id,
            assumptions=('intervention is feasible', 'measurement is reliable'),
            identifiability_status='experiment_recommended',
            recommendation=f'Run a controlled comparison with {controls} before promoting causal claims.',
            metadata={'uncertainty': float(uncertainty), 'candidate_controls': list(candidate_controls)},
        )

