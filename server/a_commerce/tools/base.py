"""Tool registry and the read / preview / commit envelopes from docs/plan.md §3.1."""

from __future__ import annotations

import inspect
import json
from dataclasses import dataclass, field
from typing import Any, Callable

from pydantic import BaseModel, ValidationError

from ..db import Store

REF_PREFIX = "ref:"


class ToolError(Exception):
    def __init__(self, code: str, message: str, next_tool: str | None = None):
        super().__init__(message)
        self.code, self.message, self.next_tool = code, message, next_tool

    def payload(self) -> dict:
        return {"error": {"code": self.code, "message": self.message, "next_tool": self.next_tool}}


@dataclass
class Check:
    rule: str
    passed: bool
    title: str
    detail: str
    fix_tool: str | None = None


@dataclass
class Plan:
    """What a write would do. Built the same way for the preview and again right before commit."""

    changes: list[dict]
    checks: list[Check]
    approval_kind: str | None  # a key in policy approval_required_for, or None
    approval_reason: str = ""
    summary: str = ""


@dataclass
class Tool:
    name: str
    kind: str  # read | instant (small, undoable shopper actions) | write (preview, then approval)
    description: str
    input: type[BaseModel]
    consumes: list[str]
    produces: list[str]
    run: Callable[..., Any] | None = None           # reads
    plan: Callable[..., Plan] | None = None         # writes
    apply: Callable[..., str] | None = None         # writes; returns the new or changed id
    scope: str = "operator"                          # operator | shopper | both
    extra: dict = field(default_factory=dict)

    def definition(self) -> dict:
        """The tool as a model sees it (OpenAI-compatible function schema)."""
        schema = self.input.model_json_schema()
        if self.kind == "write":
            schema.setdefault("properties", {})
            schema["properties"]["preview"] = {
                "type": "boolean", "default": True,
                "description": "Always true for agents: returns what would change and the rule checks without changing anything.",
            }
        return {
            "type": "function",
            "function": {"name": self.name, "description": self.description, "parameters": schema},
            "x-kind": self.kind, "x-consumes": self.consumes, "x-produces": self.produces, "x-scope": self.scope,
        }


REGISTRY: dict[str, Tool] = {}


def register(tool: Tool) -> Tool:
    if tool.name in REGISTRY:
        raise ValueError(f"duplicate tool {tool.name}")
    REGISTRY[tool.name] = tool
    return tool


def read_tool(name: str, description: str, input: type[BaseModel], consumes=(), produces=(), scope="operator"):
    def wrap(fn):
        register(Tool(name, "read", description, input, list(consumes), list(produces), run=fn, scope=scope))
        return fn
    return wrap


def instant_tool(name: str, description: str, input: type[BaseModel], consumes=(), produces=(), scope="shopper"):
    def wrap(fn):
        register(Tool(name, "instant", description, input, list(consumes), list(produces), run=fn, scope=scope))
        return fn
    return wrap


def write_tool(name: str, description: str, input: type[BaseModel], consumes=(), produces=(), scope="operator"):
    """Decorates a class with static `plan(store, args)` and `apply(store, args)`."""
    def wrap(cls):
        register(Tool(name, "write", description, input, list(consumes), list(produces),
                      plan=cls.plan, apply=cls.apply, scope=scope))
        return cls
    return wrap


# -- calling --------------------------------------------------------------

def call(store: Store, name: str, args: dict | None = None, *, actor: str = "agent", context: dict | None = None) -> dict:
    """Run one tool and return its envelope. Errors come back as {"error": ...}, never as exceptions."""
    args = dict(args or {})
    tool = REGISTRY.get(name)
    if tool is None:
        return ToolError("unknown_tool", f"There is no tool named {name}.", "search_tools").payload()
    try:
        if tool.kind in ("read", "instant"):
            parsed = _parse(tool, args)
            takes_context = "context" in inspect.signature(tool.run).parameters
            result = tool.run(store, parsed, **({"context": context} if takes_context else {}))
            store.log(actor, name, {"args": args})
            return result
        preview_id = args.pop("preview_id", None)
        preview = args.pop("preview", True)
        if preview_id:
            # Only approve() and person-only endpoints commit. An agent can never apply its own preview.
            raise ToolError("person_only", "Applying a preview needs a person's approval. Put it in a change set.", None)
        if preview is False:
            raise ToolError("preview_required", "Writes run in two steps: call with preview=true, then commit the preview_id after approval.", name)
        return make_preview(store, tool, args, actor=actor, context=context)
    except ToolError as e:
        store.log(actor, name, {"args": args, "error": e.code})
        return e.payload()


def _parse(tool: Tool, args: dict) -> BaseModel:
    try:
        return tool.input.model_validate(args)
    except ValidationError as e:
        first = e.errors()[0]
        where = ".".join(str(p) for p in first["loc"])
        raise ToolError("invalid_input", f"{tool.name}: {where} {first['msg']}.", tool.name) from None


def make_preview(store: Store, tool: Tool, args: dict, *, actor: str, context: dict | None) -> dict:
    parsed = _parse(tool, args)
    plan = tool.plan(store, parsed)
    needs_approval = plan.approval_kind is not None and plan.approval_kind in store.policy_value("approval_required_for")
    preview_id = store.new_id("pv")
    result = {
        "preview_id": preview_id,
        "tool": tool.name,
        "summary": plan.summary,
        "changes": plan.changes,
        "checks": [c.__dict__ for c in plan.checks],
        "needs_approval": needs_approval,
        "approval_reason": plan.approval_reason if needs_approval else "",
    }
    store.execute(
        "INSERT INTO previews (id, tool, args, result, status, created_at) VALUES (?, ?, ?, ?, 'open', ?)",
        (preview_id, tool.name, json.dumps(args), json.dumps(result), store.now().isoformat(timespec="minutes")),
    )
    store.log(actor, f"{tool.name}:preview", {"args": args, "preview_id": preview_id})
    return result


def commit(store: Store, preview_id: str, *, actor: str) -> dict:
    """Apply a preview. Checks run again against the current state, so a stale preview cannot slip through."""
    row = store.one("SELECT * FROM previews WHERE id = ?", (preview_id,))
    if row is None:
        raise ToolError("unknown_preview", f"No preview {preview_id}.", None)
    if row["status"] != "open":
        raise ToolError("preview_used", f"Preview {preview_id} is already {row['status']}.", None)
    tool = REGISTRY[row["tool"]]
    args = resolve_refs(store, json.loads(row["args"]))
    parsed = _parse(tool, args)
    plan = tool.plan(store, parsed)
    failed = [c for c in plan.checks if not c.passed]
    if failed:
        raise ToolError("check_failed", f"{failed[0].title}: {failed[0].detail}", failed[0].fix_tool)
    committed_id = tool.apply(store, parsed)
    store.execute("UPDATE previews SET status = 'committed', committed_id = ? WHERE id = ?", (committed_id, preview_id))
    store.log(actor, tool.name, {"preview_id": preview_id, "id": committed_id})
    return {"ok": True, "id": committed_id, "preview_id": preview_id}


def resolve_refs(store: Store, value: Any) -> Any:
    """Replace "ref:pv_N" with the id that preview produced when it was committed."""
    if isinstance(value, str) and value.startswith(REF_PREFIX):
        pv = value[len(REF_PREFIX):]
        row = store.one("SELECT committed_id FROM previews WHERE id = ?", (pv,))
        if row is None or row["committed_id"] is None:
            raise ToolError("ref_not_committed", f"{pv} has not been committed yet, so {value} has no id.", None)
        return row["committed_id"]
    if isinstance(value, dict):
        return {k: resolve_refs(store, v) for k, v in value.items()}
    if isinstance(value, list):
        return [resolve_refs(store, v) for v in value]
    return value


def pending_ref(store: Store, value: str) -> dict | None:
    """During a preview, describe a "ref:pv_N" that is not committed yet."""
    if not value.startswith(REF_PREFIX):
        return None
    row = store.one("SELECT tool, args, status, committed_id FROM previews WHERE id = ?", (value[len(REF_PREFIX):],))
    if row is None:
        raise ToolError("unknown_preview", f"{value} does not point at a preview.", None)
    return {"tool": row["tool"], "args": json.loads(row["args"]), "committed_id": row["committed_id"]}
