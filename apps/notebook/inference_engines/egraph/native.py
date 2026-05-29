"""Optional Rust/egg-backed EGraph adapter."""

from __future__ import annotations

import json
from typing import Any

from apps.notebook.inference_engines.common import stable_json
from apps.notebook.inference_kernel.settings import bgi_native_symbolic_enabled

from .contracts import EGraphExpression, EGraphReceipt, RewriteStep
from .engine import EGraphTheorem


def _native_module():
    if not bgi_native_symbolic_enabled():
        return None
    try:
        import theseus_native  # type: ignore[import-not-found]
    except Exception:
        return None
    if not hasattr(theseus_native, 'bgi_egraph_extract_context_pack_json'):
        return None
    return theseus_native


def _receipt_from_dict(payload: dict[str, Any]) -> EGraphReceipt:
    extraction = dict(payload.get('extraction') or {})
    return EGraphReceipt(
        engine=str(payload.get('engine') or 'egraph-theorem'),
        input_hash=str(payload.get('input_hash') or ''),
        output_hash=str(payload.get('output_hash') or ''),
        domain=str(payload.get('domain') or 'context_pack'),
        equivalent=bool(payload.get('equivalent')),
        original_cost=float(payload.get('original_cost') or 0.0),
        extracted_cost=float(payload.get('extracted_cost') or 0.0),
        extraction=EGraphExpression(
            expression_id=str(extraction.get('expression_id') or ''),
            domain=str(extraction.get('domain') or 'context_pack'),
            items=tuple(extraction.get('items') or ()),
            metadata=dict(extraction.get('metadata') or {}),
            expression_hash=str(extraction.get('expression_hash') or payload.get('output_hash') or ''),
        ),
        rewrite_trace=tuple(
            RewriteStep(
                rule_id=str(step.get('rule_id') or ''),
                before_hash=str(step.get('before_hash') or ''),
                after_hash=str(step.get('after_hash') or ''),
                reason=str(step.get('reason') or ''),
                delta_cost=float(step.get('delta_cost') or 0.0),
                data=dict(step.get('data') or {}),
            )
            for step in payload.get('rewrite_trace') or []
        ),
        native_backend=str(payload.get('native_backend') or 'rust-egg-context-pack'),
    )


class NativeEGraphTheorem(EGraphTheorem):
    """Use the Rust egg-backed context-pack extractor when available."""

    def context_pack(
        self,
        *,
        expression_id: str,
        items: list[dict],
        cost_config: dict | None = None,
    ) -> EGraphReceipt:
        native = _native_module()
        if native is None:
            return super().context_pack(
                expression_id=expression_id,
                items=items,
                cost_config=cost_config,
            )
        payload = {
            'expression_id': expression_id,
            'items': items,
            'cost_config': dict(cost_config or {}),
        }
        return _receipt_from_dict(
            json.loads(native.bgi_egraph_extract_context_pack_json(stable_json(payload))),
        )
