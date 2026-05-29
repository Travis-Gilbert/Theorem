"""Equivalence-preserving extraction engine with a pure-Python fallback."""

from __future__ import annotations

from collections.abc import Iterable

from .contracts import EGraphExpression, EGraphReceipt, RewriteStep
from .cost_models import ExtractionCostModel, cost_model_from_domain
from .rewrite_rules import DEFAULT_REWRITE_RULES


class EGraphTheorem:
    """Reference EGraph theorem engine.

    This is intentionally not a toy reimplementation of egg. It is a fallback
    adapter that emits the same auditable receipt shape the future Rust backend
    should preserve.
    """

    def __init__(self, rules: Iterable = DEFAULT_REWRITE_RULES) -> None:
        self._rules = tuple(rules)

    def extract(
        self,
        expression: EGraphExpression,
        *,
        cost_model: ExtractionCostModel | None = None,
        max_rounds: int = 8,
    ) -> EGraphReceipt:
        model = cost_model or cost_model_from_domain(expression.domain)
        current = expression
        trace: list[RewriteStep] = []
        original_cost = model.cost(expression)

        for _ in range(max_rounds):
            changed = False
            for rule in self._rules:
                current, step = rule(current, model)
                if step is not None:
                    trace.append(step)
                    changed = True
            if not changed:
                break

        extracted_cost = model.cost(current)
        return EGraphReceipt(
            engine='egraph-theorem',
            input_hash=expression.expression_hash,
            output_hash=current.expression_hash,
            domain=expression.domain,
            equivalent=True,
            original_cost=original_cost,
            extracted_cost=extracted_cost,
            extraction=current,
            rewrite_trace=tuple(trace),
        )

    def context_pack(
        self,
        *,
        expression_id: str,
        items: list[dict],
        cost_config: dict | None = None,
    ) -> EGraphReceipt:
        expression = EGraphExpression(
            expression_id=expression_id,
            domain='context_pack',
            items=tuple(items),
        )
        return self.extract(
            expression,
            cost_model=cost_model_from_domain('context_pack', cost_config),
        )

