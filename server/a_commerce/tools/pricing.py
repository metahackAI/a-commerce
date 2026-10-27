"""Prices as a given person sees them at the demo clock. Shared by operator and shopper tools."""

from __future__ import annotations

from datetime import datetime
from decimal import ROUND_HALF_UP, Decimal

from ..db import Store


def sale_price(price: float, percent_off: float) -> float:
    value = Decimal(str(price)) * (Decimal(100) - Decimal(str(percent_off))) / Decimal(100)
    return float(value.quantize(Decimal("0.01"), rounding=ROUND_HALF_UP))


def floor_price(store: Store, cost: float) -> float:
    markup = Decimal(str(store.policy_value("price_floor_markup")))
    value = Decimal(str(cost)) * (Decimal(100) + markup) / Decimal(100)
    return float(value.quantize(Decimal("0.01"), rounding=ROUND_HALF_UP))


def audiences_for_tier(tier: str | None) -> list[str]:
    """Which promotion audiences a person belongs to."""
    if tier is None:
        return ["everyone"]
    return ["everyone", "members", tier]


def in_period(at: datetime, starts_at: str | None, ends_at: str | None) -> bool:
    ts = at.isoformat(timespec="minutes")
    return (starts_at is None or starts_at <= ts) and (ends_at is None or ts <= ends_at)


def promotion_state(at: datetime, starts_at: str | None, ends_at: str | None, status: str) -> str:
    if status != "live":
        return status
    ts = at.isoformat(timespec="minutes")
    if starts_at and ts < starts_at:
        return "scheduled"
    if ends_at and ts > ends_at:
        return "ended"
    return "running"


def best_sale(store: Store, product_id: str, tier: str | None, at: datetime | None = None) -> dict | None:
    """The lowest active sale price for this person, with the promotion that gives it."""
    at = at or store.now()
    audiences = audiences_for_tier(tier)
    rows = store.all(
        f"""SELECT p.id, p.name, p.audience, p.starts_at, p.ends_at, i.percent_off, i.sale_price
            FROM promotion_items i JOIN promotions p ON p.id = i.promotion_id
            WHERE i.product_id = ? AND p.status = 'live' AND p.kind = 'sale'
              AND p.audience IN ({",".join("?" * len(audiences))})""",
        (product_id, *audiences),
    )
    active = [r for r in rows if in_period(at, r["starts_at"], r["ends_at"])]
    if not active:
        return None
    best = min(active, key=lambda r: r["sale_price"])
    return {"promotion_id": best["id"], "promotion": best["name"], "percent_off": best["percent_off"],
            "sale_price": best["sale_price"]}


def price_for(store: Store, product: dict, tier: str | None, at: datetime | None = None) -> dict:
    sale = best_sale(store, product["id"], tier, at)
    if sale is None:
        return {"now": product["price"], "was": None, "note": None}
    return {"now": sale["sale_price"], "was": product["price"],
            "note": f"{sale['promotion']}, {sale['percent_off']:g}%", "promotion_id": sale["promotion_id"]}
