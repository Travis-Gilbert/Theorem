"""Small probabilistic-programming fallback receipts."""

from __future__ import annotations

from .contracts import PosteriorReceipt


class ProbProgEngine:
    engine = 'beta-binomial-python-fallback'

    def source_reliability(
        self,
        *,
        source_id: str,
        prior_alpha: float = 1.0,
        prior_beta: float = 1.0,
        corroborated: int = 0,
        contradicted: int = 0,
    ) -> PosteriorReceipt:
        alpha = float(prior_alpha) + max(0, int(corroborated))
        beta = float(prior_beta) + max(0, int(contradicted))
        total = alpha + beta
        mean = alpha / total if total else 0.5
        variance = (alpha * beta) / ((total ** 2) * (total + 1)) if total > 0 else 0.0
        return PosteriorReceipt(
            engine=self.engine,
            model_id=f'source-reliability:{source_id}',
            prior={'alpha': float(prior_alpha), 'beta': float(prior_beta)},
            observations={'corroborated': int(corroborated), 'contradicted': int(contradicted)},
            posterior={'alpha': alpha, 'beta': beta, 'mean': mean, 'variance': variance},
            metadata={'source_id': source_id, 'distribution': 'beta'},
        )

    def expected_value_of_information(
        self,
        *,
        current_uncertainty: float,
        expected_uncertainty_after: float,
        decision_value: float,
        validator_cost: float,
    ) -> PosteriorReceipt:
        uncertainty_reduction = max(0.0, float(current_uncertainty) - float(expected_uncertainty_after))
        evi = (uncertainty_reduction * float(decision_value)) - float(validator_cost)
        return PosteriorReceipt(
            engine=self.engine,
            model_id='expected-value-of-information',
            prior={'current_uncertainty': float(current_uncertainty)},
            observations={'expected_uncertainty_after': float(expected_uncertainty_after), 'validator_cost': float(validator_cost)},
            posterior={'expected_value': evi, 'uncertainty_reduction': uncertainty_reduction},
            metadata={'decision_value': float(decision_value)},
        )

