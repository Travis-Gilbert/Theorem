"""Receipt adapters for EGraph theorem results."""

from __future__ import annotations

from .contracts import EGraphReceipt


def rewrite_trace_as_context_atom(receipt: EGraphReceipt) -> dict:
    return {
        'kind': 'policy',
        'title': f'EGraph rewrite trace for {receipt.domain}',
        'content_hash': receipt.output_hash,
        'included': True,
        'reason': (
            f'Equivalent extraction lowered cost from {receipt.original_cost} '
            f'to {receipt.extracted_cost}.'
        ),
        'metadata': {
            'engine': receipt.engine,
            'native_backend': receipt.native_backend,
            'rewrite_trace': [step.to_dict() for step in receipt.rewrite_trace],
        },
    }

