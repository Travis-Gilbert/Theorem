"""LLM speaker adapter placeholder that preserves additive behavior."""

from __future__ import annotations

from .brief_compiler import compile_brief
from .contracts import ExpressionInput, ExpressionResult


def speak_with_fallback(input_payload: ExpressionInput) -> ExpressionResult:
    brief = compile_brief(input_payload)
    payload = dict(brief.payload)
    payload['speaker'] = 'llm_unavailable_deterministic_fallback'
    return ExpressionResult(
        engine_id='llm_speaker_fallback',
        artifact_type='text',
        payload=payload,
    )

