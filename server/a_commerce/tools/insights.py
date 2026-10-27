"""Read-only answers about the business (docs/plan.md §4.10). Nothing here changes the store.

The estimate is arithmetic over the store's own data, so the model explains numbers it did not make up.
"""

from __future__ import annotations

from datetime import date, timedelta

from pydantic import BaseModel, Field

from ..db import Store
from .base import ToolError, read_tool

SIZE_ORDER = ["XS", "S", "M", "L", "XL"]


class SalesIn(BaseModel):
    day: str | None = Field(None, description="Date to explain, YYYY-MM-DD. Default: yesterday by the store clock.",
                            examples=["2026-11-05"])


@read_tool("explain_sales_change",
           "Explain why one day's revenue differs from the usual day of the same weekday: stockouts, campaigns that did or "
           "did not go out, and what is left unexplained. Read only. Suggests next steps that would each be a change set.",
           SalesIn, produces=["sales_explanation"])
def explain_sales_change(store: Store, args: SalesIn) -> dict:
    day = date.fromisoformat(args.day) if args.day else store.now().date() - timedelta(days=1)
    row = store.one("SELECT * FROM daily_sales WHERE day = ?", (day.isoformat(),))
    if row is None:
        raise ToolError("no_sales_data", f"No sales recorded for {day.isoformat()}.", "explain_sales_change")
    same = [store.one("SELECT * FROM daily_sales WHERE day = ?", ((day - timedelta(weeks=w)).isoformat(),)) for w in range(1, 5)]
    same = [r for r in same if r]
    usual = round(sum(r["revenue"] for r in same) / len(same), 2)
    usual_orders = round(sum(r["orders"] for r in same) / len(same))
    diff = round(row["revenue"] - usual, 2)

    causes = []
    day_end = f"{day.isoformat()}T23:59"
    outs = store.all("SELECT s.*, p.name FROM stockouts s JOIN products p ON p.id = s.product_id WHERE s.since <= ?", (day_end,))
    if outs:
        lost = round(sum(o["usual_revenue"] for o in outs), 2)
        sizes = sorted({sz for o in outs for sz in o["sizes"].split(",")}, key=SIZE_ORDER.index)
        since = outs[0]["since"]
        causes.append({"cause": f"{len(outs)} bestsellers out of {' and '.join(sizes)}",
                       "detail": ", ".join(o["name"] for o in outs),
                       "evidence": f"Sold out {date.fromisoformat(since[:10]):%a} {since[11:16]}; these sizes made ${lost:,.0f} "
                                   f"on an average {day:%A}", "estimated": -lost})
    for c in store.all("SELECT * FROM campaign_sends WHERE day = ? AND status != 'sent'", (day.isoformat(),)):
        causes.append({"cause": f"{c['name']} not sent", "detail": c["detail"],
                       "evidence": f"Email brings ${c['usual_revenue']:,.0f} on an average {day:%A}", "estimated": -c["usual_revenue"]})
    explained = round(sum(c["estimated"] for c in causes), 2)
    visits_usual = sum(r["visits"] for r in same) / len(same)
    causes.append({"cause": "Not explained", "detail": None,
                   "evidence": "Traffic and conversion otherwise normal" if abs(row["visits"] - visits_usual) / visits_usual < 0.1
                   else "Traffic changed too; not broken down further", "estimated": round(diff - explained, 2)})
    history = [dict(r) for r in store.all("SELECT day, revenue, orders FROM daily_sales WHERE day > ? AND day <= ? ORDER BY day",
                                          ((day - timedelta(days=14)).isoformat(), day.isoformat()))]
    next_steps = []
    if outs:
        sizes = sorted({sz for o in outs for sz in o["sizes"].split(",")}, key=SIZE_ORDER.index)
        next_steps.append(f"Reorder {' and '.join(sizes)} for the {len(outs)} products")
    if any(c["cause"].endswith("not sent") for c in causes):
        next_steps.append("Send the weekly email today")
    backs = sorted({o["back_on"] for o in outs if o["back_on"]})
    if backs:
        back = date.fromisoformat(backs[-1])
        next_steps.append(f"Show “back {back:%b} {back.day}” on the {len(outs)} product pages")
    return {"day": day.isoformat(), "revenue": row["revenue"], "orders": row["orders"], "usual_revenue": usual,
            "usual_orders": usual_orders, "difference": diff, "difference_pct": round(diff / usual * 100),
            "baseline": f"the last {len(same)} {day:%A}s", "causes": causes, "history": history,
            "next_steps": next_steps, "read_only": True}
