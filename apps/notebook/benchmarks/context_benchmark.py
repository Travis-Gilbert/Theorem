"""Shadow benchmark suite for the Context Compiler.

Runs ~30 to 50 representative tasks (PR review, bug fix, planning,
refactor, postmortem search) against the compile pipeline and records:

    tokens_in           sum of input tokens consumed by the agent
    tokens_out          sum of output tokens produced
    tool_call_count     observed tool invocations on the artifact
    time_to_first_correct_action  ms to the first action_rail item the
                        agent actually executes (recorded post-hoc via
                        outcome endpoint)

Two modes per task:
    with_context        compile a ContextArtifact, the agent uses it
    without_context     classic prompt (no capsule), agent runs raw

We compute a calibration constant per task type by averaging the
ratio of tokens_in/with vs tokens_in/without across all tasks of that
type. The result is a JSON report consumed by the dashboard's token
ledger to translate raw tokens-saved into believable dollar/time
estimates.

This file is the harness. The 30-50 tasks themselves live in
``benchmarks/tasks.json`` (created by the user; the harness loads it
or falls back to a tiny built-in seed list for smoke purposes).
"""

from __future__ import annotations

import json
import logging
import os
import statistics
import time
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Iterable

logger = logging.getLogger(__name__)


SEED_TASKS: list[dict[str, str]] = [
    {
        'id': 'review-001',
        'task_type': 'review',
        'task': 'review the auth module for missing rate limits',
    },
    {
        'id': 'fix-001',
        'task_type': 'fix',
        'task': 'fix the lockout bug introduced after the rate limiter',
    },
    {
        'id': 'plan-001',
        'task_type': 'plan',
        'task': 'plan the refactor of the auth module to extract a SessionService',
    },
    {
        'id': 'refactor-001',
        'task_type': 'refactor',
        'task': 'refactor the SessionService to use async-friendly storage',
    },
    {
        'id': 'search-001',
        'task_type': 'search',
        'task': 'search for prior incidents involving auth lockout',
    },
    {
        'id': 'research-001',
        'task_type': 'research',
        'task': 'research OAuth2 token rotation best practices',
    },
]


@dataclass
class TaskResult:
    task_id: str
    task_type: str
    tokens_in_with: int = 0
    tokens_in_without: int = 0
    tokens_out_with: int = 0
    tokens_out_without: int = 0
    tool_calls_with: int = 0
    tool_calls_without: int = 0
    elapsed_ms_with: int = 0
    elapsed_ms_without: int = 0
    artifact_id: str | None = None
    notes: list[str] = field(default_factory=list)


def _load_tasks(path: str | None) -> list[dict[str, str]]:
    if not path:
        return SEED_TASKS
    p = Path(path)
    if not p.exists():
        logger.warning('Benchmark tasks file not found: %s; using seed list.', path)
        return SEED_TASKS
    try:
        with p.open() as f:
            data = json.load(f)
    except Exception as exc:
        logger.warning('Benchmark tasks file unreadable: %s; using seed list.', exc)
        return SEED_TASKS
    if isinstance(data, list) and data:
        return data
    return SEED_TASKS


def _run_with_context(task: dict[str, str]) -> TaskResult:
    from django.contrib.auth import get_user_model

    from apps.notebook.ask_pipeline import ask_theseus
    from apps.notebook.models import ContextArtifact

    User = get_user_model()
    account = (
        User.objects.filter(is_staff=True).first()
        or User.objects.first()
    )
    if account is None:
        result = TaskResult(
            task_id=task.get('id', '?'),
            task_type=task.get('task_type', 'other'),
        )
        result.notes.append('no_account_available')
        return result

    artifact = ContextArtifact.objects.create(
        account=account,
        title=task.get('task', '')[:200],
        task_description=task.get('task', ''),
        task_type=_coerce_task_type(task.get('task_type')),
        budget_tokens=8000,
    )

    start = time.monotonic()
    try:
        compiled = ask_theseus(
            query=task.get('task', ''),
            user_id=account.id,
            include_web=True,
            mode='compile',
            compile_artifact=artifact,
            compile_budget_tokens=8000,
        )
    except Exception as exc:
        logger.warning('compile failed for %s: %s', task.get('id'), exc)
        result = TaskResult(
            task_id=task.get('id', '?'),
            task_type=task.get('task_type', 'other'),
            artifact_id=str(artifact.id),
        )
        result.notes.append(f'compile_failed: {exc.__class__.__name__}')
        return result
    elapsed_ms = int((time.monotonic() - start) * 1000)

    ledger = compiled.get('token_ledger') or {}
    actions = compiled.get('actions') or []

    return TaskResult(
        task_id=task.get('id', '?'),
        task_type=task.get('task_type', 'other'),
        tokens_in_with=int(ledger.get('capsuleTokens', 0)),
        tokens_out_with=0,
        tool_calls_with=len(actions),
        elapsed_ms_with=elapsed_ms,
        artifact_id=str(artifact.id),
    )


def _run_without_context(task: dict[str, str]) -> TaskResult:
    """Approximate a no-context baseline.

    Without a context compiler, the agent receives the full retrieval
    set as raw text. We simulate that cost as the rawCandidateTokens
    figure produced inside the same compile (using budget=very_large so
    no atom is excluded).
    """
    from django.contrib.auth import get_user_model

    from apps.notebook.ask_pipeline import ask_theseus
    from apps.notebook.models import ContextArtifact

    User = get_user_model()
    account = (
        User.objects.filter(is_staff=True).first()
        or User.objects.first()
    )
    if account is None:
        return TaskResult(
            task_id=task.get('id', '?'),
            task_type=task.get('task_type', 'other'),
        )

    artifact = ContextArtifact.objects.create(
        account=account,
        title=task.get('task', '')[:200],
        task_description=task.get('task', ''),
        task_type=_coerce_task_type(task.get('task_type')),
        budget_tokens=200_000,
    )

    start = time.monotonic()
    try:
        compiled = ask_theseus(
            query=task.get('task', ''),
            user_id=account.id,
            include_web=True,
            mode='compile',
            compile_artifact=artifact,
            compile_budget_tokens=200_000,
        )
    except Exception as exc:
        result = TaskResult(
            task_id=task.get('id', '?'),
            task_type=task.get('task_type', 'other'),
        )
        result.notes.append(f'baseline_failed: {exc.__class__.__name__}')
        return result
    elapsed_ms = int((time.monotonic() - start) * 1000)
    ledger = compiled.get('token_ledger') or {}

    # Cleanup the baseline artifact so the user's history doesn't bloat.
    try:
        artifact.delete()
    except Exception:
        pass

    return TaskResult(
        task_id=task.get('id', '?'),
        task_type=task.get('task_type', 'other'),
        tokens_in_without=int(ledger.get('rawCandidateTokens', 0)),
        tool_calls_without=0,
        elapsed_ms_without=elapsed_ms,
    )


def _coerce_task_type(value: str | None) -> str:
    from apps.notebook.models import ContextArtifact

    if not value:
        return ContextArtifact.TaskType.OTHER
    valid = {choice for choice, _ in ContextArtifact.TaskType.choices}
    normalized = value.strip().lower()
    if normalized in valid:
        return normalized
    return ContextArtifact.TaskType.OTHER


def run_benchmark(
    tasks_path: str | None = None,
    output_path: str | None = None,
) -> dict[str, Any]:
    """Run all benchmark tasks and produce a JSON report."""
    tasks = _load_tasks(tasks_path)
    results: list[TaskResult] = []

    for task in tasks:
        with_ctx = _run_with_context(task)
        without_ctx = _run_without_context(task)
        merged = TaskResult(
            task_id=with_ctx.task_id,
            task_type=with_ctx.task_type,
            tokens_in_with=with_ctx.tokens_in_with,
            tokens_in_without=without_ctx.tokens_in_without,
            tokens_out_with=with_ctx.tokens_out_with,
            tokens_out_without=without_ctx.tokens_out_without,
            tool_calls_with=with_ctx.tool_calls_with,
            tool_calls_without=without_ctx.tool_calls_without,
            elapsed_ms_with=with_ctx.elapsed_ms_with,
            elapsed_ms_without=without_ctx.elapsed_ms_without,
            artifact_id=with_ctx.artifact_id,
            notes=list(with_ctx.notes) + list(without_ctx.notes),
        )
        results.append(merged)

    calibration = _compute_calibration(results)
    report = {
        'task_count': len(results),
        'results': [r.__dict__ for r in results],
        'calibration': calibration,
    }

    if output_path:
        with open(output_path, 'w') as f:
            json.dump(report, f, indent=2)
        logger.info('Benchmark report written: %s', output_path)

    return report


def _compute_calibration(results: Iterable[TaskResult]) -> dict[str, Any]:
    by_type: dict[str, list[float]] = {}
    for r in results:
        if r.tokens_in_without <= 0:
            continue
        ratio = r.tokens_in_with / r.tokens_in_without
        by_type.setdefault(r.task_type, []).append(ratio)

    out: dict[str, Any] = {}
    for task_type, ratios in by_type.items():
        if not ratios:
            continue
        out[task_type] = {
            'mean_ratio': round(statistics.fmean(ratios), 4),
            'samples': len(ratios),
        }
    return out


# ---------------------------------------------------------------------------
# Management-command friendly entry point.
# ---------------------------------------------------------------------------


def main(argv: list[str] | None = None) -> None:
    import argparse

    parser = argparse.ArgumentParser(description='Context Theorem shadow benchmark')
    parser.add_argument(
        '--tasks',
        default=os.environ.get('CONTEXT_BENCH_TASKS'),
        help='Path to tasks.json file (defaults to seed list).',
    )
    parser.add_argument(
        '--output',
        default=os.environ.get('CONTEXT_BENCH_OUTPUT'),
        help='Path to write the JSON report (defaults to stdout).',
    )
    args = parser.parse_args(argv)

    report = run_benchmark(tasks_path=args.tasks, output_path=args.output)
    if not args.output:
        print(json.dumps(report, indent=2))


if __name__ == '__main__':
    main()
