"""Evolution and quality-diversity engine adapters."""

from .engine import EvolutionEngine
from .native import NativeEvolutionEngine, native_evolution_can_handle

__all__ = ['EvolutionEngine', 'NativeEvolutionEngine', 'native_evolution_can_handle']
