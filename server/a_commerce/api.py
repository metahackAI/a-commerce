"""HTTP API. Tool calls for agents, person-only endpoints for buttons, and demo controls (docs/plan.md §3.6)."""

from __future__ import annotations

import json
import os
from typing import Any

from fastapi import FastAPI, Header, HTTPException
from fastapi.middleware.cors import CORSMiddleware
from pydantic import BaseModel

from . import changesets, tasks
from .harness import operator_models, run_operator, run_shopper, shopper_model
from .db import Store
from .seed import fresh_store
from .tools.base import REGISTRY, ToolError, call
from .tools.gift import place_gift_order
from .tools.shopper import place_order, send_return_request, undo_memory

app = FastAPI(title="A-Commerce")
app.add_middleware(CORSMiddleware, allow_origins=["*"], allow_methods=["*"], allow_headers=["*"])

SHOPPERS = {
    "jamie": {"shopper": True, "member_id": "mem_jamie", "tier": "vip"},
    "alex": {"shopper": True, "member_id": "mem_alex", "tier": "member"},
    "mina": {"shopper": True, "member_id": None, "tier": None, "guest_id": "mina"},
    "outside": {"shopper": True, "member_id": None, "tier": None, "guest_id": "outside-agent"},  # MCP guests
}

_state: dict[str, Store] = {}


def store() -> Store:
    if "store" not in _state:
        _state["store"] = fresh_store(os.environ.get("A_COMMERCE_DB", ":memory:"))
    return _state["store"]


def shopper(name: str | None) -> dict:
    if name not in SHOPPERS:
        raise HTTPException(400, f"Unknown shopper {name!r}. Use one of {', '.join(SHOPPERS)}.")
    return SHOPPERS[name]


def guard(fn, *args, **kwargs):
    try:
        return fn(*args, **kwargs)
    except ToolError as e:
        raise HTTPException(409, e.payload()["error"]) from None


@app.get("/health")
def health() -> dict:
    return {"ok": True, "clock": store().now().isoformat(timespec="minutes"), "tools": len(REGISTRY)}


# -- tools (what agents call) -------------------------------------------------

@app.get("/tools")
def list_tools(scope: str | None = None) -> list[dict]:
    tools = [t for t in REGISTRY.values() if scope is None or t.scope in (scope, "both")]
    return [t.definition() for t in tools]


class ToolCall(BaseModel):
    args: dict[str, Any] = {}


@app.post("/tools/{name}")
def run_tool(name: str, body: ToolCall, x_shopper: str | None = Header(None)) -> Any:
    context = shopper(x_shopper) if x_shopper else None
    return call(store(), name, body.args, actor="agent", context=context)


# -- operator chat --------------------------------------------------------------

class OperatorRequest(BaseModel):
    request: str
    change_set_id: int | None = None


@app.post("/bo/chat")
def operator_chat(body: OperatorRequest) -> dict:
    """One operator message in, agent steps and UI blocks out. Edits to a change set pass its id."""
    worker, fallback = operator_models(store())
    return run_operator(store(), body.request, worker, change_set_id=body.change_set_id, fallback=fallback)


# -- storefront chat ---------------------------------------------------------------

class ShopperMessage(BaseModel):
    message: str


@app.post("/fo/chat")
def shopper_chat(body: ShopperMessage, x_shopper: str = Header(...)) -> dict:
    """One shopper message in, UI blocks out. The conversation is kept per shopper until /demo/reset."""
    ctx = shopper(x_shopper)
    history = _state.setdefault("fo_history", {}).setdefault(x_shopper, [])
    result = run_shopper(store(), body.message, shopper_model(store()), ctx, history)
    shown = next((e["data"] for e in result["events"] if e.get("ui") in ("compare_table", "look_gallery", "product_list")), None)
    ids = [p["product_id"] for p in (shown.get("products") or shown.get("items") or [])] if shown else []
    history.append({"user": body.message, "shown": ids})
    return result


@app.get("/fo/profile")
def shopper_profile(x_shopper: str = Header(...)) -> dict:
    return call(store(), "get_my_profile", {}, actor="storefront", context=shopper(x_shopper))


@app.get("/bo/tasks")
def bo_tasks() -> dict:
    """The back office home: open change sets, work the agent noticed, and what is done."""
    return tasks.task_list(store())


# -- change sets ----------------------------------------------------------------

class NewChangeSet(BaseModel):
    title: str
    request: str
    preview_ids: list[str]


class Revision(BaseModel):
    request: str
    preview_ids: list[str]


class Approval(BaseModel):
    version: int


@app.post("/change-sets")
def create_change_set(body: NewChangeSet) -> dict:
    cs = guard(changesets.create, store(), body.title, body.request, body.preview_ids)
    return changesets.view(store(), cs)


@app.post("/change-sets/{cs_id}/revise")
def revise_change_set(cs_id: int, body: Revision) -> dict:
    guard(changesets.revise, store(), cs_id, body.request, body.preview_ids)
    return changesets.view(store(), cs_id)


@app.get("/change-sets/{cs_id}")
def get_change_set(cs_id: int) -> dict:
    return guard(changesets.view, store(), cs_id)


@app.get("/change-sets")
def list_change_sets() -> list[dict]:
    return [changesets.view(store(), r["id"]) for r in store().all("SELECT id FROM change_sets ORDER BY id DESC")]


@app.post("/change-sets/{cs_id}/approve")
def approve_change_set(cs_id: int, body: Approval) -> dict:
    """Person-only. Called by the Approve button."""
    return guard(changesets.approve, store(), cs_id, body.version)


@app.post("/change-sets/{cs_id}/undo")
def undo_change_set(cs_id: int) -> dict:
    """Person-only. Called by the Undo button."""
    return guard(changesets.undo, store(), cs_id)


# -- shopper buttons --------------------------------------------------------------

class PlaceOrder(BaseModel):
    order_preview_id: str


@app.post("/orders")
def post_order(body: PlaceOrder, x_shopper: str = Header(...)) -> dict:
    ctx = shopper(x_shopper)
    return guard(place_order, store(), body.order_preview_id, ctx["member_id"], ctx)


class GiftOrder(BaseModel):
    product_id: str
    size: str
    wrap: bool = True
    message: str = ""
    recipient: str
    address: str


@app.post("/gift-orders")
def post_gift_order(body: GiftOrder, x_shopper: str = Header(...)) -> dict:
    """Person-only. The shopper's Place gift order button; the address is used for this order only."""
    ctx = shopper(x_shopper)
    return guard(place_gift_order, store(), ctx["member_id"], ctx, body.product_id, body.size, body.wrap, body.message,
                 body.recipient, body.address)


class ReturnRequest(BaseModel):
    order_id: str
    option: str = "refund"
    reason: str = "defect"
    detail: str = ""
    photos: int = 0


@app.post("/returns")
def post_return(body: ReturnRequest, x_shopper: str = Header(...)) -> dict:
    """Person-only. Called by the shopper's Send button; the request joins the back office review."""
    ctx = shopper(x_shopper)
    return guard(send_return_request, store(), ctx["member_id"], body.order_id, body.option, body.reason, body.detail, body.photos)


@app.post("/memories/{memory_id}/undo")
def post_undo_memory(memory_id: str, x_shopper: str = Header(...)) -> dict:
    ctx = shopper(x_shopper)
    owner = ctx["member_id"] or f"guest:{ctx['guest_id']}"
    guard(undo_memory, store(), memory_id, owner)
    return {"ok": True}


# -- log and demo controls ---------------------------------------------------------

@app.get("/actions")
def actions(limit: int = 50) -> list[dict]:
    rows = store().all("SELECT * FROM action_log ORDER BY id DESC LIMIT ?", (limit,))
    return [{**dict(r), "detail": json.loads(r["detail"])} for r in rows]


class Clock(BaseModel):
    at: str


@app.post("/demo/clock")
def set_clock(body: Clock) -> dict:
    try:
        store().set_clock(body.at)
    except ValueError:
        raise HTTPException(400, f"Not a date and time: {body.at}") from None
    return {"clock": body.at}


@app.post("/demo/reset")
def reset() -> dict:
    _state.pop("store", None)
    _state.pop("fo_history", None)
    return health()
