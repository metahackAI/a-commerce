"""Operator agent: request → tool search → tool calls (reads, then previews) → change set (docs/design.md §5).

The agent can only preview writes. It bundles previews into a change set with the harness tool
`propose_change_set`; a person approves it through the API. Approval, run and read-back are not the agent's job.
"""

from __future__ import annotations

import json
from dataclasses import dataclass, field

from .. import changesets
from ..db import Store
from ..tools.base import REGISTRY, call
from .models import Model, Reply
from .search import search

MAX_STEPS = 8
RESULT_CHARS = 6000

PROPOSE = {
    "type": "function",
    "function": {
        "name": "propose_change_set",
        "description": "Bundle the previews you made into one change set for a person to review. Call it once, last.",
        "parameters": {"type": "object", "required": ["title", "preview_ids", "reply"], "properties": {
            "title": {"type": "string", "description": "Short title, e.g. 'VIP weekend sale: summer outerwear'."},
            "preview_ids": {"type": "array", "items": {"type": "string"}, "description": "preview_id of every write preview to include."},
            "reply": {"type": "string", "description": "What you tell the operator: what you prepared, and any check that needs their call."},
        }},
    },
}

SYSTEM = """You are the operator agent for {store}. Store time is {now}.
You change the store only through tools. Rules:
- Read first, then preview writes. Every write tool returns a preview with rule checks; nothing changes yet.
- You cannot apply a preview. Bundle all previews with propose_change_set; a person approves it.
- To link a write to another write's result before it exists, pass "ref:<preview_id>" (for example a coupon's promotion_id).
- Follow the policies. If a check fails, say which and why in your reply; propose anyway so the person can decide.
- Prefer category-wide arguments (category, percent_off, min_stock, overrides) over listing products one by one.
- Reply in two or three plain sentences.
{revision}"""


@dataclass
class Step:
    model: str
    tool_calls: int = 0
    duration_ms: int = 0
    calls: list[dict] = field(default_factory=list)


def run_operator(store: Store, request: str, model: Model, *, change_set_id: int | None = None,
                 fallback: Model | None = None) -> dict:
    """Run one operator request. Returns {"events": [...], "change_set_id": int | None}."""
    result = _attempt(store, request, model, change_set_id)
    if result["ok"] or fallback is None:
        return result
    # The worker could not finish; the planner takes the request over (design.md §5).
    retry = _attempt(store, request, fallback, change_set_id)
    retry["events"].insert(0, {"type": "handoff", "from": model.name, "to": fallback.name, "reason": result["error"]})
    return retry


def _attempt(store: Store, request: str, model: Model, change_set_id: int | None) -> dict:
    revision = ""
    if change_set_id is not None:
        view = changesets.view(store, change_set_id)
        prior = [{"tool": p["tool"], "args": p["args"]} for p in _preview_args(store, change_set_id)]
        revision = (f"You are revising change set #{change_set_id} v{view['version']} ({view['title']}). "
                    f"Earlier requests: {json.dumps(view['request'])}. Its previews: {json.dumps(prior)}. "
                    "Make new previews for the whole change set (all of them, changed or not) and propose them.")
    tools = _tools_for(request)
    messages = [
        {"role": "system", "content": SYSTEM.format(store="Northwind Apparel", now=store.now().strftime("%a %b %-d %Y %H:%M"),
                                                   revision=revision)},
        {"role": "user", "content": request},
    ]
    step = Step(model.name)
    proposal, errors = None, 0
    for _ in range(MAX_STEPS):
        reply: Reply = model.complete(messages, tools)
        step.duration_ms += reply.duration_ms
        if not reply.tool_calls:
            text = reply.text
            break
        messages.append({"role": "assistant", "content": reply.text or None, "tool_calls": [
            {"id": c.id, "type": "function", "function": {"name": c.name, "arguments": json.dumps(c.arguments)}}
            for c in reply.tool_calls]})
        for c in reply.tool_calls:
            step.tool_calls += 1
            if c.name == "propose_change_set":
                proposal = c.arguments
                out = {"ok": True}
            elif c.name not in REGISTRY:
                out = {"error": {"code": "unknown_tool", "message": f"No tool {c.name}.", "next_tool": None}}
            else:
                out = call(store, c.name, c.arguments, actor=f"agent:{model.name}")
            if "error" in out:
                errors += 1
            step.calls.append({"tool": c.name, "args": c.arguments, "error": out.get("error")})
            messages.append({"role": "tool", "tool_call_id": c.id, "content": compact(out)})
    else:
        text = ""

    events = []
    if proposal is None:
        error = "No change set proposed" + (f" ({errors} tool errors)" if errors else "")
        events.append(_step_event(step, text or "I couldn't put this together. " + error, read_only=True))
        return {"ok": False, "error": error, "events": events, "change_set_id": change_set_id}

    ids = [p for p in proposal.get("preview_ids", []) if _is_open_preview(store, p)]
    if not ids:
        return {"ok": False, "error": "Proposed change set had no valid previews",
                "events": [_step_event(step, "I couldn't build any previews for this.", read_only=True)],
                "change_set_id": change_set_id}
    if change_set_id is None:
        change_set_id = changesets.create(store, proposal.get("title") or request[:60], request, ids)
    else:
        changesets.revise(store, change_set_id, request, ids)
    view = changesets.view(store, change_set_id)
    events.append(_step_event(step, proposal.get("reply") or text, read_only=False))
    events.append({"type": "ui", "ui": "change_set", "schema": 1, "block_id": f"cs_{change_set_id}_v{view['version']}",
                   "title": view["title"], "data": view,
                   "actions": [{"intent": "approve_change_set", "label": "Approve and run", "by": "operator",
                                "args": {"change_set_id": change_set_id, "version": view["version"]}}]})
    return {"ok": True, "events": events, "change_set_id": change_set_id}


def compact(out: dict, limit: int = RESULT_CHARS) -> str:
    """Keep tool results small but always valid JSON: long lists keep their first rows plus the total."""
    text = json.dumps(out, default=str)
    if len(text) <= limit:
        return text
    out = dict(out)
    for key in ("items", "changes", "rows"):
        if isinstance(out.get(key), list) and len(out[key]) > 5:
            out[f"{key}_total"] = len(out[key])
            out[key] = out[key][:5]
            out["note"] = f"Showing 5 of {out[f'{key}_total']} {key}. Narrow the request or use fields to see more."
    return json.dumps(out, default=str)


def _tools_for(request: str) -> list[dict]:
    picked = search(request, scope="operator")
    names = [t.name for t in picked]
    for always in ("get_policy", "search_products"):
        if always not in names:
            names.append(always)
    return [REGISTRY[n].definition() for n in names] + [PROPOSE]


def _step_event(step: Step, text: str, *, read_only: bool) -> dict:
    return {"type": "agent_step", "model": step.model, "tool_calls": step.tool_calls, "duration_ms": step.duration_ms,
            "read_only": read_only, "text": text, "calls": step.calls}


def _is_open_preview(store: Store, preview_id: str) -> bool:
    row = store.one("SELECT status FROM previews WHERE id = ?", (preview_id,))
    return row is not None and row["status"] == "open"


def _preview_args(store: Store, cs_id: int) -> list[dict]:
    cs = store.one("SELECT preview_ids FROM change_sets WHERE id = ?", (cs_id,))
    out = []
    for pid in json.loads(cs["preview_ids"]):
        row = store.one("SELECT tool, args FROM previews WHERE id = ?", (pid,))
        out.append({"tool": row["tool"], "args": json.loads(row["args"])})
    return out

