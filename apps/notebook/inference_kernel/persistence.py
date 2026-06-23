"""Persistence helpers for routed inference-kernel executions."""

from __future__ import annotations

from typing import Any

from django.utils import timezone

from apps.notebook.inference_engines.common import stable_hash
from apps.notebook.models import (
    DiscoveryRun,
    KernelResultReceipt,
    KernelRun,
)

from .contracts import InferenceKernelContract


def _now_ms(started_at) -> int:
    return max(0, int((timezone.now() - started_at).total_seconds() * 1000))


def _result_status(result_payload: dict[str, Any]) -> str:
    status = str(result_payload.get('status') or '').lower()
    if status in {'passed', 'succeeded', 'success', 'sat', 'unsat', 'feasible', 'admitted'}:
        return KernelResultReceipt.Status.PASSED
    if status in {'failed', 'invalid', 'dropped', 'infeasible'}:
        return KernelResultReceipt.Status.FAILED
    if status in {'skipped', 'cancelled', 'canceled'}:
        return KernelResultReceipt.Status.SKIPPED
    return KernelResultReceipt.Status.UNKNOWN


def begin_kernel_run(
    contract: InferenceKernelContract,
    *,
    request_payload: dict[str, Any],
    budget: dict[str, Any] | None = None,
    metadata: dict[str, Any] | None = None,
    discovery_run_id: str | None = None,
) -> KernelRun:
    """Create a durable run row before execution starts."""

    discovery_run = None
    if discovery_run_id:
        discovery_run = DiscoveryRun.objects.filter(run_id=discovery_run_id).first()
    payload_hash = stable_hash({
        'kernel_id': contract.kernel_id,
        'request_payload': request_payload,
        'budget': budget or {},
        'metadata': metadata or {},
        'discovery_run_id': discovery_run_id or '',
        'created_at': timezone.now().isoformat(),
    })
    return KernelRun.objects.create(
        run_id=f'kernel-{payload_hash[:20]}',
        kernel_id=contract.kernel_id,
        epistemic_job=contract.epistemic_job,
        inference_family=contract.inference_family,
        status=KernelRun.Status.RUNNING,
        request_payload=dict(request_payload or {}),
        budget=dict(budget or {}),
        metadata=dict(metadata or {}),
        writeback_policy=contract.writeback_policy,
        canonical_graph_mutation=False,
        discovery_run=discovery_run,
    )


def append_kernel_receipt(
    kernel_run: KernelRun,
    *,
    receipt_type: str,
    payload: dict[str, Any],
    status: str | None = None,
    validator_id: str = '',
    writeback_proposals: list[dict[str, Any]] | None = None,
) -> KernelResultReceipt:
    """Append or update one receipt without exposing raw private content."""

    payload_copy = dict(payload or {})
    payload_hash = stable_hash(payload_copy)
    receipt_hash = str(payload_copy.get('receipt_hash') or stable_hash({
        'kernel_run': kernel_run.run_id,
        'receipt_type': receipt_type,
        'payload_hash': payload_hash,
    }))
    receipt, _ = KernelResultReceipt.objects.update_or_create(
        kernel_run=kernel_run,
        receipt_hash=receipt_hash,
        defaults={
            'receipt_type': str(receipt_type or 'kernel_result'),
            'status': status or _result_status(payload_copy),
            'validator_id': validator_id,
            'payload': payload_copy,
            'payload_hash': payload_hash,
            'writeback_proposals': list(writeback_proposals or payload_copy.get('writeback_proposals') or []),
            'private_content_excluded': True,
        },
    )
    return receipt


def finish_kernel_run(
    kernel_run: KernelRun,
    *,
    result_payload: dict[str, Any],
    started_at,
    status: str = KernelRun.Status.SUCCEEDED,
    error_payload: dict[str, Any] | None = None,
) -> KernelRun:
    """Mark a run complete and bind the result hash."""

    kernel_run.status = status
    kernel_run.result_payload = dict(result_payload or {})
    kernel_run.error_payload = dict(error_payload or {})
    kernel_run.duration_ms = _now_ms(started_at)
    kernel_run.receipt_hash = stable_hash({
        'run_id': kernel_run.run_id,
        'status': status,
        'result_payload': kernel_run.result_payload,
        'error_payload': kernel_run.error_payload,
    })
    kernel_run.canonical_graph_mutation = False
    kernel_run.save(update_fields=[
        'status',
        'result_payload',
        'error_payload',
        'duration_ms',
        'receipt_hash',
        'canonical_graph_mutation',
        'updated_at',
    ])
    return kernel_run


def render_kernel_run(kernel_run: KernelRun) -> dict[str, Any]:
    payload = kernel_run.to_contract_snapshot()
    payload['result_receipts'] = [
        receipt.to_contract_receipt()
        for receipt in kernel_run.result_receipts.order_by('created_at')
    ]
    payload['append_only'] = True
    return payload
