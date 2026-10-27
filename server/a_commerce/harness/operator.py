"""Operator agent: request → tool search → tool calls (reads, then previews) → change set (docs/design.md §5).

The agent can only preview writes. It bundles previews into a change set with the harness tool
`propose_change_set`; a person approves it through the API. Approval, run and read-back are not the agent's job.
"""

from __future__ import annotations

import json
import re
from dataclasses import dataclass, field

from .. import changesets, facade
from ..db import Store
from ..tools.base import REGISTRY, call, loose_json
from .grounding import check_reply, check_sales_reply, sales_summary, summary
from .models import Model, ModelError, Reply, ToolCallReq
from .search import search

MAX_STEPS = 10
MAX_NUDGES = 2
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
{tool_tip}- Reply in two or three plain sentences.
{revision}"""


@dataclass
class Step:
    model: str
    tool_calls: int = 0
    duration_ms: int = 0
    calls: list[dict] = field(default_factory=list)


def run_operator(store: Store, request: str, model: Model, *, change_set_id: int | None = None,
                 fallback: Model | None = None, toolset: str = "a2api") -> dict:
    """Run one operator request. Returns {"events": [...], "change_set_id": int | None}.

    toolset="facade" gives the model the screen-shaped API instead (docs/design.md §7); everything else is the same."""
    result = _attempt(store, request, model, change_set_id, toolset)
    if result["ok"] or fallback is None:
        return result
    # The worker could not finish; the planner takes the request over (design.md §5).
    retry = _attempt(store, request, fallback, change_set_id, toolset)
    retry["events"].insert(0, {"type": "handoff", "from": model.name, "to": fallback.name, "reason": result["error"]})
    return retry


def _attempt(store: Store, request: str, model: Model, change_set_id: int | None, toolset: str = "a2api") -> dict:
    revision = ""
    if change_set_id is not None:
        view = changesets.view(store, change_set_id)
        prior = _preview_args(store, change_set_id)
        asked = []
        for p in prior:
            if p["asked"] not in asked:
                asked.append(p["asked"])
        revision = (f"You are revising change set #{change_set_id} v{view['version']} ({view['title']}). "
                    f"Earlier requests: {json.dumps(view['request'])}. It was made with these calls: {json.dumps(asked)}. "
                    f"Its checks: {json.dumps([{'check': c['title'], 'passed': c['passed'], 'detail': c['detail'], **({'fix': c['fix']} if c.get('fix') else {})} for c in view['checks']])}. "
                    f"Its previews: {json.dumps([{'tool': p['tool'], 'args': p['args']} for p in prior])}. "
                    "Call the same tool again passing only what changes; the harness keeps every other argument. Then propose."
                    + "".join(f"\nFailing check: {c['title']}. {c['fix']}" for c in view["checks"] if not c["passed"] and c.get("fix")))
    tools = facade.definitions() + [PROPOSE] if toolset == "facade" else _tools_for(request)
    # In a revision a call to the same tool is a patch: unchanged arguments come from the earlier call.
    patch_base = {a["tool"]: a["args"] for a in (asked if change_set_id is not None else [])}
    messages = [
        {"role": "system", "content": SYSTEM.format(store="Northwind Apparel", now=store.now().strftime("%a %b %-d %Y %H:%M"),
                                                   revision=revision, tool_tip="" if toolset == "facade" else
                                                   "- Prefer category-wide arguments (category, percent_off, min_stock, overrides) over listing products one by one.\n")},
        {"role": "user", "content": request},
    ]
    step = Step(model.name)
    proposal, errors, nudges, last_error, previews, insight = None, 0, 0, None, [], None
    for _ in range(MAX_STEPS):
        try:
            reply: Reply = model.complete(messages, tools)
        except ModelError as e:
            events = [_step_event(step, f"The model failed: {e}", read_only=True)]
            return {"ok": False, "error": str(e), "events": events, "change_set_id": change_set_id}
        step.duration_ms += reply.duration_ms
        if not reply.tool_calls:
            reply.tool_calls = calls_in_text(reply.text, {t["function"]["name"] for t in tools})
        if not reply.tool_calls:
            text = reply.text
            if proposal is not None or insight is not None or nudges >= MAX_NUDGES:
                break
            # Small models often stop to talk after an error or before proposing. Point at the next step once or twice.
            nudges += 1
            messages.append({"role": "assistant", "content": reply.text or ""})
            messages.append({"role": "user", "content": _nudge(last_error, previews)})
            continue
        messages.append({"role": "assistant", "content": reply.text or None, "tool_calls": [
            {"id": c.id, "type": "function", "function": {"name": c.name, "arguments": json.dumps(c.arguments)}}
            for c in reply.tool_calls]})
        for c in reply.tool_calls:
            step.tool_calls += 1
            if c.name in patch_base:
                c.arguments = {**patch_base[c.name], **{k: v for k, v in c.arguments.items() if v not in (None, "", "null")}}
            if c.name == "propose_change_set":
                proposal = decode_strings(c.arguments)
                out = {"ok": True}
            elif toolset == "facade":
                out = facade.call(store, c.name, c.arguments)
            elif c.name not in REGISTRY:
                out = {"error": {"code": "unknown_tool", "message": f"No tool {c.name}.", "next_tool": None}}
            else:
                out = call(store, c.name, c.arguments, actor=f"agent:{model.name}")
            if toolset == "facade" and out.get("resultCd", "0000") != "0000":
                errors += 1
                last_error = (c.name, {"message": out.get("resultMsg", ""), "next_tool": None})
            elif toolset == "facade":
                previews += [x for k in ("prmtNoList", "cpnNo", "bnrNo") for x in ([out[k]] if isinstance(out.get(k), str) else out.get(k, []))]
            if "error" in out:
                errors += 1
                last_error = (c.name, out["error"])
            elif out.get("read_only") and "causes" in out:
                insight = out
            elif "preview_id" in out or "preview_ids" in out:
                previews += [out["preview_id"]] if "preview_id" in out else out["preview_ids"]
                last_error = None
            err = out.get("error") or ({"code": out["resultCd"], "message": out.get("resultMsg", "")}
                                       if toolset == "facade" and out.get("resultCd", "0000") != "0000" else None)
            step.calls.append({"tool": c.name, "args": c.arguments, "error": err})
            messages.append({"role": "tool", "tool_call_id": c.id, "content": compact(out)})
    else:
        text = ""

    events = []
    if proposal is None and insight is not None:
        # A question, not a change: answer with a read-only block. Numbers in the reply must be in the data.
        issues = check_sales_reply(text, insight)
        step_event = _step_event(step, sales_summary(insight) if issues or not text else text, read_only=True)
        if issues:
            step_event["corrected"] = {"said": text, "issues": issues}
        events += [step_event, {"type": "ui", "ui": "sales_explainer", "schema": 1, "block_id": f"sales_{insight['day']}",
                                "title": f"Sales on {insight['day']}", "data": insight,
                                "actions": [{"intent": "new_request", "label": s, "by": "operator", "args": {"request": s}}
                                            for s in insight["next_steps"]]}]
        return {"ok": True, "events": events, "change_set_id": None}
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
    said = proposal.get("reply") or text
    corrections = check_reply(said, view)
    event = _step_event(step, summary(view) if corrections or not said else said, read_only=False)
    if corrections:
        event["corrected"] = {"said": said, "issues": corrections}
    events.append(event)
    events.append({"type": "ui", "ui": view.get("kind", "change_set"), "schema": 1, "block_id": f"cs_{change_set_id}_v{view['version']}",
                   "title": view["title"], "data": view,
                   "actions": [{"intent": "approve_change_set", "label": view.get("approve_label", "Approve and run"), "by": "operator",
                                "args": {"change_set_id": change_set_id, "version": view["version"]}}]})
    return {"ok": True, "events": events, "change_set_id": change_set_id}


def calls_in_text(text: str, names: set[str]) -> list[ToolCallReq]:
    """Some open models (Llama 3.x among them) write a tool call as JSON in the message instead of tool_calls."""
    calls = []
    for m in re.finditer(r"\{.*\}", text or "", re.S):
        try:
            obj = json.loads(m.group(0))
        except ValueError:
            continue
        name = obj.get("name") or obj.get("function")
        args = obj.get("parameters") or obj.get("arguments") or {}
        if name in names and isinstance(args, dict):
            calls.append(ToolCallReq(f"text_{len(calls)}", name, args))
    return calls


def decode_strings(args: dict) -> dict:
    """Harness tools get the same leniency as commerce tools: '["pv_1"]' and "['pv_1']" are lists."""
    out = {k: loose_json(v) for k, v in args.items()}
    if isinstance(out.get("preview_ids"), str):
        out["preview_ids"] = re.findall(r"pv_\d+", out["preview_ids"])
    return out


def _nudge(last_error: tuple[str, dict] | None, previews: list[str]) -> str:
    if last_error:
        name, err = last_error
        return (f"{name} failed: {err['message']} Call {err.get('next_tool') or name} again with fixed arguments. "
                "Nothing is applied until a person approves, so keep going.")
    if previews:
        return f"Call propose_change_set now with preview_ids {json.dumps(previews)} and a short reply for the operator."
    return "Use the tools: preview each change (sale prices, coupon, banner as asked), then call propose_change_set."


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
    for always in ("plan_sale", "plan_return_review", "explain_sales_change", "get_policy", "search_products"):
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
        row = store.one("SELECT tool, args, result FROM previews WHERE id = ?", (pid,))
        asked = json.loads(row["result"]).get("asked") or {"tool": row["tool"], "args": json.loads(row["args"])}
        out.append({"tool": row["tool"], "args": json.loads(row["args"]), "asked": asked})
    return out

