"""Shopper tools (docs/plan.md §3.4). They act as the signed-in shopper given in `context`.

Context: {"shopper": True, "member_id": "mem_jamie" | None, "tier": "vip" | None, "guest_id": "..."}
"""

from __future__ import annotations

import json
from datetime import datetime, timedelta
from typing import Literal

from pydantic import BaseModel, Field

from ..db import Store, money
from .base import ToolError, instant_tool, read_tool
from .operator import product_row
from .pricing import best_sale, price_for

SHIPPING_FEE = 6.95
DELIVERY_DAYS = 3


def owner(context: dict | None) -> str:
    if not context or not context.get("shopper"):
        raise ToolError("not_a_shopper", "Shopper tools need a signed-in shopper or a guest session.", None)
    return context.get("member_id") or f"guest:{context.get('guest_id', 'anon')}"


# -- compare ----------------------------------------------------------------

Attribute = Literal["packed", "weight", "warmth", "light_rain", "price"]


class CompareIn(BaseModel):
    product_ids: list[str] = Field(min_length=2, max_length=4, description="Products to put side by side.",
                                   examples=[["prd_windbreaker", "prd_field_jacket", "prd_overshirt"]])
    attributes: list[Attribute] = Field(["packed", "weight", "warmth", "light_rain", "price"],
                                        description="Rows to show.", examples=[["packed", "weight", "price"]])
    sort: Literal["weight", "price", "given"] = Field("given", description="Column order.", examples=["weight"])


@read_tool("compare_products", "Put 2 to 4 products side by side: packed size, weight, warmth, rain, and the shopper's own price.",
           CompareIn, consumes=["product_id[]"], produces=["comparison"], scope="shopper")
def compare_products(store: Store, args: CompareIn, context: dict | None = None) -> dict:
    tier = (context or {}).get("tier")
    items = []
    for pid in args.product_ids:
        row = store.one("SELECT * FROM products WHERE id = ?", (pid,))
        if row is None:
            raise ToolError("unknown_product", f"No product {pid}.", "search_products")
        p = product_row(store, row)
        p["your_price"] = price_for(store, p, tier)
        items.append(p)
    if args.sort == "weight":
        items.sort(key=lambda p: p["weight_g"] or 0)
    elif args.sort == "price":
        items.sort(key=lambda p: p["your_price"]["now"])
    labels = {"packed": "Packed", "weight": "Weight, M", "warmth": "Warmth", "light_rain": "Light rain", "price": "Your price"}
    rows = []
    for a in args.attributes:
        if a == "packed":
            values = [p["packed"] for p in items]
        elif a == "weight":
            values = [f"{p['weight_g']} g" if p["weight_g"] else "—" for p in items]
        elif a == "warmth":
            values = [p["warmth"] for p in items]
        elif a == "light_rain":
            values = ["Yes" if p["light_rain"] else "No" for p in items]
        else:
            values = [p["your_price"] for p in items]
        rows.append({"label": labels[a], "values": values})
    products = [{"product_id": p["id"], "name": p["name"], "sub": f"{p['color']} · {p['fabric']}"} for p in items]
    return {"products": products, "rows": rows, "sort": args.sort}


# -- profile and memory -------------------------------------------------------

class NoInput(BaseModel):
    pass


@read_tool("get_my_profile", "What the store knows about this shopper: tier, remembered preferences, saved address and card (last 4 only).",
           NoInput, produces=["profile"], scope="shopper")
def get_my_profile(store: Store, args: NoInput, context: dict | None = None) -> dict:
    who = owner(context)
    prefs = [dict(r) for r in store.all(
        "SELECT id, key, value, source FROM preferences WHERE member_id = ? AND status = 'active' ORDER BY created_at", (who,))]
    member = store.one("SELECT * FROM members WHERE id = ?", (who,))
    profile = json.loads(member["profile"]) if member else {}
    return {"member_id": who, "name": member["name"] if member else None, "tier": member["tier_id"] if member else None,
            "preferences": prefs, "home": profile.get("home"), "card_last4": profile.get("card_last4"),
            "orders": store.one("SELECT COUNT(*) AS n FROM orders WHERE member_id = ?", (who,))["n"]}


class Preference(BaseModel):
    key: str = Field(description="What kind of preference.", examples=["travel"])
    value: str = Field(description="The preference in the shopper's terms.", examples=["Weight and packed size matter when traveling"])


class UpdatePrefsIn(BaseModel):
    add: list[Preference] = Field([], description="Preferences the shopper just told you about themselves.")
    remove: list[str] = Field([], description="Preference ids to forget.", examples=[["pref_2"]])
    about_someone_else: bool = Field(False, description="True when the request is about another person (a gift); nothing is saved.",
                                     examples=[False])


@instant_tool("update_my_preferences",
              "Remember or forget something the shopper said about themselves. Each memory is shown to them with Undo.",
              UpdatePrefsIn, produces=["memory_id"], scope="shopper")
def update_my_preferences(store: Store, args: UpdatePrefsIn, context: dict | None = None) -> dict:
    who = owner(context)
    if args.about_someone_else:
        return {"memories": [], "note": "Not saved: this is about someone else."}
    out = []
    for p in args.add:
        mid = store.new_id("pref")
        store.execute("INSERT INTO preferences (id, member_id, key, value, source, created_at) VALUES (?, ?, ?, ?, 'conversation', ?)",
                      (mid, who, p.key, p.value, store.now().isoformat(timespec="minutes")))
        out.append({"memory_id": mid, "text": f"Remembered: {p.value[0].lower()}{p.value[1:]}"})
    for mid in args.remove:
        store.execute("UPDATE preferences SET status = 'removed' WHERE id = ? AND member_id = ?", (mid, who))
    return {"memories": out, "removed": args.remove}


def undo_memory(store: Store, memory_id: str, member_id: str) -> None:
    """Person-only: the shopper taps Undo."""
    cur = store.execute("UPDATE preferences SET status = 'removed' WHERE id = ? AND member_id = ?", (memory_id, member_id))
    if cur.rowcount == 0:
        raise ToolError("unknown_memory", f"No memory {memory_id} for this shopper.", None)


# -- cart -------------------------------------------------------------------

class AddToCartIn(BaseModel):
    product_id: str = Field(description="Product id.", examples=["prd_windbreaker"])
    size: str = Field(description="Size.", examples=["M"])
    qty: int = Field(1, ge=1, le=10, description="How many.", examples=[1])


def _cart_id(store: Store, who: str) -> str:
    row = store.one("SELECT id FROM carts WHERE member_id = ?", (who,))
    if row:
        return row["id"]
    cid = store.new_id("cart")
    store.execute("INSERT INTO carts VALUES (?, ?)", (cid, who))
    return cid


@instant_tool("add_to_cart", "Put a product in the shopper's cart. The shopper still reviews and places the order.",
              AddToCartIn, consumes=["product_id"], produces=["cart_id"], scope="shopper")
def add_to_cart(store: Store, args: AddToCartIn, context: dict | None = None) -> dict:
    who = owner(context)
    v = store.one("SELECT stock FROM variants WHERE product_id = ? AND size = ?", (args.product_id, args.size))
    if v is None:
        raise ToolError("unknown_variant", f"{args.product_id} has no size {args.size}.", "get_product")
    if v["stock"] < args.qty:
        raise ToolError("out_of_stock", f"Only {v['stock']} left in {args.size}.", "search_products")
    cid = _cart_id(store, who)
    store.execute("INSERT INTO cart_lines VALUES (?, ?, ?, ?) ON CONFLICT(cart_id, product_id, size) DO UPDATE SET qty = qty + excluded.qty",
                  (cid, args.product_id, args.size, args.qty))
    return cart(store, who, context)


def cart(store: Store, who: str, context: dict | None) -> dict:
    tier = (context or {}).get("tier")
    cid = _cart_id(store, who)
    lines, subtotal, benefit, labels, promo_ids = [], 0.0, 0.0, [], []
    for r in store.all("SELECT * FROM cart_lines WHERE cart_id = ? ORDER BY rowid", (cid,)):
        p = store.one("SELECT * FROM products WHERE id = ?", (r["product_id"],))
        sale = best_sale(store, p["id"], tier)
        line_total = money(p["price"] * r["qty"])
        subtotal += line_total
        if sale:
            benefit += money((p["price"] - sale["sale_price"]) * r["qty"])
            labels.append(sale["promotion"])
            promo_ids.append(sale["promotion_id"])
        lines.append({"product_id": p["id"], "name": p["name"], "variant": f"{p['color']} · {r['size']}", "size": r["size"],
                      "qty": r["qty"], "price": line_total})
    subtotal, benefit = money(subtotal), money(benefit)
    coupon = None
    if promo_ids:
        c = store.one(f"SELECT code FROM coupons WHERE status = 'live' AND auto_apply = 1 AND promotion_id IN ({','.join('?' * len(promo_ids))})",
                      promo_ids)
        coupon = c["code"] if c else None
    free_ship = store.one("SELECT status FROM promotions WHERE id = 'promo_free_ship'")
    shipping = 0.0 if not lines or (free_ship and free_ship["status"] == "live"
                                    and subtotal >= store.policy_value("free_shipping_min_subtotal")) else SHIPPING_FEE
    return {
        "cart_id": cid, "lines": lines, "subtotal": subtotal,
        "benefit": {"label": labels[0], "amount": -benefit, "coupon": coupon} if benefit else None,
        "shipping": shipping, "total": money(subtotal - benefit + shipping),
    }


class GetCartIn(BaseModel):
    pass


@read_tool("get_cart", "Read the shopper's cart with the member benefit, shipping, and total.", GetCartIn,
           consumes=["cart_id"], produces=["cart"], scope="shopper")
def get_cart(store: Store, args: GetCartIn, context: dict | None = None) -> dict:
    return cart(store, owner(context), context)


class PrepareOrderIn(BaseModel):
    ship_to: Literal["home"] = Field("home", description="Saved address to ship to.", examples=["home"])


@read_tool("prepare_order", "Get the cart ready to order: address, arrival date, card, total. Only the shopper can place it.",
           PrepareOrderIn, consumes=["cart_id"], produces=["order_preview_id"], scope="shopper")
def prepare_order(store: Store, args: PrepareOrderIn, context: dict | None = None) -> dict:
    who = owner(context)
    c = cart(store, who, context)
    if not c["lines"]:
        raise ToolError("empty_cart", "The cart is empty.", "add_to_cart")
    member = store.one("SELECT profile FROM members WHERE id = ?", (who,))
    profile = json.loads(member["profile"]) if member else {}
    arrives = store.now() + timedelta(days=DELIVERY_DAYS)
    pv = store.new_id("order_pv")
    order = {**c, "order_preview_id": pv, "ship_to": f"Home · {profile.get('home', '')}",
             "arrives": f"{arrives:%a, %b} {arrives.day}", "pay_with": f"Card ending {profile.get('card_last4', '')}",
             "notice": "Northwind can prepare your order, but only you can place it."}
    store.execute("INSERT INTO previews (id, tool, args, result, status, created_at) VALUES (?, 'prepare_order', ?, ?, 'open', ?)",
                  (pv, json.dumps({"member_id": who}), json.dumps(order), store.now().isoformat(timespec="minutes")))
    return order


def place_order(store: Store, order_preview_id: str, member_id: str, context: dict) -> dict:
    """Person-only: the shopper taps Place order. Prices are recomputed, not trusted from the preview."""
    pv = store.one("SELECT * FROM previews WHERE id = ? AND tool = 'prepare_order'", (order_preview_id,))
    if pv is None or json.loads(pv["args"])["member_id"] != member_id:
        raise ToolError("unknown_order_preview", "That order is not ready to place.", "prepare_order")
    if pv["status"] != "open":
        raise ToolError("already_placed", "This order was already placed.", None)
    c = cart(store, member_id, context)
    shown = json.loads(pv["result"])
    if c["total"] != shown["total"]:
        raise ToolError("price_changed", f"The total changed from ${shown['total']:.2f} to ${c['total']:.2f}. Review it again.", "prepare_order")
    oid = "#" + store.new_id("order").split("_")[1]
    now = store.now().isoformat(timespec="minutes")
    store.execute("INSERT INTO orders VALUES (?, ?, 'placed', ?, NULL, ?, ?, ?, ?, ?, ?, ?, 'home')",
                  (oid, member_id, now, json.dumps(c["lines"]), c["subtotal"],
                   c["benefit"]["amount"] if c["benefit"] else 0.0, c["shipping"], c["total"],
                   c["benefit"]["coupon"] if c["benefit"] else None, shown["pay_with"]))
    for line in c["lines"]:
        store.execute("UPDATE variants SET stock = stock - ? WHERE product_id = ? AND size = ?",
                      (line["qty"], line["product_id"], line["size"]))
    store.execute("DELETE FROM cart_lines WHERE cart_id = ?", (c["cart_id"],))
    store.execute("UPDATE previews SET status = 'committed', committed_id = ? WHERE id = ?", (oid, order_preview_id))
    store.log(member_id, "place_order", {"order_id": oid, "total": c["total"]})
    return {"order_id": oid, "total": c["total"], "coupon": c["benefit"]["coupon"] if c["benefit"] else None}


# -- orders and returns (P1) ---------------------------------------------------------

@read_tool("get_my_orders", "The shopper's recent orders with items and delivery dates.", NoInput,
           produces=["order_id[]"], scope="shopper")
def get_my_orders(store: Store, args: NoInput, context: dict | None = None) -> dict:
    who = owner(context)
    rows = store.all("SELECT id, status, placed_at, delivered_at, lines, total FROM orders WHERE member_id = ? ORDER BY placed_at DESC LIMIT 10",
                     (who,))
    items = [{**dict(r), "lines": json.loads(r["lines"])} for r in rows]
    return {"items": items, "total": len(items)}


class ReturnOptionsIn(BaseModel):
    order_id: str = Field(description="Order number.", examples=["#20402"])
    reason: Literal["defect", "damaged", "too_small", "too_large", "color", "changed_mind"] = Field(
        description="Why it is coming back.", examples=["defect"])


@read_tool("get_return_options", "What the shopper can choose for a return: a new item or a refund, with timing, cost, and who confirms.",
           ReturnOptionsIn, consumes=["order_id"], produces=["return_options"], scope="shopper")
def get_return_options(store: Store, args: ReturnOptionsIn, context: dict | None = None) -> dict:
    who = owner(context)
    order = store.one("SELECT * FROM orders WHERE id = ? AND member_id = ?", (args.order_id, who))
    if order is None:
        raise ToolError("unknown_order", f"No order {args.order_id} for this shopper.", "get_my_orders")
    line = json.loads(order["lines"])[0]
    product = store.one("SELECT * FROM products WHERE id = ?", (line.get("product_id"),))
    days = (store.now().date() - datetime.fromisoformat(order["delivered_at"]).date()).days
    window = store.policy_value("return_window_days")
    if days > window:
        raise ToolError("outside_window", f"Order {args.order_id} was delivered {days} days ago; returns close after {window} days.", None)
    stock = store.one("SELECT stock FROM variants WHERE product_id = ? AND size = ?", (line.get("product_id"), line.get("size")))
    arrives = store.now() + timedelta(days=4)
    limit = store.policy_value("refund_without_person_max")
    free = args.reason in ("defect", "damaged")
    member = store.one("SELECT profile FROM members WHERE id = ?", (who,))
    card = json.loads(member["profile"]).get("card_last4") if member else None
    options = []
    if stock and stock["stock"] > 0 and args.reason in ("defect", "damaged", "too_small", "too_large"):
        options.append({"kind": "replacement", "title": "New pair, same size" if product and product["category_id"] == "footwear" else "New one, same size",
                        "ships": f"Today, {stock['stock']} in stock", "arrives": f"{arrives:%a %b} {arrives.day}", "you_pay": 0.0})
    options.append({"kind": "refund", "title": "Full refund", "amount": order["total"], "to": f"card ending {card}",
                    "confirmed_by": "A person at Northwind, usually within 1 business day" if order["total"] > limit else "Automatic",
                    "money_back": "3–5 days after it's confirmed"})
    return {"order": {"id": order["id"], "delivered": f"{datetime.fromisoformat(order['delivered_at']):%a %b} {datetime.fromisoformat(order['delivered_at']).day}",
                      "item": product["name"] if product else line.get("item"), "variant": f"Size {line.get('size')} · {product['color'] if product else ''}",
                      "amount": order["total"]},
            "reason": args.reason, "options": options,
            "send_back": {"cost": "free, defect" if free else "paid by you", "pickup": "Mon Nov 9, 9:00–12:00"},
            "note": f"Refunds over ${limit:.0f} are confirmed by a person at Northwind." if order["total"] > limit else ""}


REASON_TEXT = {"defect": "Defect", "damaged": "Arrived damaged", "too_small": "Too small", "too_large": "Too large",
               "color": "Color differs from photo", "changed_mind": "Changed mind"}


def send_return_request(store: Store, member_id: str, order_id: str, option: str, reason: str, detail: str, photos: int) -> dict:
    """Person-only: the shopper taps Send. The request lands in the back office review as it is."""
    order = store.one("SELECT * FROM orders WHERE id = ? AND member_id = ?", (order_id, member_id))
    if order is None:
        raise ToolError("unknown_order", f"No order {order_id} for this shopper.", None)
    if store.one("SELECT 1 FROM return_requests WHERE order_id = ? AND status = 'waiting'", (order_id,)):
        raise ToolError("already_sent", f"A return for {order_id} is already waiting.", None)
    line = json.loads(order["lines"])[0]
    product = store.one("SELECT name FROM products WHERE id = ?", (line.get("product_id"),))
    rid = store.new_id("ret")
    text = REASON_TEXT.get(reason, reason) + (f": {detail}" if detail else "")
    store.execute("INSERT INTO return_requests VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 'waiting')",
                  (rid, order_id, member_id, product["name"] if product else line.get("item", ""), order["total"], reason, text,
                   photos, order["delivered_at"], store.now().isoformat(timespec="minutes"), option))
    store.log(member_id, "send_return_request", {"return_request_id": rid, "order_id": order_id, "option": option})
    return {"return_request_id": rid, "status": "waiting",
            "next": "A person at Northwind confirms refunds over $200, usually within one business day."}
