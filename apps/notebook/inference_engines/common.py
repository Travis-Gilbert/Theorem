"""Shared helpers for additive inference-engine receipts."""

from __future__ import annotations

import hashlib
import json
from typing import Any


def stable_json(value: Any) -> str:
    return json.dumps(value, sort_keys=True, separators=(',', ':'), default=str)


def stable_hash(value: Any) -> str:
    return hashlib.sha256(stable_json(value).encode('utf-8')).hexdigest()


def clamp01(value: float) -> float:
    return max(0.0, min(1.0, float(value)))

