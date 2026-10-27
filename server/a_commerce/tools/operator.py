"""Operator tools that change the store (docs/plan.md §3.2)."""

from __future__ import annotations

from datetime import datetime
from typing import Literal

from pydantic import BaseModel, Field, field_validator

from ..db import Store
from .base import Check, Plan, ToolError, pending_ref, read_tool, write_tool
from .periods import Period
from .pricing import floor_price, price_for, promotion_state, sale_price

AUDIENCE_LABEL = {"vip": "VIP", "gold": "Gold", "member": "member", "members": "member", "everyone": "store"}

Audience = Literal["everyone", "members", "vip", "gold", "member"]

PRODUCT_FIELDS = ["id", "name", "category", "color", "fabric", "price", "cost", "stock", "stock_by_size",
                  "weight_g", "packed", "warmth", "light_rain", "colors", "audience_price"]


def product_row(store: Store, row, audience: str | None = None) -> dict:
    sizes = store.all("SELECT size, stock FROM variants WHERE product_id = ? ORDER BY rowid", (row["id"],))
    item = {
        "id": row["id"], "name": row["name"], "category": row["category_id"], "color": row["color"],
        "fabric": row["fabric"], "price": row["price"], "cost": row["cost"],
        "stock": sum(s["stock"] for s in sizes), "stock_by_size": {s["size"]: s["stock"] for s in sizes},
        "weight_g": row["weight_g"], "packed": row["packed"], "warmth": row["warmth"],
        "light_rain": bool(row["light_rain"]), "colors": row["colors"],
    }
    if audience and audience != "everyone":
        item["audience_price"] = price_for(store, item, audience)
    return item


def pick(item: dict, fields: list[str] | None) -> dict:
    return {k: v for k, v in item.items() if not fields or k in fields or k == "id"}


# -- reads ----------------------------------------------------------------

class GetPolicyIn(BaseModel):
    name: str | None = Field(None, description="One policy key, or omit for all.", examples=["max_discount"])


@read_tool("get_policy", "Read the store's business rules (discount limits, price floor, refund limits) with what each one means.",
           GetPolicyIn, produces=["policy"], scope="both")
def get_policy(store: Store, args: GetPolicyIn) -> dict:
    policy = store.policy
    if args.name:
        if args.name not in policy:
            raise ToolError("unknown_policy", f"No policy named {args.name}. Known: {', '.join(policy)}.", "get_policy")
        policy = {args.name: policy[args.name]}
    items = [{"name": k, "value": v["value"], "meaning": v["meaning"]} for k, v in policy.items()]
    return {"items": items, "total": len(items)}


class SearchProductsIn(BaseModel):
    query: str | None = Field(None, description="Words to match in the product name or fabric.", examples=["windbreaker"])
    category: str | None = Field(None, description="Category id or name.", examples=["summer_outerwear"])
    min_stock: int | None = Field(None, description="Only products with at least this many units in total.", examples=[10])
    max_stock: int | None = Field(None, description="Only products with at most this many units in total.", examples=[9])
    audience: Audience | None = Field(None, description="Also return the price this audience pays right now.", examples=["vip"])
    fields: list[str] | None = Field(None, description=f"Fields to return. Any of: {', '.join(PRODUCT_FIELDS)}.",
                                     examples=[["name", "price", "stock"]])
    limit: int = Field(20, ge=1, le=100, description="Page size.", examples=[20])
    offset: int = Field(0, ge=0, description="Items to skip.", examples=[0])


@read_tool("search_products", "Find products by words, category, or stock. Returns the items and the total count together.",
           SearchProductsIn, produces=["product_id[]"], scope="both")
def search_products(store: Store, args: SearchProductsIn, context: dict | None = None) -> dict:
    sql, params = "SELECT * FROM products WHERE status = 'live'", []
    if args.category:
        sql += " AND category_id = ?"
        params.append(args.category)
    if args.query:
        for word in args.query.lower().split():
            sql += " AND (lower(name) LIKE ? OR lower(fabric) LIKE ?)"
            params += [f"%{word}%", f"%{word}%"]
    audience = args.audience or (context or {}).get("tier")
    items = [product_row(store, r, audience) for r in store.all(sql + " ORDER BY rowid", params)]
    if args.min_stock is not None:
        items = [i for i in items if i["stock"] >= args.min_stock]
    if args.max_stock is not None:
        items = [i for i in items if i["stock"] <= args.max_stock]
    if context and context.get("shopper"):
        for i in items:  # shoppers never see cost
            i.pop("cost", None)
    page = items[args.offset: args.offset + args.limit]
    return {"items": [pick(i, args.fields) for i in page], "total": len(items)}


class GetProductIn(BaseModel):
    product_id: str = Field(description="Product id from search_products.", examples=["prd_windbreaker"])
    fields: list[str] | None = Field(None, description="Fields to return.", examples=[["name", "stock_by_size"]])


@read_tool("get_product", "Read one product with stock for each size.", GetProductIn,
           consumes=["product_id"], produces=["product"], scope="both")
def get_product(store: Store, args: GetProductIn, context: dict | None = None) -> dict:
    row = store.one("SELECT * FROM products WHERE id = ?", (args.product_id,))
    if row is None:
        raise ToolError("unknown_product", f"No product {args.product_id}.", "search_products")
    item = product_row(store, row)
    if context and context.get("shopper"):
        item.pop("cost")  # shoppers never see cost
    return pick(item, args.fields)


class NoInput(BaseModel):
    pass


@read_tool("list_tiers", "List member tiers with how many members each has.", NoInput, produces=["tier"])
def list_tiers(store: Store, args: NoInput) -> dict:
    rows = store.all("SELECT t.id, t.name, COUNT(m.id) AS members FROM tiers t LEFT JOIN members m ON m.tier_id = t.id"
                     " GROUP BY t.id ORDER BY t.rowid")
    items = [dict(r) for r in rows]
    return {"items": items, "total": len(items)}


class ListPromotionsIn(BaseModel):
    period: Period | None = Field(None, description="A named period instead of dates.", examples=["this_weekend"])
    starts_at: datetime = Field(description="Start of the period to check.", examples=["2026-11-07T00:00"])
    ends_at: datetime = Field(description="End of the period to check.", examples=["2026-11-08T23:59"])
    audience: Audience | None = Field(None, description="Only promotions for this audience.", examples=["vip"])


def overlapping_promotions(store: Store, starts_at: datetime, ends_at: datetime) -> list[dict]:
    s, e = starts_at.isoformat(timespec="minutes"), ends_at.isoformat(timespec="minutes")
    rows = store.all("SELECT * FROM promotions WHERE status = 'live' AND (starts_at IS NULL OR starts_at <= ?)"
                     " AND (ends_at IS NULL OR ends_at >= ?)", (e, s))
    return [dict(r) for r in rows]


@read_tool("list_active_promotions", "List promotions that run during a period, with who they are for.",
           ListPromotionsIn, produces=["promotion_id[]"])
def list_active_promotions(store: Store, args: ListPromotionsIn) -> dict:
    items = overlapping_promotions(store, args.starts_at, args.ends_at)
    if args.audience:
        items = [p for p in items if p["audience"] in ("everyone", args.audience)]
    now = store.now()
    for p in items:
        p["state"] = promotion_state(now, p["starts_at"], p["ends_at"], p["status"])
    return {"items": items, "total": len(items)}


class ListSlotsIn(BaseModel):
    page: Literal["main"] = Field("main", description="Store page.", examples=["main"])
    period: Period | None = Field(None, description="A named period instead of dates.", examples=["this_weekend"])
    starts_at: datetime = Field(description="Start of the period.", examples=["2026-11-07T00:00"])
    ends_at: datetime = Field(description="End of the period.", examples=["2026-11-08T23:59"])


@read_tool("list_display_slots", "List display slots on a page and what is booked in them for a period, and for whom.",
           ListSlotsIn, produces=["slot_id"])
def list_display_slots(store: Store, args: ListSlotsIn) -> dict:
    s, e = args.starts_at.isoformat(timespec="minutes"), args.ends_at.isoformat(timespec="minutes")
    items = []
    for slot in store.all("SELECT * FROM display_slots WHERE page = ?", (args.page,)):
        booked = store.all("SELECT id, audience, headline, starts_at, ends_at FROM displays WHERE slot_id = ? AND status = 'live'"
                           " AND (starts_at IS NULL OR starts_at <= ?) AND (ends_at IS NULL OR ends_at >= ?)",
                           (slot["id"], e, s))
        items.append({"slot_id": slot["id"], "page": slot["page"], "slot": slot["slot"], "booked": [dict(b) for b in booked]})
    return {"items": items, "total": len(items)}


# -- writes ---------------------------------------------------------------

class SaleItem(BaseModel):
    product_id: str = Field(description="Product id, or its name within the category.", examples=["prd_windbreaker"])
    percent_off: float = Field(gt=0, lt=100, description="Percent off the list price.", examples=[20])


def loose_overrides(value):
    """Accept {"overshirt": 15}, ["overshirt", {"percent_off": 15}] and [{"product_id": ..., "percent_off": ...}]."""
    if isinstance(value, dict):
        if "product_id" in value:
            return [value]
        return [{"product_id": k, "percent_off": v} for k, v in value.items()]
    if isinstance(value, list):
        out, pending = [], None
        for x in value:
            if isinstance(x, str):
                if pending is not None:
                    raise ValueError(f"{pending!r} has no percent_off. To leave products out of the sale, use exclude")
                pending = x
            elif isinstance(x, dict) and "product_id" not in x and pending is not None:
                out.append({"product_id": pending, **x})
                pending = None
            else:
                out.append(x)
        if pending is not None:
            raise ValueError(f"{pending!r} has no percent_off. To leave products out of the sale, use exclude")
        return out
    return value


class SetSalePricesIn(BaseModel):
    name: str = Field(description="Promotion name shoppers see next to the price.", examples=["VIP weekend"])
    audience: Audience = Field(description="Who gets the sale price.", examples=["vip"])
    period: Period | None = Field(None, description="A named period instead of dates, e.g. this_weekend for Sat 00:00 – Sun 23:59.",
                                  examples=["this_weekend"])
    starts_at: datetime = Field(description="First minute of the sale (or use period).", examples=["2026-11-07T00:00"])
    ends_at: datetime = Field(description="Last minute of the sale (or use period).", examples=["2026-11-08T23:59"])
    category: str | None = Field(None, description="Put every product in this category on sale (id or name).", examples=["summer_outerwear"])
    percent_off: float | None = Field(None, gt=0, lt=100, description="Discount for the category.", examples=[20])
    min_stock: int | None = Field(None, ge=0, description="Leave out products with fewer units than this.", examples=[10])
    overrides: list[SaleItem] = Field([], description="Products in the category that get a different discount.",
                                      examples=[[{"product_id": "prd_overshirt", "percent_off": 15}]])
    exclude: list[str] = Field([], description="Products to leave out, by id or name.", examples=[["prd_chore_coat"]])
    leave_out_low_stock: bool = Field(False, description="Leave out products under the policy's minimum stock for sales.",
                                      examples=[True])
    items: list[SaleItem] = Field([], description="Or list products one by one instead of a category.")

    _overrides = field_validator("overrides", mode="before")(lambda cls, v: loose_overrides(v))

    def resolve(self, store: Store) -> list[SaleItem]:
        """Turn the category form into explicit items, so the preview shows exactly what will run."""
        if self.items:
            return self.items
        if not (self.category and self.percent_off):
            raise ToolError("no_products", "Give a category and percent_off, or a list of items.", "search_products")
        rows = store.all("SELECT id FROM products WHERE category_id = ? AND status = 'live' ORDER BY rowid", (self.category,))
        if not rows:
            raise ToolError("unknown_category", f"No products in category {self.category}.", "search_products")
        special = {match_product(store, self.category, o.product_id): o.percent_off for o in self.overrides}
        excluded = {match_product(store, self.category, x) for x in self.exclude}
        if self.leave_out_low_stock and self.min_stock is None:
            self.min_stock = store.policy_value("promo_min_stock")
        items = []
        for r in rows:
            if r["id"] in excluded:
                continue
            if self.min_stock is not None:
                stock = store.one("SELECT SUM(stock) AS n FROM variants WHERE product_id = ?", (r["id"],))["n"]
                if stock < self.min_stock:
                    continue
            items.append(SaleItem(product_id=r["id"], percent_off=special.get(r["id"], self.percent_off)))
        return items


def max_pct_above_floor(row: dict) -> float:
    """Largest whole-percent discount that keeps this product at or above its floor."""
    pct = int(100 - row["floor"] * 100 / row["from"])
    return max(pct, 0)


def match_product(store: Store, category: str, ref: str) -> str:
    """A product id, or a name a person would use ("overshirt", "Seersucker overshirt"), within one category."""
    if store.one("SELECT 1 FROM products WHERE id = ? AND category_id = ?", (ref, category)):
        return ref
    rows = store.all("SELECT id, name FROM products WHERE category_id = ? AND lower(name) LIKE ?",
                     (category, f"%{ref.strip().lower()}%"))
    if len(rows) == 1:
        return rows[0]["id"]
    names = ", ".join(r["name"] for r in rows[:5])
    raise ToolError("product_not_found" if not rows else "product_ambiguous",
                    f"{ref!r} matches {len(rows)} products in {category}" + (f": {names}." if rows else ".") +
                    " Use a fuller name or the product id.", "search_products")


@write_tool("set_sale_prices",
            "Give an audience sale prices on products for a period. List prices do not change; undo removes the sale.",
            SetSalePricesIn, consumes=["product_id[]", "tier"], produces=["promotion_id"])
class SetSalePrices:
    @staticmethod
    def plan(store: Store, args: SetSalePricesIn) -> Plan:
        if args.ends_at <= args.starts_at:
            raise ToolError("bad_period", "The sale ends before it starts.", "set_sale_prices")
        args.items = args.resolve(store)
        products = {}
        for it in args.items:
            row = store.one("SELECT * FROM products WHERE id = ?", (it.product_id,))
            if row is None:
                raise ToolError("unknown_product", f"No product {it.product_id}.", "search_products")
            products[it.product_id] = product_row(store, row)

        rows, below_floor, low_stock = [], [], []
        min_stock = store.policy_value("promo_min_stock")
        for it in args.items:
            p = products[it.product_id]
            new, floor = sale_price(p["price"], it.percent_off), floor_price(store, p["cost"])
            rows.append({"op": "update", "entity": "sale_price", "id": p["id"], "name": p["name"], "stock": p["stock"],
                         "percent_off": it.percent_off, "from": p["price"], "to": new, "floor": floor})
            if new < floor:
                below_floor.append(f"{p['name']} would be ${new:.2f}, floor ${floor:.2f}")
            if p["stock"] < min_stock:
                low_stock.append(f"{p['name']} ({p['stock']} left)")

        limit = store.policy_value("max_discount").get(args.audience, store.policy_value("max_discount")["everyone"])
        top = max(it.percent_off for it in args.items)
        chosen = set(products)
        categories = {p["category"] for p in products.values()}
        left_out = [r for r in store.all(
            f"SELECT p.id FROM products p WHERE p.category_id IN ({','.join('?' * len(categories))})", list(categories))
            if r["id"] not in chosen]
        left_out_low = [i for i in (product_row(store, store.one("SELECT * FROM products WHERE id = ?", (r["id"],)))
                                    for r in left_out) if i["stock"] < min_stock]
        overlap = overlapping_promotions(store, args.starts_at, args.ends_at)
        running = [p for p in overlap if promotion_state(args.starts_at, p["starts_at"], p["ends_at"], p["status"]) == "running"]
        clash = [p["name"] for p in running if p["kind"] == "sale" and p["audience"] in ("everyone", args.audience)
                 and store.one(f"SELECT 1 FROM promotion_items WHERE promotion_id = ? AND product_id IN ({','.join('?' * len(chosen))})",
                               (p["id"], *chosen))]
        reduced = [r for r in rows if r["percent_off"] < top]

        checks = [
            Check("max_discount", top <= limit, f"Discount within the {AUDIENCE_LABEL.get(args.audience, args.audience)} limit",
                  f"Highest is {top:g}%; policy allows {limit:g}%", None if top <= limit else "set_sale_prices"),
            Check("price_floor", not below_floor, "No price below the floor",
                  "; ".join(below_floor) if below_floor else
                  (f"{', '.join(r['name'] for r in reduced)} at {reduced[0]['percent_off']:g}% to stay at cost + "
                   f"{store.policy_value('price_floor_markup')}%" if reduced else "Every sale price is at or above cost + 15%"),
                  None if not below_floor else "set_sale_prices",
                  None if not below_floor else "Give " + ", ".join(r["name"] for r in rows if r["to"] < r["floor"])
                  + " a smaller discount with overrides (at most "
                  + ", ".join(f"{max_pct_above_floor(r):g}%" for r in rows if r["to"] < r["floor"]) + "); keep percent_off for the rest."),
            Check("min_stock", not low_stock, "Enough stock",
                  f"Under {min_stock} units: {', '.join(low_stock)}" if low_stock else
                  (f"{len(left_out_low)} products under {min_stock} units excluded" if left_out_low else
                   f"Every product has {min_stock} or more units"),
                  None if not low_stock else "set_sale_prices",
                  None if not low_stock else "Pass leave_out_low_stock: true to leave them out."),
            Check("promotion_overlap", not clash, "No overlap with running promotions",
                  f"Overlaps with {', '.join(clash)}" if clash else f"Checked {len(running)} running promotions",
                  None if not clash else "list_active_promotions"),
        ]
        resolved = {k: v for k, v in args.model_dump(mode="json").items()
                    if k in ("name", "audience", "starts_at", "ends_at", "items")}
        return Plan(rows, checks, "prices", f"Changes {len(rows)} prices", resolved_args=resolved,
                    summary=f"{args.name}: {len(rows)} products for {args.audience}, "
                            f"{args.starts_at:%b %-d} – {args.ends_at:%b %-d}")

    @staticmethod
    def apply(store: Store, args: SetSalePricesIn) -> str:
        args.items = args.resolve(store)
        pid = store.new_id("promo")
        store.execute("INSERT INTO promotions VALUES (?, ?, 'sale', ?, ?, ?, 'live', ?)",
                      (pid, args.name, args.audience, args.starts_at.isoformat(timespec="minutes"),
                       args.ends_at.isoformat(timespec="minutes"), f"{len(args.items)} products"))
        for it in args.items:
            price = store.one("SELECT price FROM products WHERE id = ?", (it.product_id,))["price"]
            store.execute("INSERT INTO promotion_items VALUES (?, ?, ?, ?)",
                          (pid, it.product_id, it.percent_off, sale_price(price, it.percent_off)))
        return pid


class CreateCouponIn(BaseModel):
    code: str = Field(pattern=r"^[A-Z0-9]{4,20}$", description="Code in capitals and digits.", examples=["VIPWEEKEND20"])
    promotion_id: str = Field(description="The promotion this coupon records on orders, or ref:<preview_id>.",
                              examples=["ref:pv_3"])
    audience: Audience = Field(description="Who receives the coupon.", examples=["vip"])
    auto_apply: bool = Field(True, description="Apply at checkout without the shopper typing the code.", examples=[True])
    stacking: Literal["no_tier_coupons", "stacks"] = Field(
        "no_tier_coupons", description="Whether it may combine with tier coupons.", examples=["no_tier_coupons"])


@write_tool("create_coupon",
            "Issue a coupon to an audience that records a promotion's benefit on each order. It never adds a second discount.",
            CreateCouponIn, consumes=["promotion_id", "tier"], produces=["coupon_id"])
class CreateCoupon:
    @staticmethod
    def plan(store: Store, args: CreateCouponIn) -> Plan:
        if store.one("SELECT 1 FROM coupons WHERE code = ? AND status = 'live'", (args.code,)):
            raise ToolError("code_taken", f"Coupon code {args.code} is already in use.", "create_coupon")
        ref = pending_ref(store, args.promotion_id)
        if ref is None and not store.one("SELECT 1 FROM promotions WHERE id = ?", (args.promotion_id,)):
            raise ToolError("unknown_promotion", f"No promotion {args.promotion_id}.", "set_sale_prices")
        members = store.one("SELECT COUNT(*) AS n FROM members WHERE tier_id = ?", (args.audience,))["n"]
        allowed = store.policy_value("member_benefits_per_order")
        ok = args.stacking == "no_tier_coupons" or allowed > 1
        change = {"op": "create", "entity": "coupon", "code": args.code, "audience": args.audience,
                  "issued_to": members, "auto_apply": args.auto_apply, "stacking": args.stacking,
                  "promotion": args.promotion_id}
        checks = [Check("coupon_stacking", ok, "Coupon doesn't stack with tier coupons",
                        f"Stacking policy: one member benefit per order" if ok else
                        "Policy allows one member benefit per order; set stacking to no_tier_coupons",
                        None if ok else "create_coupon")]
        return Plan([change], checks, "coupons", "Issues a coupon",
                    f"Coupon {args.code} for {members:,} {args.audience} members")

    @staticmethod
    def apply(store: Store, args: CreateCouponIn) -> str:
        cid = store.new_id("coupon")
        members = store.one("SELECT COUNT(*) AS n FROM members WHERE tier_id = ?", (args.audience,))["n"]
        store.execute("INSERT INTO coupons VALUES (?, ?, ?, ?, ?, ?, ?, 'live')",
                      (cid, args.code, args.promotion_id, args.audience, int(args.auto_apply), args.stacking, members))
        return cid


class ScheduleBannerIn(BaseModel):
    slot_id: str = Field(description="Slot id from list_display_slots.", examples=["slot_main_hero"])
    audience: Audience = Field(description="Who sees it; others keep what they see now.", examples=["vip"])
    period: Period | None = Field(None, description="A named period instead of dates.", examples=["this_weekend"])
    starts_at: datetime = Field(description="First minute shown (or use period).", examples=["2026-11-07T00:00"])
    ends_at: datetime = Field(description="Last minute shown (or use period).", examples=["2026-11-08T23:59"])
    eyebrow: str | None = Field(None, description="Small line above the headline.", examples=["VIP WEEKEND"])
    headline: str = Field(description="Main line.", examples=["20% off summer outerwear"])
    cta: str | None = Field(None, description="Button text.", examples=["Shop the sale"])
    link: str | None = Field(None, description="Where the button goes.", examples=["/sale/vip-weekend"])
    promotion_id: str | None = Field(None, description="Promotion the banner advertises, or ref:<preview_id>.",
                                     examples=["ref:pv_3"])


@write_tool("schedule_banner", "Show a banner in a display slot to an audience for a period.",
            ScheduleBannerIn, consumes=["slot_id", "promotion_id"], produces=["display_id"])
class ScheduleBanner:
    @staticmethod
    def plan(store: Store, args: ScheduleBannerIn) -> Plan:
        if not store.one("SELECT 1 FROM display_slots WHERE id = ?", (args.slot_id,)):
            raise ToolError("unknown_slot", f"No slot {args.slot_id}.", "list_display_slots")
        s, e = args.starts_at.isoformat(timespec="minutes"), args.ends_at.isoformat(timespec="minutes")
        taken = store.all("SELECT headline FROM displays WHERE slot_id = ? AND audience = ? AND status = 'live'"
                          " AND (starts_at IS NULL OR starts_at <= ?) AND (ends_at IS NULL OR ends_at >= ?)",
                          (args.slot_id, args.audience, e, s))
        when = f"{args.starts_at:%b %-d}–{args.ends_at:%-d}"
        slot = store.one("SELECT slot FROM display_slots WHERE id = ?", (args.slot_id,))["slot"]
        change = {"op": "schedule", "entity": "banner", "slot_id": args.slot_id, "audience": args.audience,
                  "starts_at": s, "ends_at": e, "eyebrow": args.eyebrow, "headline": args.headline, "cta": args.cta}
        checks = [Check("slot_free", not taken, "Main page slot is free",
                        f"{slot.title()} slot open {when}" if not taken else
                        f"{slot.title()} slot already shows “{taken[0]['headline']}” to {args.audience}",
                        None if not taken else "list_display_slots")]
        return Plan([change], checks, "display", "Changes the main page", f"{slot.title()} slot, {when}, {args.audience}")

    @staticmethod
    def apply(store: Store, args: ScheduleBannerIn) -> str:
        did = store.new_id("disp")
        store.execute("INSERT INTO displays VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 'live')",
                      (did, args.slot_id, args.audience, args.starts_at.isoformat(timespec="minutes"),
                       args.ends_at.isoformat(timespec="minutes"), args.eyebrow, args.headline, args.cta, args.link,
                       args.promotion_id))
        return did
