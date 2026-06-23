"""Pure-Python Datalog-style derivation engine for first-pass consequence rules."""

from .engine import DatalogEngine
from .facts import build_fact_pack_from_models, build_fact_pack_from_records

__all__ = [
    'DatalogEngine',
    'build_fact_pack_from_models',
    'build_fact_pack_from_records',
]

