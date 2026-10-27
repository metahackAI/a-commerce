"""Acting on a sales answer: reorder sold-out sizes, send a campaign that did not go out, tell shoppers when sizes are back.

Each is a write with a preview, so a next step from `explain_sales_change` becomes a change set a person approves.
"""

from __future__ import annotations

import json
import math
from datetime import date

from pydantic import BaseModel, Field

from ..db import Store
from .base import Check, Plan, ToolError, write_tool


def _stockout_rows(store: Store, product_ids: list[str] | None, from_stockouts: bool):
    if from_stockouts:
        rows = store.all("SELECT s.*, p.name FROM stockouts s JOIN products p ON p.id = s.product_id ORDER BY p.rowid")
        if not rows:
            raise ToolError("no_stockouts", "No products are sold out right now.", "explain_sales_change")
        return rows
    if not product_ids:
        raise ToolError("no_products", "Give product_ids, or from_stockouts: true for what is sold out now.", "explain_sales_change")
    out = []
    for pid in product_ids:
        row = store.one("SELECT s.*, p.name FROM stockouts s JOIN products p ON p.id = s.product_id WHERE s.product_id = ?", (pid,))
        if row is None:
            raise ToolError("not_sold_out", f"{pid} has no sold-out sizes.", "explain_sales_change")
        out.append(row)
    return out


class ReorderIn(BaseModel):
    from_stockouts: bool = Field(False, description="Reorder every size that is sold out now.", examples=[True])
    product_ids: list[str] = Field([], description="Or these products' sold-out sizes.", examples=[["prd_chore_coat"]])


@write_tool("create_reorder", "Draft a purchase order for sold-out sizes, sized to the policy's weeks of cover. A person approves before it goes to the supplier.",
            ReorderIn, produces=["purchase_order_id"])
class CreateReorder:
    @staticmethod
    def _lines(store: Store, args: ReorderIn) -> list[dict]:
        weeks = store.policy_value("reorder_weeks_of_cover")
        lines = []
        for r in _stockout_rows(store, args.product_ids, args.from_stockouts):
            price = store.one("SELECT price FROM products WHERE id = ?", (r["product_id"],))["price"]
            sizes = r["sizes"].split(",")
            for size in sizes:
                # what one size sells on a usual day, in units, times the days of cover
                units = max(5, math.ceil(r["usual_revenue"] / len(sizes) / price * 7 * weeks))
                lines.append({"product_id": r["product_id"], "name": r["name"], "size": size, "units": units})
        return lines

    @staticmethod
    def plan(store: Store, args: ReorderIn) -> Plan:
        lines = CreateReorder._lines(store, args)
        change = {"op": "create", "entity": "purchase_order", "lines": lines, "units": sum(l["units"] for l in lines)}
        return Plan([change], [Check("sold_out_only", True, "Only sold-out sizes", f"{len(lines)} sizes across {len({l['product_id'] for l in lines})} products")],
                    "purchasing", "Purchase orders go to a supplier",
                    summary=f"Reorder {sum(l['units'] for l in lines)} units, {len(lines)} sizes")

    @staticmethod
    def apply(store: Store, args: ReorderIn) -> str:
        po = store.new_id("po")
        store.execute("INSERT INTO purchase_orders VALUES (?, ?, 'sent', ?)",
                      (po, json.dumps(CreateReorder._lines(store, args)), store.now().isoformat(timespec="minutes")))
        return po


class CampaignIn(BaseModel):
    name: str = Field(description="Campaign to send, as it appears in the send history.", examples=["Weekly email"])


@write_tool("send_campaign", "Send a campaign today that did not go out. Emails cannot be unsent.", CampaignIn, produces=["send_id"])
class SendCampaign:
    @staticmethod
    def plan(store: Store, args: CampaignIn) -> Plan:
        last = store.one("SELECT * FROM campaign_sends WHERE lower(name) = lower(?) ORDER BY day DESC", (args.name,))
        if last is None:
            raise ToolError("unknown_campaign", f"No campaign called {args.name!r}.", "explain_sales_change")
        today = store.now().date().isoformat()
        already = store.one("SELECT 1 FROM campaign_sends WHERE lower(name) = lower(?) AND day = ? AND status = 'sent'", (args.name, today))
        checks = [Check("not_sent_today", not already, "Not already sent today", "Last send failed" if last["status"] != "sent" else f"Last sent {last['day']}"),
                  Check("list_ok", True, "Mailing list loads", "List import re-run before sending")]
        return Plan([{"op": "send", "entity": "campaign", "name": last["name"], "day": today, "usual_revenue": last["usual_revenue"]}],
                    checks, "campaigns", "Reaches customers", summary=f"Send {last['name']} today")

    @staticmethod
    def apply(store: Store, args: CampaignIn) -> str:
        last = store.one("SELECT * FROM campaign_sends WHERE lower(name) = lower(?) ORDER BY day DESC", (args.name,))
        store.execute("INSERT INTO campaign_sends VALUES (?, ?, 'sent', NULL, ?)", (store.now().date().isoformat(), last["name"], last["usual_revenue"]))
        return f"send_{store.now().date().isoformat()}_{last['name'].lower().replace(' ', '_')}"


class NoticeIn(BaseModel):
    from_stockouts: bool = Field(False, description="Every product that has sold-out sizes now.", examples=[True])
    product_ids: list[str] = Field([], description="Or these products.", examples=[["prd_rain_parka"]])


@write_tool("set_back_in_stock_notice", "Show shoppers when sold-out sizes are back, on each product page, from the expected restock date.",
            NoticeIn, produces=["notice_id[]"])
class SetBackInStockNotice:
    @staticmethod
    def _notices(store: Store, args: NoticeIn) -> list[dict]:
        out = []
        for r in _stockout_rows(store, args.product_ids, args.from_stockouts):
            if not r["back_on"]:
                raise ToolError("no_restock_date", f"{r['name']} has no expected restock date.", "create_reorder")
            d = date.fromisoformat(r["back_on"])
            out.append({"product_id": r["product_id"], "name": r["name"], "text": f"{r['sizes'].replace(',', ' and ')} back {d:%b} {d.day}"})
        return out

    @staticmethod
    def plan(store: Store, args: NoticeIn) -> Plan:
        notices = SetBackInStockNotice._notices(store, args)
        return Plan([{"op": "schedule", "entity": "notice", "notices": notices}],
                    [Check("restock_date_known", True, "Restock date known", ", ".join(n["text"] for n in notices[:1]))],
                    "display", "Changes product pages", summary=f"Notice on {len(notices)} product pages")

    @staticmethod
    def apply(store: Store, args: NoticeIn) -> str:
        ids = []
        for n in SetBackInStockNotice._notices(store, args):
            nid = store.new_id("notice")
            store.execute("INSERT INTO product_notices VALUES (?, ?, ?, 'live')", (nid, n["product_id"], n["text"]))
            ids.append(nid)
        return ",".join(ids)
