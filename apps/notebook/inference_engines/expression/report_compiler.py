"""Structured report compiler for inference receipts."""

from __future__ import annotations

from .contracts import ExpressionInput, ExpressionResult


def compile_report(input_payload: ExpressionInput) -> ExpressionResult:
    result = input_payload.result
    sections = [
        {'title': 'Status', 'body': str(result.get('status') or result.get('identifiability_status') or 'unknown')},
        {'title': 'Engine', 'body': str(result.get('engine') or result.get('provider') or input_payload.metadata.get('engine', 'unknown'))},
    ]
    if result.get('counterexample'):
        sections.append({'title': 'Counterexample', 'body': result['counterexample']})
    if result.get('rewrite_trace'):
        sections.append({'title': 'Rewrite Trace', 'body': result['rewrite_trace']})
    if result.get('writeback_proposals'):
        sections.append({'title': 'Writeback Proposals', 'body': result['writeback_proposals']})
    return ExpressionResult(
        engine_id='structured_report',
        artifact_type='report',
        payload={
            'title': str(result.get('title') or 'Inference Report'),
            'sections': sections,
            'metadata': dict(input_payload.metadata),
        },
    )

