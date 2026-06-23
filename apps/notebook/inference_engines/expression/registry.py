"""Expression engine registry."""

from __future__ import annotations

from .brief_compiler import compile_brief
from .contracts import ExpressionEngineRegistration, ExpressionInput, ExpressionResult
from .llm_speaker import speak_with_fallback
from .report_compiler import compile_report
from .scene_compiler import compile_scene


class ExpressionRegistry:
    def __init__(self) -> None:
        self._entries = {
            'deterministic_brief': ExpressionEngineRegistration(
                engine_id='deterministic_brief',
                artifact_type='brief',
                renderer=compile_brief,
                description='Deterministic one-paragraph operator brief.',
            ),
            'structured_report': ExpressionEngineRegistration(
                engine_id='structured_report',
                artifact_type='report',
                renderer=compile_report,
                description='Structured report with status, engine, and receipt sections.',
            ),
            'scene_package': ExpressionEngineRegistration(
                engine_id='scene_package',
                artifact_type='scene_package',
                renderer=compile_scene,
                description='Scene OS-compatible package for receipt panels.',
            ),
            'llm_speaker_fallback': ExpressionEngineRegistration(
                engine_id='llm_speaker_fallback',
                artifact_type='text',
                renderer=speak_with_fallback,
                description='LLM expression adapter with deterministic fallback.',
            ),
        }

    def render(self, engine_id: str, result: dict, *, metadata: dict | None = None) -> ExpressionResult:
        entry = self._entries[engine_id]
        return entry.renderer(ExpressionInput(result=result, metadata=dict(metadata or {})))

    def all(self) -> tuple[ExpressionEngineRegistration, ...]:
        return tuple(self._entries[key] for key in sorted(self._entries))


_REGISTRY = ExpressionRegistry()


def get_expression_registry() -> ExpressionRegistry:
    return _REGISTRY

