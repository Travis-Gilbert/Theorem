"""Append-only JSONL benchmark ledger.

Dev/eval infrastructure, not a graph entity (no Django model, no migration): the
ledger is a file the cost-test arms append to. The first line of every ledger is
the locked pre-registration header (spec section 4: "Pre-registered predictions
recorded in the ledger header before any run").
"""

from __future__ import annotations

import json
from pathlib import Path
from typing import Iterator

from .preregistration import PRE_REGISTRATION
from .records import BenchmarkRecord


class BenchmarkLedger:
    """Open-or-create a JSONL ledger and append operation records."""

    def __init__(self, path: str | Path) -> None:
        self._path = Path(path)
        self._path.parent.mkdir(parents=True, exist_ok=True)
        if not self._path.exists() or self._path.stat().st_size == 0:
            self._write_line({'record_type': 'pre_registration', **PRE_REGISTRATION})

    def record(self, record: BenchmarkRecord) -> None:
        self._write_line({'record_type': 'operation', **record.to_dict()})

    def records(self) -> list[BenchmarkRecord]:
        out: list[BenchmarkRecord] = []
        for data in self._iter_payloads():
            if data.get('record_type') != 'operation':
                continue
            data.pop('record_type', None)
            out.append(BenchmarkRecord.from_dict(data))
        return out

    def pre_registration(self) -> dict:
        for data in self._iter_payloads():
            if data.get('record_type') == 'pre_registration':
                data.pop('record_type', None)
                return data
        return {}

    @property
    def path(self) -> Path:
        return self._path

    def _write_line(self, payload: dict) -> None:
        with self._path.open('a', encoding='utf-8') as handle:
            handle.write(json.dumps(payload, sort_keys=True) + '\n')

    def _iter_payloads(self) -> Iterator[dict]:
        if not self._path.exists():
            return
        with self._path.open('r', encoding='utf-8') as handle:
            for line in handle:
                stripped = line.strip()
                if stripped:
                    yield json.loads(stripped)
