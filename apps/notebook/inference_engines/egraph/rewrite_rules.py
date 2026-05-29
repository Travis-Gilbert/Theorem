"""Equivalence-preserving rewrite rules for the Python fallback."""

from __future__ import annotations

from dataclasses import replace

from .contracts import EGraphExpression, RewriteStep
from .cost_models import ExtractionCostModel


def _item_key(item: dict) -> tuple[str, str, str]:
    return (
        str(item.get('channel', '') or ''),
        str(item.get('obligation_id', '') or item.get('evidence_id', '') or item.get('id', '')),
        str(item.get('semantic_hash', '') or item.get('content_hash', '') or item.get('text', '')),
    )


def dedupe_same_obligation(
    expression: EGraphExpression,
    cost_model: ExtractionCostModel,
) -> tuple[EGraphExpression, RewriteStep | None]:
    seen: set[tuple[str, str, str]] = set()
    next_items: list[dict] = []
    removed: list[dict] = []
    for item in expression.items:
        key = _item_key(item)
        if key in seen and not item.get('hard_required'):
            removed.append(dict(item))
            continue
        seen.add(key)
        next_items.append(dict(item))
    if not removed:
        return expression, None

    before_cost = cost_model.cost(expression)
    rewritten = replace(expression, items=tuple(next_items), expression_hash='')
    rewritten.__post_init__()
    after_cost = cost_model.cost(rewritten)
    return rewritten, RewriteStep(
        rule_id='dedupe_same_obligation',
        before_hash=expression.expression_hash,
        after_hash=rewritten.expression_hash,
        reason='Removed duplicate non-required context items with the same obligation and channel.',
        delta_cost=round(after_cost - before_cost, 6),
        data={'removed_count': len(removed)},
    )


def drop_empty_optional(
    expression: EGraphExpression,
    cost_model: ExtractionCostModel,
) -> tuple[EGraphExpression, RewriteStep | None]:
    next_items = [
        dict(item)
        for item in expression.items
        if item.get('hard_required') or str(item.get('text', '') or item.get('summary', '')).strip()
    ]
    if len(next_items) == len(expression.items):
        return expression, None

    before_cost = cost_model.cost(expression)
    rewritten = replace(expression, items=tuple(next_items), expression_hash='')
    rewritten.__post_init__()
    after_cost = cost_model.cost(rewritten)
    return rewritten, RewriteStep(
        rule_id='drop_empty_optional',
        before_hash=expression.expression_hash,
        after_hash=rewritten.expression_hash,
        reason='Removed optional empty context items without changing represented obligations.',
        delta_cost=round(after_cost - before_cost, 6),
        data={'removed_count': len(expression.items) - len(next_items)},
    )


DEFAULT_REWRITE_RULES = (drop_empty_optional, dedupe_same_obligation)

