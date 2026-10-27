"""A rule-based stand-in for the model, used when no model endpoint is configured and in tests.

It speaks the same protocol as a real model (tool calls in, tool results back), so everything the harness
does around it is exercised for real. It only understands sale requests like the demo's. Every event it
produces is labelled "scripted" so it is never mistaken for a model.
"""

from __future__ import annotations

import json
import re
from datetime import datetime, timedelta

from ..db import Store
from .models import Reply, ToolCallReq

NAME = "scripted"


class ScriptedOperator:
    name = NAME

    def __init__(self, store: Store):
        self.store = store

    def complete(self, messages: list[dict], tools: list[dict]) -> Reply:
        system, request = messages[0]["content"], messages[1]["content"]
        results = [json.loads(m["content"]) for m in messages if m["role"] == "tool"]
        calls = self._next(system, request, results)
        return Reply("", [ToolCallReq(f"call_{len(results)}_{i}", n, a) for i, (n, a) in enumerate(calls)], NAME, 0)

    # -- the plan --------------------------------------------------------------
    def _next(self, system: str, request: str, results: list[dict]) -> list[tuple[str, dict]]:
        revising = "You are revising change set" in system
        sale = self._sale_args(system, request, revising)
        if sale is None:
            return []
        weekend = {"starts_at": sale["starts_at"], "ends_at": sale["ends_at"]}
        step = len(results)
        if step == 0:
            reads = [("get_policy", {"name": "promo_min_stock" if revising else "max_discount"}),
                     ("search_products", {"category": sale["category"], "fields": ["name", "stock", "price"], "limit": 100})]
            if not revising:
                reads += [("list_tiers", {}), ("list_active_promotions", weekend), ("list_display_slots", weekend)]
            return reads
        n_reads = 2 if revising else 5
        if step == n_reads:
            return [("set_sale_prices", sale)]
        if step == n_reads + 1:
            pv = results[-1].get("preview_id")
            if not pv:
                return []
            banner = {"slot_id": "slot_main_hero", "audience": sale["audience"], **weekend,
                      "eyebrow": f"{sale['audience'].upper()} WEEKEND",
                      "headline": f"{sale['percent_off']:g}% off {self._category_name(sale['category']).lower()}",
                      "cta": "Shop the sale", "link": "/sale/vip-weekend", "promotion_id": f"ref:{pv}"}
            return [("create_coupon", {"code": f"{sale['audience'].upper()}WEEKEND{sale['percent_off']:g}",
                                       "promotion_id": f"ref:{pv}", "audience": sale["audience"],
                                       "auto_apply": True, "stacking": "no_tier_coupons"}),
                    ("schedule_banner", banner)]
        if step == n_reads + 3:
            previews = [r for r in results if "preview_id" in r]
            return [("propose_change_set", {
                "title": f"{sale['audience'].upper()} weekend sale: {self._category_name(sale['category']).lower()}",
                "preview_ids": [r["preview_id"] for r in previews],
                "reply": self._reply(previews, sale, revising)})]
        return []

    def _sale_args(self, system: str, request: str, revising: bool) -> dict | None:
        text = request.lower()
        if revising:
            prior = json.loads(re.search(r"Its previews: (\[.*\])\. Make new", system).group(1))
            base = next(p["args"] for p in prior if p["tool"] == "set_sale_prices")
            first = json.loads(re.search(r"Earlier requests: (\[.*?\])\. Its previews", system).group(1))[0].lower()
            sale = self._parse(first, base["starts_at"], base["ends_at"])
            prev = {i["product_id"]: i["percent_off"] for i in base["items"]}
            overrides = {pid: pct for pid, pct in prev.items() if pct != sale["percent_off"]}
            if "low-stock" in text or "low stock" in text:
                sale["min_stock"] = self.store.policy_value("promo_min_stock")
            elif any(i for i in base["items"]) and len(prev) < self._count(sale["category"]):
                sale["exclude"] = [r["id"] for r in self.store.all("SELECT id FROM products WHERE category_id = ?",
                                                                   (sale["category"],)) if r["id"] not in prev]
            for m in re.finditer(r"(?:give )?the ([a-z ]+?) (\d+)%", text):
                row = self.store.one("SELECT id FROM products WHERE category_id = ? AND lower(name) LIKE ?",
                                     (sale["category"], f"%{m.group(1).strip()}%"))
                if row:
                    overrides[row["id"]] = float(m.group(2))
            sale["overrides"] = [{"product_id": k, "percent_off": v} for k, v in overrides.items()]
            return sale
        if "%" not in text:
            return None
        return self._parse(text, None, None)

    def _parse(self, text: str, starts: str | None, ends: str | None) -> dict:
        pct = float(re.search(r"(\d+)\s*%", text).group(1))
        audience = "vip" if "vip" in text else "gold" if "gold" in text else "everyone"
        category = next((r["id"] for r in self.store.all("SELECT id, name FROM categories") if r["name"].lower() in text),
                        "summer_outerwear")
        if starts is None:
            sat = next_saturday(self.store.now())
            starts, ends = sat.isoformat(timespec="minutes"), (sat + timedelta(days=1, hours=23, minutes=59)).isoformat(timespec="minutes")
        return {"name": f"{audience.upper() if audience == 'vip' else audience.title()} weekend", "audience": audience,
                "starts_at": starts, "ends_at": ends, "category": category, "percent_off": pct}

    def _reply(self, previews: list[dict], sale: dict, revising: bool) -> str:
        checks = [c for p in previews for c in p["checks"]]
        order = ["min_stock", "price_floor"]
        failing = sorted((c for c in checks if not c["passed"]), key=lambda c: order.index(c["rule"]) if c["rule"] in order else 9)
        n = previews[0].get("changes_total", len(previews[0]["changes"])) if previews else 0
        if failing:
            reasons = []
            for c in failing:
                if c["rule"] == "min_stock":
                    k = c["detail"].count(" left)")
                    reasons.append(f"{k} products have fewer than {self.store.policy_value('promo_min_stock')} left")
                elif c["rule"] == "price_floor":
                    reasons.append(f"at {sale['percent_off']:g}% the {c['detail'].split(' would')[0]} would go below its price floor")
                else:
                    reasons.append(c["detail"])
            word = {1: "One thing needs", 2: "Two things need", 3: "Three things need"}.get(len(reasons), "Some things need")
            return ("I put this together as a change set. Nothing has changed in the store yet. "
                    f"{word} your call: {', and '.join(reasons)}.")
        if revising:
            special = [f"the {self._short(o['product_id'])} at {o['percent_off']:g}%" for o in sale.get("overrides", [])]
            return f"Done. {n} products now" + (f", {', '.join(special)}" if special else "") + f". All {len(checks)} checks pass."
        return f"I put this together as a change set. Nothing has changed in the store yet. All {len(checks)} checks pass."

    def _category_name(self, cid: str) -> str:
        return self.store.one("SELECT name FROM categories WHERE id = ?", (cid,))["name"]

    def _count(self, cid: str) -> int:
        return self.store.one("SELECT COUNT(*) AS n FROM products WHERE category_id = ?", (cid,))["n"]

    def _short(self, pid: str) -> str:
        return self.store.one("SELECT name FROM products WHERE id = ?", (pid,))["name"].split()[-1].lower()


def next_saturday(now: datetime) -> datetime:
    return (now + timedelta(days=(5 - now.weekday()) % 7 or 7)).replace(hour=0, minute=0)
