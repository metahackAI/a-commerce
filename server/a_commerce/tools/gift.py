"""Gift picker (design/storefront/fo-gift): picks for someone else, delivery by a date, gift receipt without prices.

Nothing about the recipient is saved to the shopper's profile. The address is typed by the shopper at order time and
used for that order only. Only the shopper places the order.
"""

from __future__ import annotations

import json
from datetime import date, timedelta

from pydantic import BaseModel, Field

from ..db import Store, money
from .base import ToolError, read_tool
from .pricing import price_for
from .shopper import DELIVERY_DAYS, owner

WRAP = 5.00


class GiftIn(BaseModel):
    product_ids: list[str] = Field(min_length=1, max_length=4, description="Gift ideas to show, best first.",
                                   examples=[["prd_windbreaker", "prd_fleece", "prd_merino_tee"]])
    chosen: str | None = Field(None, description="The one the shopper picked, if any.", examples=["prd_windbreaker"])
    size: str | None = Field(None, description="Size for the recipient, if known.", examples=["L"])
    budget: float | None = Field(None, description="Most the shopper wants to spend.", examples=[100])
    by: str | None = Field(None, description="Must arrive by this date, YYYY-MM-DD.", examples=["2026-11-14"])
    wrap: bool = Field(True, description="Paper wrap and card.", examples=[True])


@read_tool("plan_gift", "Prepare gift options: prices for this shopper, whether it arrives in time, wrap, gift receipt, and the total. "
           "Saves nothing about the recipient.", GiftIn, consumes=["product_id[]"], produces=["gift_plan"], scope="shopper")
def plan_gift(store: Store, args: GiftIn, context: dict | None = None) -> dict:
    owner(context)
    tier = (context or {}).get("tier")
    picks = []
    for pid in args.product_ids:
        p = store.one("SELECT * FROM products WHERE id = ?", (pid,))
        if p is None:
            raise ToolError("unknown_product", f"No product {pid}.", "search_products")
        price = price_for(store, dict(p), tier)
        picks.append({"product_id": pid, "name": p["name"], "price": price, "look": p["look"], "fabric": p["fabric"],
                      "over_budget": bool(args.budget and price["now"] > args.budget)})
    chosen = args.chosen or picks[0]["product_id"]
    pick = next((p for p in picks if p["product_id"] == chosen), picks[0])
    arrives = store.now().date() + timedelta(days=DELIVERY_DAYS)
    early = (date.fromisoformat(args.by) - arrives).days if args.by else None
    total = money(pick["price"]["now"] + (WRAP if args.wrap else 0))
    until = date.fromisoformat(store.policy_value("gift_exchange_until"))
    return {
        "picks": picks, "chosen": chosen, "size": args.size,
        "options": {"wrap": f"Paper wrap and card, +${WRAP:.2f}" if args.wrap else "No wrap",
                    "receipt": "Gift receipt, no prices shown", "exchange": f"They can swap the size until {until:%b} {until.day}"},
        "delivery": {"arrives": f"{arrives:%a %b} {arrives.day}", "on_time": early is None or early >= 0,
                     "note": (f"{_words(early)} days early" if early and early > 0 else "on time" if early == 0 else None)},
        "address_note": "Used for this order only. Not saved to your profile.",
        "total": total, "total_detail": f"{pick['name'].split()[-1].lower()} ${pick['price']['now']:.2f}" + (f" + wrap ${WRAP:.2f}" if args.wrap else ""),
        "saved_to_profile": False,
    }


def _words(n: int) -> str:
    return {1: "one", 2: "two", 3: "three", 4: "four", 5: "five", 6: "six", 7: "seven"}.get(n, str(n)).capitalize() if n else str(n)


def place_gift_order(store: Store, member_id: str, context: dict, product_id: str, size: str, wrap: bool, message: str,
                     recipient: str, address: str) -> dict:
    """Person-only: the shopper taps Place gift order. The address goes on this order only."""
    if not address.strip() or not recipient.strip():
        raise ToolError("address_needed", "Add who it is for and where it goes.", None)
    v = store.one("SELECT stock FROM variants WHERE product_id = ? AND size = ?", (product_id, size))
    if v is None or v["stock"] < 1:
        raise ToolError("out_of_stock", f"{product_id} is not available in {size}.", None)
    p = dict(store.one("SELECT * FROM products WHERE id = ?", (product_id,)))
    price = price_for(store, p, context.get("tier"))
    total = money(price["now"] + (WRAP if wrap else 0))
    oid = "#" + store.new_id("order").split("_")[1]
    now = store.now().isoformat(timespec="minutes")
    lines = [{"product_id": product_id, "size": size, "qty": 1, "price": p["price"], "gift": True, "wrap": wrap, "message": message,
              "receipt": "no prices"}]
    profile = json.loads(store.one("SELECT profile FROM members WHERE id = ?", (member_id,))["profile"]) if member_id else {}
    store.execute("INSERT INTO orders VALUES (?, ?, 'placed', ?, NULL, ?, ?, ?, 0, ?, NULL, ?, ?)",
                  (oid, member_id, now, json.dumps(lines), p["price"], money(price["now"] - p["price"]), total,
                   f"Card ending {profile.get('card_last4', '')}", json.dumps({"to": recipient, "address": address, "once": True})))
    store.execute("UPDATE variants SET stock = stock - 1 WHERE product_id = ? AND size = ?", (product_id, size))
    store.log(member_id, "place_gift_order", {"order_id": oid, "total": total})
    return {"order_id": oid, "total": total}
