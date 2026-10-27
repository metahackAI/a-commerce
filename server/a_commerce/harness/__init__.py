"""The commerce harness: tool search, model calls, change sets (docs/design.md §5)."""

from ..db import Store
from .models import Model, from_env
from .operator import run_operator
from .scripted import ScriptedOperator


def operator_models(store: Store) -> tuple[Model, Model | None]:
    """Worker and fallback. Without a configured worker the scripted stand-in runs, and says so."""
    worker, planner = from_env("WORKER"), from_env("PLANNER")
    if worker is None and planner is None:
        return ScriptedOperator(store), None
    if worker is None:
        return planner, None
    return worker, planner


__all__ = ["run_operator", "operator_models", "ScriptedOperator"]
