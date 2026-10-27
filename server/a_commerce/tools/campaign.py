"""One call for a whole sale (sale prices + coupon + banner), so a small model does not have to chain three writes.

It only makes previews with the same three write tools; approval, run and read-back are unchanged.
"""

from __future__ import annotations

import re
from datetime import datetime

from pydantic import BaseModel, Field, field_validator

from ..db import Store
from .base import REGISTRY, ToolError, make_preview, normalize, read_tool
from .operator import Audience, SaleItem, loose_overrides
from .periods import Period

LABEL = {"vip": "VIP", "gold": "Gold", "member": "Member", "members": "Member", "everyone": "Store"}


class PlanSaleIn(BaseModel):
    category: str = Field(description="Category id or name to put on sale.", examples=["summer outerwear"])
    percent_off: float = Field(gt=0, lt=100, description="Discount for the whole category. To change one product, keep this and use overrides.",
                               examples=[20])
    audience: Audience = Field(description="Who gets the sale.", examples=["vip"])
    period: Period | None = Field(None, description="When: this_weekend, next_weekend, today, tomorrow, next_7_days.",
                                  examples=["this_weekend"])
    starts_at: str | None = Field(None, description="Or an exact start, ISO.", examples=["2026-11-07T00:00"])
    ends_at: str | None = Field(None, description="Or an exact end, ISO.", examples=["2026-11-08T23:59"])
    on_main_page: bool = Field(False, description="Also show a banner in the main page hero slot to that audience.",
                               examples=[True])
    leave_out_low_stock: bool = Field(False, description="Leave out products under the policy's minimum stock.", examples=[True])
    overrides: list[SaleItem] = Field([], description="Products with a different discount, by name or id.",
                                      examples=[[{"product_id": "overshirt", "percent_off": 15}]])
    exclude: list[str] = Field([], description="Products to leave out, by name or id.", examples=[["chore coat"]])
    name: str | None = Field(None, description="Name shoppers see. Default: e.g. 'VIP weekend'.", examples=["VIP weekend"])

    _overrides = field_validator("overrides", mode="before")(lambda cls, v: loose_overrides(v))


@read_tool("plan_sale",
           "Prepare a whole sale in one call: sale prices for an audience over a period, the coupon that records it at "
           "checkout, and optionally a main-page banner. Returns previews and rule checks; nothing changes until a person approves.",
           PlanSaleIn, consumes=["tier"], produces=["preview_id[]"])
def plan_sale(store: Store, args: PlanSaleIn) -> dict:
    asked = {"tool": "plan_sale", "args": args.model_dump(mode="json", exclude_defaults=True)}
    if not (args.period or (args.starts_at and args.ends_at)):
        raise ToolError("no_period", "Give period (e.g. this_weekend) or both starts_at and ends_at.", "plan_sale")
    when = {"period": args.period} if args.period else {"starts_at": args.starts_at, "ends_at": args.ends_at}
    # call() has already turned a named period into dates; a Sat 00:00 – Sun 23:59 span is a weekend sale.
    start = datetime.fromisoformat(args.starts_at) if args.starts_at else None
    end = datetime.fromisoformat(args.ends_at) if args.ends_at else None
    weekend = args.period in ("this_weekend", "next_weekend") or bool(
        start and end and start.weekday() == 5 and start.hour == 0 and (end - start).days == 1 and end.weekday() == 6)
    name = args.name.split(":")[0].strip() if args.name else f"{LABEL[args.audience]} {'weekend' if weekend else 'sale'}"

    made: list[dict] = []

    def preview(tool: str, raw: dict) -> dict:
        t = REGISTRY[tool]
        made.append(make_preview(store, t, normalize(store, t, raw), actor="agent", asked=asked))
        return made[-1]

    try:
        sale = preview("set_sale_prices", {
            "name": name, "audience": args.audience, **when, "category": args.category, "percent_off": args.percent_off,
            "leave_out_low_stock": args.leave_out_low_stock, "overrides": [o.model_dump() for o in args.overrides],
            "exclude": args.exclude})
        ref = f"ref:{sale['preview_id']}"
        if args.audience != "everyone":
            preview("create_coupon", {"code": coupon_code(name, args.percent_off), "promotion_id": ref,
                                      "audience": args.audience, "auto_apply": True, "stacking": "no_tier_coupons"})
        if args.on_main_page:
            cat = normalize(store, REGISTRY["set_sale_prices"], {"category": args.category})["category"]
            row = store.one("SELECT name FROM categories WHERE id = ?", (cat,))
            preview("schedule_banner", {
                "slot_id": "slot_main_hero", "audience": args.audience, **when, "eyebrow": name.upper(),
                "headline": f"{args.percent_off:g}% off {(row['name'] if row else args.category).lower()}",
                "cta": "Shop the sale", "promotion_id": ref})
    except ToolError:
        # All or nothing: do not leave half a sale behind as open previews.
        for p in made:
            store.execute("UPDATE previews SET status = 'discarded' WHERE id = ? AND status = 'open'", (p["preview_id"],))
        raise
    checks = [c for p in made for c in p["checks"]]
    return {"preview_ids": [p["preview_id"] for p in made], "summary": [p["summary"] for p in made],
            "products": sale.get("changes_total", len(sale["changes"])),
            "checks": [{"rule": c["rule"], "passed": c["passed"], "detail": c["detail"], **({"fix": c["fix"]} if c.get("fix") else {})}
                       for c in checks],
            "next": "Call propose_change_set with these preview_ids. Mention any failed check in your reply."}

def coupon_code(name: str, percent_off: float) -> str:
    """'VIP weekend', 20 → VIPWEEKEND20. Only capitals and digits, 4 to 20 characters."""
    suffix = f"{percent_off:g}".replace(".", "")
    base = re.sub(r"[^A-Z0-9]", "", name.upper().split(":")[0])[: 20 - len(suffix)] or "SALE"
    return (base + suffix).ljust(4, "0")
