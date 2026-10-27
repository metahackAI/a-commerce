"""Fit check (design/storefront/fo-fit): which size, given what the shopper wears under it and what they already own.

It compares with garments the shopper kept, says which data it used, and which it did not have. Body measurements
are never asked for or guessed.
"""

from __future__ import annotations

import json
from datetime import datetime, timedelta
from typing import Literal

from pydantic import BaseModel, Field

from ..db import Store
from .base import ToolError, read_tool
from .shopper import DELIVERY_DAYS, owner

# Extra chest room each layer needs, over the shopper's reference size (cm, laid flat).
LAYER_ROOM = {"hoodie": 2.0, "sweater": 0.0, "shirt": -1.0}
LAYER_LABEL = {"hoodie": "Over a hoodie", "sweater": "Over a shirt or sweater", "shirt": "Over a shirt or sweater"}


def owned_with_measurements(store: Store, who: str) -> list[dict]:
    """Items the shopper has received and kept, newest first, that we have measurements for."""
    out = []
    for o in store.all("SELECT * FROM orders WHERE member_id = ? ORDER BY placed_at DESC", (who,)):
        received = o["status"] == "delivered" or (
            o["status"] == "placed" and datetime.fromisoformat(o["placed_at"]) + timedelta(days=DELIVERY_DAYS) <= store.now())
        returned = store.one("SELECT 1 FROM return_requests WHERE order_id = ?", (o["id"],))
        if not received or returned:
            continue
        for line in json.loads(o["lines"]):
            m = store.one("SELECT * FROM measurements WHERE product_id = ? AND size = ?", (line.get("product_id"), line.get("size")))
            if m:
                name = store.one("SELECT name FROM products WHERE id = ?", (line["product_id"],))["name"]
                out.append({**dict(m), "name": name, "short": name.split()[-1].lower()})
    return out


class FitIn(BaseModel):
    product_id: str = Field(description="Product to size.", examples=["prd_overcoat"])
    layers: list[Literal["hoodie", "sweater", "shirt"]] = Field(["hoodie", "sweater"], description="What the shopper might wear under it.",
                                                                 examples=[["hoodie", "sweater"]])


@read_tool("check_fit", "Recommend a size for each way the shopper might wear a product, comparing its measurements with something "
           "they own and kept, and with reviews. Says which data it used and which it did not have.",
           FitIn, consumes=["product_id"], produces=["fit"], scope="shopper")
def check_fit(store: Store, args: FitIn, context: dict | None = None) -> dict:
    who = owner(context)
    product = store.one("SELECT * FROM products WHERE id = ?", (args.product_id,))
    sizes = store.all("SELECT * FROM measurements WHERE product_id = ? ORDER BY chest", (args.product_id,))
    if product is None or not sizes:
        raise ToolError("no_measurements", f"No measurements for {args.product_id}.", "search_products")
    owned = owned_with_measurements(store, who)
    ref = owned[0] if owned else None
    prefs = {p["key"]: p["value"] for p in store.all("SELECT key, value FROM preferences WHERE member_id = ? AND status = 'active'", (who,))}
    base_chest = ref["chest"] if ref else next((s["chest"] for s in sizes if s["size"] == prefs.get("size")), None)
    if base_chest is None:
        raise ToolError("no_reference", "Nothing to compare with: no kept item with measurements and no saved size.", None)
    recs = []
    for layer in dict.fromkeys(LAYER_LABEL[layer] for layer in args.layers):
        key = next(k for k in args.layers if LAYER_LABEL[k] == layer)
        want = base_chest + LAYER_ROOM[key]
        pick = min(sizes, key=lambda s: (abs(s["chest"] - want), -s["chest"]))
        m_size = next((s for s in sizes if s["size"] == prefs.get("size", "M")), sizes[0])
        if pick["size"] != m_size["size"]:
            why = f"{pick['chest'] - m_size['chest']:g} cm more in the chest than {m_size['size']}, room for a {key}"
        else:
            why = f"{pick['chest']:g} cm chest, close to your {ref['short']} ({ref['chest']:g} cm), which you kept" if ref \
                else f"{pick['chest']:g} cm chest, your usual size"
        recs.append({"wearing": layer, "size": pick["size"], "why": why})
    review = store.one("SELECT * FROM review_summaries WHERE product_id = ?", (args.product_id,))
    phrases = json.loads(review["phrases"]) if review else {}
    show = [s for s in sizes if s["size"] in {r["size"] for r in recs}]
    rows = [{"label": lbl, "values": [f"{s[k]:g} cm" for s in show] + ([f"{ref[k]:g} cm"] if ref else [])}
            for lbl, k in (("Chest, laid flat", "chest"), ("Shoulder", "shoulder"), ("Sleeve", "sleeve"), ("Length", "length"))]
    return {
        "product": {"product_id": product["id"], "name": product["name"], "color": product["color"], "fabric": product["fabric"],
                    "price": product["price"], "exchange": f"Free size exchange within {store.policy_value('size_exchange_days')} days"},
        "recommendations": recs,
        "columns": [f"This {product['name'].split()[-1].lower()}, {s['size']}" for s in show] + ([f"Your {ref['short']}, {ref['size']}"] if ref else []),
        "rows": rows,
        "reviews": {"people": review["reviewers"], "heights": review["height_range"], "true_to_size": review["true_to_size"],
                    "snug": review["snug"], "phrases": phrases} if review else None,
        "used": ([f"Your orders: {ref['name'].split()[-1].title()} {ref['size']}, kept"] if ref else [])
                + ([f"Your profile: size {prefs['size']}" + (", layers hoodies" if "layers" in prefs else "")] if "size" in prefs else []),
        "not_used": ["Height and weight (you haven't shared them)"],
    }
