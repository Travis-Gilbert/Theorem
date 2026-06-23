"""Configurable extraction costs for the EGraph fallback."""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any

from .contracts import EGraphExpression


@dataclass(frozen=True, slots=True)
class ExtractionCostModel:
    token_weight: float = 1.0
    item_weight: float = 0.1
    channel_penalties: dict[str, float] = field(default_factory=dict)

    def cost(self, expression: EGraphExpression) -> float:
        total = 0.0
        for item in expression.items:
            tokens = float(item.get('tokens') or len(str(item.get('text', ''))) / 4.0 or 1.0)
            channel = str(item.get('channel', '') or '')
            total += (tokens * self.token_weight) + self.item_weight
            total += float(self.channel_penalties.get(channel, 0.0))
        return round(total, 6)


def cost_model_from_domain(domain: str, config: dict[str, Any] | None = None) -> ExtractionCostModel:
    payload = dict(config or {})
    if domain == 'context_pack':
        return ExtractionCostModel(
            token_weight=float(payload.get('token_weight', 1.0)),
            item_weight=float(payload.get('item_weight', 0.05)),
            channel_penalties=dict(payload.get('channel_penalties', {})),
        )
    return ExtractionCostModel(
        token_weight=float(payload.get('token_weight', 1.0)),
        item_weight=float(payload.get('item_weight', 0.1)),
        channel_penalties=dict(payload.get('channel_penalties', {})),
    )

