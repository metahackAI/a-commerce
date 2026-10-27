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
        if isinstance(calls, str):
            return Reply(calls, [], NAME, 0)
        return Reply("", [ToolCallReq(f"call_{len(results)}_{i}", n, a) for i, (n, a) in enumerate(calls)], NAME, 0)

    # -- the plan --------------------------------------------------------------
    def _next(self, system: str, request: str, results: list[dict]) -> list[tuple[str, dict]] | str:
        step = self._next_step(request, results)
        if step is not None:
            return step
        if ("sales" in request.lower() or "revenue" in request.lower()) and "You are revising" not in system:
            if not results:
                return [("explain_sales_change", {})]
            d = results[0]
            if "causes" not in d:
                return ""
            explained = abs(sum(c["estimated"] for c in d["causes"] if c["cause"] != "Not explained"))
            return (f"Revenue was ${d['revenue']:,.0f}, {abs(d['difference_pct'])}% under a normal Thursday. Most of it comes from two "
                    f"things: three bestsellers ran out of M and L on Wednesday afternoon, and the weekly email didn't go out. "
                    f"The two causes cover about ${round(explained, -3):,.0f} of the ${abs(d['difference']):,.0f}. I couldn't explain "
                    f"the rest; traffic and conversion look normal.")
        if "return" in request.lower() and "You are revising" not in system:
            return self._returns(request, results)
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

    def _next_step(self, request: str, results: list[dict]):
        """The sales answer's next steps: reorder, send the email, back-in-stock notices."""
        text = request.lower()
        if "supplier sheet" in text:
            if not results:
                return [("read_supplier_sheet", {})]
            if len(results) == 1:
                return [("save_product_drafts", {"only_ready": True})]
            if len(results) == 2 and "preview_id" in results[1]:
                r = results[0]
                return [("propose_change_set", {"title": "New products from supplier sheet", "preview_ids": [results[1]["preview_id"]],
                         "reply": f"The sheet uses short supplier codes, so I turned each row into a product name, category, sizes, "
                                  f"and a price from your pricing rule. {r['needs_you']} rows need you: 2 are missing the fabric mix "
                                  f"and 2 photos show more than one item."})]
            return []
        if text.startswith("reorder"):
            call, title = ("create_reorder", {"from_stockouts": True}), "Reorder sold-out sizes"
        elif "send the weekly email" in text or "send the email" in text:
            call, title = ("send_campaign", {"name": "Weekly email"}), "Send the weekly email"
        elif text.startswith("show") and "back" in text:
            call, title = ("set_back_in_stock_notice", {"from_stockouts": True}), "Back-in-stock notices"
        else:
            return None
        if not results:
            return [call]
        if len(results) == 1 and "preview_id" in results[0]:
            r = results[0]
            return [("propose_change_set", {"title": title, "preview_ids": [r["preview_id"]],
                                            "reply": f"{r['summary']}. Nothing changes until you approve."})]
        return []

    def _returns(self, request: str, results: list[dict]) -> list[tuple[str, dict]]:
        m = re.search(r"\$\s?(\d+)", request)
        if not results:
            return [("plan_return_review", {"min_amount": float(m.group(1)) if m else 200})]
        if len(results) == 1 and "preview_ids" in results[0]:
            r = results[0]
            return [("propose_change_set", {
                "title": f"Returns over ${int(float(m.group(1))) if m else 200}", "preview_ids": r["preview_ids"],
                "reply": f"{r['waiting']} return requests over ${int(float(m.group(1))) if m else 200} are waiting. The refund policy "
                         f"says a person approves refunds above $200, so I sorted them by what the policy says instead of refunding."})]
        return []

    def _sale_args(self, system: str, request: str, revising: bool) -> dict | None:
        text = request.lower()
        if revising:
            prior = json.loads(re.search(r"Its previews: (\[.*\])\. Call the same", system).group(1))
            base = next(p["args"] for p in prior if p["tool"] == "set_sale_prices")
            first = json.loads(re.search(r"Earlier requests: (\[.*?\])\. It was made", system).group(1))[0].lower()
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


class ScriptedShopper:
    """Stand-in for the shopper model in the storefront demo: Jamie compares and buys, Mina asks for photos."""

    name = NAME
    LIGHT_JACKETS = ["prd_windbreaker", "prd_field_jacket", "prd_overshirt"]

    def __init__(self, store: Store):
        self.store = store

    def complete(self, messages: list[dict], tools: list[dict]) -> Reply:
        if any(m["role"] == "tool" for m in messages):
            return Reply("", [], NAME, 0)
        system, text = messages[0]["content"], messages[1]["content"].lower()
        profile = json.loads(re.search(r"knows about this shopper: (\{.*?\})\n", system).group(1))
        prefs = {p["key"]: p["value"] for p in profile.get("preferences", [])}
        m = re.search(r"Earlier in this conversation: (\[.*\])", system)
        history = json.loads(m.group(1)) if m else []
        shown = next((h["shown"] for h in reversed(history) if h.get("shown")), self.LIGHT_JACKETS)
        calls = self._calls(text, prefs, shown, profile)
        return Reply("", [ToolCallReq(f"s{i}", n, a) for i, (n, a) in enumerate(calls)], NAME, 0)

    def _chips(self, text: str, prefs: dict) -> list[dict]:
        chips = [{"label": "Light jacket", "source": "conversation"}, {"label": "Spring travel", "source": "conversation"}]
        if "carry-on" in text or "packs" in text or "bulky" in text:
            chips.append({"label": "Packs small", "source": "conversation"})
        if "size" in prefs:
            chips.append({"label": f"Size {prefs['size']}", "source": "profile"})
        if "fabric" in prefs:
            chips.append({"label": prefs["fabric"], "source": "profile"})
        return chips

    def _calls(self, text: str, prefs: dict, shown: list[str], profile: dict) -> list[tuple[str, dict]]:
        search = ("search_products", {"category": "summer_outerwear", "fields": ["name", "weight_g", "fabric", "stock"], "limit": 50})
        if any(w in text for w in ("coming apart", "broke", "broken", "defect", "falling apart")):
            return [("get_my_orders", {}),
                    ("show_return_options", {"order_id": "#20402", "reason": "defect", "reply":
                        "Sorry about that. It looks like a defect, so you can have a new pair or a full refund, and return "
                        "shipping is free either way. Refunds over $200 are confirmed by a person at Northwind, usually within "
                        "one business day. A new pair doesn't wait."})]
        if text.startswith("refund") or "refund, please" in text:
            return [("show_return_options", {"order_id": "#20402", "reason": "defect", "chosen": "refund",
                                             "reply": "OK. Check the details on the right and send it. Nothing is sent until you do."})]
        if any(w in text for w in ("packs smallest", "bulky", "smallest")):
            return [("update_my_preferences", {"add": [{"key": "travel", "value": "Weight and packed size matter to you when traveling"}]}),
                    ("show_products", {"view": "compare_table", "product_ids": shown, "sort": "weight",
                                       "chips": self._chips(text + " packs", prefs),
                                       "reply": "Side by side on the right. The windbreaker folds into its own pocket and weighs 190 g. "
                                                "Of the natural fabrics, the field jacket is lightest."})]
        if any(w in text for w in ("bigger photos", "less text", "show photos", "show me photos")):
            return [("update_my_preferences", {"add": [{"key": "view", "value": "Photos over details"}]}),
                    ("show_products", {"view": "look_gallery", "product_ids": shown, "chips": self._chips(text, prefs),
                                       "reply": "Same results, bigger photos."})]
        if "compare" in text:
            return [("show_products", {"view": "compare_table", "product_ids": shown, "sort": "weight",
                                       "chips": self._chips(text, prefs), "reply": "Side by side on the right."})]
        if any(w in text for w in ("the windbreaker", "add it", "same size", "i'll take")):
            pid = next((p for p in shown if self._name(p).split()[-1].lower() in text), shown[0])
            size = prefs.get("size", "M")
            name = self._name(pid).split()[-1].lower()
            color = self.store.one("SELECT color FROM products WHERE id = ?", (pid,))["color"].lower()
            vip = " Your VIP weekend price is applied." if profile.get("tier") == "vip" else ""
            return [("add_to_cart", {"product_id": pid, "size": size}),
                    ("show_cart", {"reply": f"Added the {name} in {size}, {color}.{vip} Take a look at the cart when you're ready."})]
        if any(w in text for w in ("jacket", "coat", "looking for", "need a")):
            view = ("compare_table" if prefs.get("view") == "Compares specs"
                    else "look_gallery" if prefs.get("view") == "Photos over details" else "product_list")
            reply = ("Here are a few in your size. I kept to natural fabrics like you usually do, plus one packable nylon "
                     "that's hard to beat for a carry-on." if profile.get("tier") else
                     "Here are three. Tell me what matters most and I'll narrow it down.")
            return [search, ("show_products", {"view": view, "product_ids": self.LIGHT_JACKETS, "sort": "weight",
                                               "chips": self._chips(text, prefs), "reply": reply})]
        return []

    def _name(self, pid: str) -> str:
        return self.store.one("SELECT name FROM products WHERE id = ?", (pid,))["name"]
