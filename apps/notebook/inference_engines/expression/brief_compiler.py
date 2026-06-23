"""Deterministic brief compiler for inference results."""

from __future__ import annotations

from .contracts import ExpressionInput, ExpressionResult


def compile_brief(input_payload: ExpressionInput) -> ExpressionResult:
    result = input_payload.result
    title = str(result.get('title') or result.get('engine') or result.get('provider') or 'Inference result')
    status = str(result.get('status') or result.get('identifiability_status') or 'unknown')
    summary_parts = [f'{title}: {status}.']
    if result.get('reason'):
        summary_parts.append(str(result['reason']))
    if result.get('recommendation'):
        summary_parts.append(str(result['recommendation']))
    if result.get('derived_count') is not None:
        summary_parts.append(f"Derived facts: {result['derived_count']}.")
    return ExpressionResult(
        engine_id='deterministic_brief',
        artifact_type='brief',
        payload={
            'title': title,
            'status': status,
            'summary': ' '.join(summary_parts),
            'source_result_hash': result.get('receipt_hash') or result.get('formula_hash') or result.get('fact_pack_hash', ''),
        },
    )

