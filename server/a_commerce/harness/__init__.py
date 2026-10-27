"""The commerce harness: tool search, model calls, change sets (docs/design.md §5)."""

from ..db import Store
from .models import Model, from_env
from .operator import run_operator
from .scripted import ScriptedOperator, ScriptedShopper
from .shopper import run_shopper


def operator_models(store: Store) -> tuple[Model, Model | None]:
    """Worker and fallback. Without a configured worker the scripted stand-in runs, and says so."""
    worker, planner = from_env("WORKER"), from_env("PLANNER")
    if worker is None and planner is None:
        return ScriptedOperator(store), None
    if worker is None:
        return planner, None
    return worker, planner


def shopper_model(store: Store) -> Model:
    return from_env("WORKER") or from_env("PLANNER") or ScriptedShopper(store)


__all__ = ["run_operator", "operator_models", "run_shopper", "shopper_model", "ScriptedOperator", "ScriptedShopper"]
