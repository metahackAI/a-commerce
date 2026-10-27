"""Shopper agent: a message in, UI blocks out (docs/plan.md §5.5, design/storefront).

The model decides *which* component to show and *which* products go in it, through the harness tools below.
The harness fills every price, stock count and spec from the commerce tools, so a model can never put a
made-up price on the screen. Orders are never placed here; the shopper taps Place order.
"""

from __future__ import annotations

import json

from ..db import Store
from ..tools.base import REGISTRY, ToolError, call
from ..tools.pricing import price_for
from .models import Model
from .operator import compact
from .search import search

MAX_STEPS = 6

VIEW_WHY = {
    "compare_table": "Shown as a table because you compare specs. Say “show photos” to change it.",
    "look_gallery": "Photos first. Say “compare them” for a table.",
    "product_list": "A short list with details. Say “bigger photos” to change it.",
}

SHOW_PRODUCTS = {"type": "function", "function": {
    "name": "show_products",
    "description": "Show products to the shopper in one of the store's components. Prices and specs are filled in by the store.",
    "parameters": {"type": "object", "required": ["view", "product_ids", "chips", "reply"], "properties": {
        "view": {"type": "string", "enum": list(VIEW_WHY),
                 "description": "compare_table for shoppers who compare specs or ask which is best on a spec; look_gallery for browsing by look or when asked for photos; product_list for a first answer with details."},
        "product_ids": {"type": "array", "items": {"type": "string"}, "description": "2 to 4 product ids, in the order to show."},
        "sort": {"type": "string", "enum": ["weight", "price", "given"], "description": "Order for compare_table."},
        "chips": {"type": "array", "description": "What you understood, as short removable chips. Mark ones taken from the profile.",
                  "items": {"type": "object", "properties": {"label": {"type": "string"}, "source": {"type": "string", "enum": ["conversation", "profile"]}}}},
        "reply": {"type": "string", "description": "One or two sentences to the shopper."},
    }},
}}

SHOW_CART = {"type": "function", "function": {
    "name": "show_cart",
    "description": "Show the cart ready to order, after add_to_cart. Only the shopper can place the order.",
    "parameters": {"type": "object", "required": ["reply"], "properties": {"reply": {"type": "string", "description": "One sentence."}}},
}}

SHOW_RETURN = {"type": "function", "function": {
    "name": "show_return_options",
    "description": "Show the shopper what they can do about an order they want to return: a new item or a refund. "
                   "The shopper picks and sends it themselves.",
    "parameters": {"type": "object", "required": ["order_id", "reason", "reply"], "properties": {
        "order_id": {"type": "string", "description": "Order number from get_my_orders, e.g. #20402."},
        "reason": {"type": "string", "enum": ["defect", "damaged", "too_small", "too_large", "color", "changed_mind"]},
        "chosen": {"type": "string", "enum": ["replacement", "refund"], "description": "Preselect what the shopper asked for."},
        "reply": {"type": "string", "description": "One or two sentences to the shopper."},
    }},
}}

SYSTEM = """You are the shopping assistant for Northwind, talking with {who}. Store time is {now}.
What the store knows about this shopper: {profile}
- Find products with the tools, then show them with show_products. Never write prices yourself.
- When the shopper tells you something lasting about themselves, save it with update_my_preferences. Never save things about other people.
- You can add to the cart; only the shopper places orders.
{history}"""


def run_shopper(store: Store, message: str, model: Model, context: dict, history: list[dict] | None = None) -> dict:
    profile = call(store, "get_my_profile", {}, actor="shopper-agent", context=context)
    tools = [REGISTRY[t.name].definition() for t in search(message, scope="shopper")]
    for always in ("get_my_profile", "search_products", "compare_products", "update_my_preferences", "add_to_cart", "get_my_orders"):
        if always not in [t["function"]["name"] for t in tools]:
            tools.append(REGISTRY[always].definition())
    tools += [SHOW_PRODUCTS, SHOW_CART, SHOW_RETURN]
    hist = "Earlier in this conversation: " + json.dumps(history[-6:]) if history else ""
    messages = [
        {"role": "system", "content": SYSTEM.format(who=profile.get("name") or "a first-time visitor",
                                                   now=store.now().strftime("%a %b %-d %Y %H:%M"),
                                                   profile=json.dumps(profile), history=hist)},
        {"role": "user", "content": message},
    ]
    blocks, text, tool_calls, duration = [], "", 0, 0
    for _ in range(MAX_STEPS):
        reply = model.complete(messages, tools)
        duration += reply.duration_ms
        if not reply.tool_calls:
            text = text or reply.text
            break
        messages.append({"role": "assistant", "content": reply.text or None, "tool_calls": [
            {"id": c.id, "type": "function", "function": {"name": c.name, "arguments": json.dumps(c.arguments)}}
            for c in reply.tool_calls]})
        for c in reply.tool_calls:
            tool_calls += 1
            try:
                if c.name == "show_products":
                    blocks += products_blocks(store, context, c.arguments)
                    text = c.arguments.get("reply", text)
                    out = {"ok": True}
                elif c.name == "show_return_options":
                    opts = call(store, "get_return_options", {"order_id": c.arguments.get("order_id"), "reason": c.arguments.get("reason")},
                                actor="harness", context=context)
                    if "error" in opts:
                        raise ToolError(opts["error"]["code"], opts["error"]["message"], opts["error"]["next_tool"])
                    opts["chosen"] = c.arguments.get("chosen")
                    blocks.append({"type": "ui", "ui": "return_options", "schema": 1, "block_id": f"ret_{opts['order']['id']}",
                                   "data": opts, "actions": [{"intent": "send_return_request", "label": "Send request", "by": "shopper"}]})
                    text = c.arguments.get("reply", text)
                    out = {"ok": True}
                elif c.name == "show_cart":
                    blocks.append(cart_block(store, context))
                    text = c.arguments.get("reply", text)
                    out = {"ok": True}
                elif c.name in REGISTRY and REGISTRY[c.name].scope in ("shopper", "both"):
                    out = call(store, c.name, c.arguments, actor=f"shopper-agent:{model.name}", context=context)
                    if c.name == "update_my_preferences":
                        blocks += [{"type": "ui", "ui": "memory_note", "schema": 1, "block_id": m["memory_id"], "data": m,
                                    "actions": [{"intent": "undo_memory", "label": "Undo", "by": "shopper", "args": {"memory_id": m["memory_id"]}}]}
                                   for m in out.get("memories", [])]
                else:
                    out = {"error": {"code": "unknown_tool", "message": f"No shopper tool {c.name}.", "next_tool": None}}
            except ToolError as e:
                out = e.payload()
            messages.append({"role": "tool", "tool_call_id": c.id, "content": compact(out)})
    step = {"type": "agent_step", "model": model.name, "tool_calls": tool_calls, "duration_ms": duration,
            "read_only": False, "text": text}
    return {"events": [step] + blocks}


def products_blocks(store: Store, context: dict, args: dict) -> list[dict]:
    ids = args.get("product_ids", [])[:4]
    view = args.get("view", "product_list")
    chips = {"type": "ui", "ui": "understood_chips", "schema": 1, "block_id": "chips",
             "data": {"chips": args.get("chips", [])}, "actions": [{"intent": "remove_chip", "label": "×", "by": "shopper"}]}
    if view == "compare_table":
        cmp = call(store, "compare_products", {"product_ids": ids, "sort": args.get("sort", "weight")}, actor="harness", context=context)
        if "error" in cmp:
            raise ToolError(cmp["error"]["code"], cmp["error"]["message"], cmp["error"]["next_tool"])
        data = cmp
    else:
        data = {"items": [gallery_item(store, context, pid) for pid in ids]}
    actions = [{"intent": "add_to_cart", "label": "Add", "by": "shopper"}]
    return [chips, {"type": "ui", "ui": view, "schema": 1, "block_id": view, "why": VIEW_WHY[view], "data": data, "actions": actions}]


def gallery_item(store: Store, context: dict, pid: str) -> dict:
    p = call(store, "get_product", {"product_id": pid}, actor="harness", context=context)
    if "error" in p:
        raise ToolError(p["error"]["code"], p["error"]["message"], "search_products")
    row = store.one("SELECT look FROM products WHERE id = ?", (pid,))
    facts = [f"{p['weight_g']} g" if p.get("weight_g") else None, (p.get("packed") or "").lower() or None,
             "light rain" if p.get("light_rain") else None, p.get("fabric"), f"{p['colors']} colors"]
    return {"product_id": pid, "name": p["name"], "color": p["color"], "price": price_for(store, p, context.get("tier")),
            "facts": [f for f in facts if f], "look": row["look"] if row else None}


def cart_block(store: Store, context: dict) -> dict:
    order = call(store, "prepare_order", {}, actor="harness", context=context)
    if "error" in order:
        raise ToolError(order["error"]["code"], order["error"]["message"], order["error"]["next_tool"])
    return {"type": "ui", "ui": "cart", "schema": 1, "block_id": order["order_preview_id"], "data": order,
            "actions": [{"intent": "place_order", "label": f"Place order · ${order['total']:.2f}", "by": "shopper",
                         "args": {"order_preview_id": order["order_preview_id"]}}]}
