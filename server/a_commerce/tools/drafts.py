"""New products from a supplier sheet (docs/plan.md §5.2 `product_drafts`, design/options/intent-listing).

Supplier codes become names, categories, sizes and prices from the pricing rule. Rows the sheet cannot answer are left
for a person. Drafts are not visible to shoppers; publishing them is a separate change set.
"""

from __future__ import annotations

import math

from pydantic import BaseModel, Field

from ..db import Store
from .base import Check, Plan, ToolError, read_tool, write_tool

WORDS = {"WOOL": "wool", "BLND": "blend", "OVRCT": "overcoat", "CHRCL": "charcoal", "MERINO": "merino", "CRW": "crew-neck sweater",
         "NVY": "navy", "FLNL": "flannel", "OVRSHT": "overshirt", "CHK": "check", "QLTD": "quilted", "LNR": "liner", "JKT": "jacket",
         "OLV": "olive", "CRD": "corduroy", "TRSR": "trousers", "SND": "sand", "LMBSWL": "lambswool", "CRDGN": "cardigan",
         "OAT": "oatmeal", "PEACOAT": "peacoat", "CSHMR": "cashmere", "OXFRD": "oxford", "SHRT": "shirt", "TRTLNK": "turtleneck",
         "WXD": "waxed", "COTN": "cotton", "BLK": "black", "RST": "rust", "BRWN": "brown"}
COLORS = {"charcoal", "navy", "check", "olive", "sand", "oatmeal", "black", "rust", "brown"}
CATEGORY = {"OT": "coats", "KN": "knitwear", "SH": "shirts", "PT": "trousers"}
CARE = {"coats": "Dry clean only.", "knitwear": "Hand wash cold, dry flat.", "shirts": "Machine wash at 30°C.", "trousers": "Machine wash at 30°C."}


def price_from_rule(store: Store, cost: float) -> float:
    """cost × markup, then the nearest price ending in 9 (docs/plan.md §7: $52 → $130 → $129)."""
    rule = store.policy_value("pricing_rule")
    raw = cost * rule["markup"]
    candidates = [math.floor(raw / 10) * 10 + rule["ending"] + k * 10 for k in (-1, 0)]
    return float(min(candidates, key=lambda c: (abs(c - raw), -c)))


def decode(code: str) -> tuple[str, str]:
    parts = code.split()
    cat = CATEGORY.get(parts[0].split("-")[1], "coats")
    words = [WORDS.get(w, w.lower()) for w in parts[1:]]
    color = next((w for w in words if w in COLORS), None)
    body = " ".join(w for w in words if w != color)
    name = body[0].upper() + body[1:] + (f", {color}" if color else "")
    return name, cat


def draft(store: Store, row) -> dict:
    name, cat = decode(row["code"])
    price = price_from_rule(store, row["cost"])
    problems = []
    if not row["fabric_mix"]:
        problems.append("Fill % missing in the sheet")
    if row["photo_items"] > 1:
        problems.append("Photo shows two items, pick one")
    sizes = row["sizes"].replace("-", "–")
    description = (f"{name.split(',')[0]} in {row['fabric_mix'] or 'a fabric the sheet does not give'}. "
                   f"Cut to layer, sized {sizes}. {CARE[cat]}")
    return {"row": row["row_no"], "code": row["code"], "name": name, "category": cat, "cost": row["cost"], "price": price,
            "floor": round(row["cost"] * (1 + store.policy_value("price_floor_markup") / 100), 2), "sizes": sizes,
            "fabric": row["fabric_mix"], "description": description,
            "status": "needs_you" if problems else "ready", "problem": "; ".join(problems) or None}


class SheetIn(BaseModel):
    sheet: str = Field("fw26_supplier.xlsx", description="Supplier sheet name.", examples=["fw26_supplier.xlsx"])


@read_tool("read_supplier_sheet", "Turn every row of a supplier sheet into a draft: name, category, sizes, price from the pricing "
           "rule, description; and say which rows need a person. Read only.", SheetIn, produces=["draft[]"])
def read_supplier_sheet(store: Store, args: SheetIn) -> dict:
    rows = store.all("SELECT * FROM supplier_rows WHERE sheet = ? ORDER BY row_no", (args.sheet,))
    if not rows:
        raise ToolError("unknown_sheet", f"No supplier sheet {args.sheet!r}.", None)
    drafts = [draft(store, r) for r in rows]
    return {"sheet": args.sheet, "rows": len(drafts), "ready": sum(d["status"] == "ready" for d in drafts),
            "needs_you": sum(d["status"] == "needs_you" for d in drafts), "items": drafts, "total": len(drafts),
            "next": "Call save_product_drafts for the ready rows; rows that need a person stay out."}


class SaveDraftsIn(BaseModel):
    sheet: str = Field("fw26_supplier.xlsx", description="Supplier sheet name.", examples=["fw26_supplier.xlsx"])
    only_ready: bool = Field(True, description="Save only rows with every field filled.", examples=[True])


@write_tool("save_product_drafts", "Save the sheet's ready rows as product drafts. Drafts are not visible to shoppers; publishing "
            "is a separate change set.", SaveDraftsIn, consumes=["draft[]"], produces=["draft_id[]"])
class SaveProductDrafts:
    @staticmethod
    def _drafts(store: Store, args: SaveDraftsIn) -> list[dict]:
        rows = store.all("SELECT * FROM supplier_rows WHERE sheet = ? ORDER BY row_no", (args.sheet,))
        if not rows:
            raise ToolError("unknown_sheet", f"No supplier sheet {args.sheet!r}.", "read_supplier_sheet")
        return [draft(store, r) for r in rows]

    @staticmethod
    def plan(store: Store, args: SaveDraftsIn) -> Plan:
        drafts = SaveProductDrafts._drafts(store, args)
        ready = [d for d in drafts if d["status"] == "ready" or not args.only_ready]
        names = {r["name"].lower() for r in store.all("SELECT name FROM products")}
        dupes = [d["name"] for d in ready if d["name"].lower() in names]
        below = [d["name"] for d in ready if d["price"] < d["floor"]]
        missing = [d for d in drafts if d["status"] == "needs_you"]
        checks = [
            Check("no_duplicates", not dupes, "No duplicate products in the catalog", ", ".join(dupes) or f"Checked {len(ready)} names"),
            Check("above_floor", not below, "All prices above the floor", ", ".join(below) or "Every price ≥ cost + 15%"),
            Check("required_fields", True, "Required fields: fabric, one photo per product",
                  f"{len(missing)} rows left for you" if missing else "All rows complete"),
        ]
        change = {"op": "create", "entity": "product_drafts", "sheet": args.sheet, "drafts": ready, "left_out": missing}
        return Plan([change], checks, None, "", summary=f"Save {len(ready)} drafts from {args.sheet}")

    @staticmethod
    def apply(store: Store, args: SaveDraftsIn) -> str:
        ids = []
        for d in SaveProductDrafts._drafts(store, args):
            if args.only_ready and d["status"] != "ready":
                continue
            did = store.new_id("draft")
            store.execute("INSERT INTO product_drafts VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 'draft')",
                          (did, args.sheet, d["row"], d["code"], d["name"], d["category"], d["price"], d["cost"], d["sizes"], d["description"]))
            ids.append(did)
        return ",".join(ids)
