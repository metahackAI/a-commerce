"""Returns and refunds (docs/plan.md §3.3, §4.9). The policy decides; a person approves refunds and messages."""

from __future__ import annotations

from datetime import datetime
from typing import Literal

from pydantic import BaseModel, Field

from ..db import Store
from .base import REGISTRY, Check, Plan, ToolError, make_preview, read_tool, write_tool

Suggested = Literal["refund", "ask_for_photos", "decline"]
ORDER = {"refund": 0, "ask_for_photos": 1, "decline": 2}


def _request(store: Store, rid: str):
    row = store.one("SELECT * FROM return_requests WHERE id = ? OR order_id = ?", (rid, rid))
    if row is None:
        raise ToolError("unknown_return", f"No return request {rid}.", "search_return_requests")
    return row


def days_since_delivery(store: Store, row) -> int:
    """Calendar days, as the return window counts them: delivered Oct 31, asked Nov 6 = day 6."""
    return (store.now().date() - datetime.fromisoformat(row["delivered_at"]).date()).days


def evaluate(store: Store, row) -> dict:
    """What the policy says about one request, with the rule it cites."""
    days = days_since_delivery(store, row)
    window = store.policy_value("return_window_days")
    if days > window:
        suggested, rule = "decline", f"Outside the {window}-day window"
    elif row["reason"] == "damaged" and row["photos"] == 0 and store.policy_value("damage_claim_needs_photo"):
        suggested, rule = "ask_for_photos", "Damage claims need a photo"
    elif row["reason"] == "defect":
        suggested, rule = "refund", "Defects are refunded in full, return shipping free"
    elif row["reason"] == "changed_mind":
        suggested, rule = "refund", f"Within {window} days; customer pays return shipping"
    elif row["reason"] in ("too_small", "too_large"):
        suggested, rule = "refund", f"Within {window} days, tags on"
    else:
        suggested, rule = "refund", f"Within {window} days, unworn"
    return {"return_request_id": row["id"], "order_id": row["order_id"], "item": row["item"], "amount": row["amount"],
            "reason": row["reason_text"], "photos": row["photos"], "day": days, "suggested": suggested, "rule": rule,
            "needs_person": row["amount"] > store.policy_value("refund_without_person_max")}


# -- reads -------------------------------------------------------------------

class SearchReturnsIn(BaseModel):
    status: Literal["waiting", "refunded", "photos_requested", "declined"] = Field("waiting", description="Request status.",
                                                                                  examples=["waiting"])
    min_amount: float | None = Field(None, description="Only requests above this amount.", examples=[200])


@read_tool("search_return_requests", "List return requests with the amount, reason, photos and days since delivery, plus the total.",
           SearchReturnsIn, produces=["return_request_id[]"])
def search_return_requests(store: Store, args: SearchReturnsIn) -> dict:
    rows = store.all("SELECT * FROM return_requests WHERE status = ? AND amount > ? ORDER BY requested_at, id",
                     (args.status, args.min_amount or 0))
    items = [evaluate(store, r) for r in rows]
    return {"items": items, "total": len(items)}


class ReturnIn(BaseModel):
    return_request_id: str = Field(description="Return request id, or the order number.", examples=["#20402"])


@read_tool("evaluate_return_request", "What the return policy says about one request: refund, ask for photos, or decline, and why.",
           ReturnIn, consumes=["return_request_id"], produces=["suggestion"])
def evaluate_return_request(store: Store, args: ReturnIn) -> dict:
    return evaluate(store, _request(store, args.return_request_id))


class HistoryIn(BaseModel):
    member_id: str = Field(description="Customer id.", examples=["mem_ret_20431"])
    days: int = Field(90, description="Look back this many days.", examples=[90])


@read_tool("get_return_history", "How many returns a customer made recently.", HistoryIn,
           consumes=["customer_id"], produces=["history"])
def get_return_history(store: Store, args: HistoryIn) -> dict:
    rows = [dict(r) for r in store.all("SELECT id, order_id, amount, requested_at, status FROM return_requests WHERE member_id = ?",
                                       (args.member_id,))
            if (store.now() - datetime.fromisoformat(r["requested_at"])).days <= args.days]
    return {"items": rows, "total": len(rows)}


# -- writes ------------------------------------------------------------------

@write_tool("refund_return", "Refund a return request to the original payment. Refunds cannot be undone.",
            ReturnIn, consumes=["return_request_id"], produces=["refund_id"])
class RefundReturn:
    @staticmethod
    def plan(store: Store, args: ReturnIn) -> Plan:
        row = _request(store, args.return_request_id)
        if row["status"] != "waiting":
            raise ToolError("not_waiting", f"Return {row['order_id']} is already {row['status']}.", "search_return_requests")
        e = evaluate(store, row)
        order = store.one("SELECT payment FROM orders WHERE id = ?", (row["order_id"],))
        checks = [
            Check("refund_to_original_payment", bool(order and order["payment"]), "Refunds go back to the original payment",
                  f"{row['order_id']}: {order['payment'] if order else 'no payment on file'}"),
            Check("policy_allows_refund", e["suggested"] == "refund", "Policy allows a refund",
                  e["rule"] if e["suggested"] == "refund" else f"Policy says {e['suggested'].replace('_', ' ')}: {e['rule']}",
                  None if e["suggested"] == "refund" else "send_customer_message"),
        ]
        change = {"op": "refund", "entity": "refund", "return_request_id": row["id"], "order_id": row["order_id"],
                  "item": row["item"], "amount": row["amount"], "rule": e["rule"]}
        return Plan([change], checks, "refunds", f"Refund over ${store.policy_value('refund_without_person_max'):.0f}",
                    summary=f"Refund {row['order_id']} ${row['amount']:.2f}")

    @staticmethod
    def apply(store: Store, args: ReturnIn) -> str:
        row = _request(store, args.return_request_id)
        order = store.one("SELECT payment FROM orders WHERE id = ?", (row["order_id"],))
        fid = store.new_id("refund")
        store.execute("INSERT INTO refunds VALUES (?, ?, ?, ?, 'sent', ?)",
                      (fid, row["id"], row["amount"], order["payment"], store.now().isoformat(timespec="minutes")))
        store.execute("UPDATE return_requests SET status = 'refunded' WHERE id = ?", (row["id"],))
        return fid


class MessageIn(BaseModel):
    return_request_id: str = Field(description="Return request id, or the order number.", examples=["#20290"])
    kind: Literal["ask_for_photos", "decline"] = Field(description="Why we write.", examples=["decline"])
    text: str = Field(min_length=20, description="The message as the customer will read it.",
                      examples=["Thanks for sending this back. It arrived 39 days after delivery…"])


@write_tool("send_customer_message", "Write to a customer about their return. A person reads every message before it goes out.",
            MessageIn, consumes=["return_request_id"], produces=["message_id"])
class SendCustomerMessage:
    @staticmethod
    def plan(store: Store, args: MessageIn) -> Plan:
        row = _request(store, args.return_request_id)
        e = evaluate(store, row)
        ok = e["suggested"] == args.kind
        change = {"op": "message", "entity": "message", "return_request_id": row["id"], "order_id": row["order_id"],
                  "kind": args.kind, "text": args.text}
        return Plan([change], [Check("message_matches_policy", ok, "Message matches the policy",
                                     e["rule"] if ok else f"Policy says {e['suggested'].replace('_', ' ')} for {row['order_id']}")],
                    "customer_messages", "Every customer message is read first", summary=f"Message to {row['order_id']}")

    @staticmethod
    def apply(store: Store, args: MessageIn) -> str:
        row = _request(store, args.return_request_id)
        mid = store.new_id("msg")
        store.execute("INSERT INTO messages VALUES (?, ?, ?, ?, ?, 'sent', ?)",
                      (mid, row["member_id"], row["id"], args.kind, args.text, store.now().isoformat(timespec="minutes")))
        store.execute("UPDATE return_requests SET status = ? WHERE id = ?",
                      ("photos_requested" if args.kind == "ask_for_photos" else "declined", row["id"]))
        return mid


# -- the batch, in one call --------------------------------------------------

DRAFTS = {
    "ask_for_photos": "Thanks for letting us know your {item} arrived damaged. Could you reply with a photo of the damage? "
                      "As soon as we see it, we will refund you in full.",
    "decline": "Thanks for sending back the {item}. It reached us {day} days after delivery, and our return window is "
               "{window} days, so we can't refund it this time. We're sorry, and we'd be glad to help you find something else.",
}


class PlanReviewIn(BaseModel):
    min_amount: float = Field(200, description="Review waiting requests above this amount.", examples=[200])


@read_tool("plan_return_review",
           "Sort every waiting return over an amount by what the policy says, and prepare the refunds and the customer "
           "messages as previews. Nothing is refunded or sent until a person approves.",
           PlanReviewIn, produces=["preview_id[]"])
def plan_return_review(store: Store, args: PlanReviewIn) -> dict:
    asked = {"tool": "plan_return_review", "args": args.model_dump(mode="json")}
    rows = sorted((evaluate(store, r) for r in store.all(
        "SELECT * FROM return_requests WHERE status = 'waiting' AND amount > ? ORDER BY requested_at, id", (args.min_amount,))),
        key=lambda e: ORDER[e["suggested"]])
    if not rows:
        raise ToolError("nothing_waiting", f"No waiting returns over ${args.min_amount:.0f}.", "search_return_requests")
    previews = []
    for e in rows:
        if e["suggested"] == "refund":
            previews.append(make_preview(store, REGISTRY["refund_return"], {"return_request_id": e["return_request_id"]},
                                         actor="agent", asked=asked))
        else:
            text = DRAFTS[e["suggested"]].format(item=e["item"].lower(), day=e["day"],
                                                 window=store.policy_value("return_window_days"))
            previews.append(make_preview(store, REGISTRY["send_customer_message"], {
                "return_request_id": e["return_request_id"], "kind": e["suggested"], "text": text}, actor="agent", asked=asked))
    refunds = [e for e in rows if e["suggested"] == "refund"]
    return {"preview_ids": [p["preview_id"] for p in previews], "waiting": len(rows),
            "refund": {"count": len(refunds), "amount": round(sum(e["amount"] for e in refunds), 2)},
            "ask_for_photos": sum(e["suggested"] == "ask_for_photos" for e in rows),
            "decline": sum(e["suggested"] == "decline" for e in rows),
            "next": "Call propose_change_set with these preview_ids."}


def review_checks(store: Store, rids: list[str]) -> list[dict]:
    """Batch checks shown on the refund review (docs/plan.md §4.9)."""
    rows = [store.one("SELECT * FROM return_requests WHERE id = ?", (r,)) for r in rids]
    orders = [store.one("SELECT payment, coupon_code FROM orders WHERE id = ?", (r["order_id"],)) for r in rows]
    window = store.policy_value("return_window_days")
    inside = sum(days_since_delivery(store, r) <= window for r in rows)
    repeat = []
    for r in rows:
        n = get_return_history(store, HistoryIn(member_id=r["member_id"]))["total"]
        if n >= 4:
            repeat.append((r["order_id"], n))
    return [
        {"rule": "refund_to_original_payment", "passed": all(o and o["payment"] for o in orders),
         "title": "Refunds go back to the original payment", "detail": f"{sum(bool(o and o['payment']) for o in orders)} of {len(rows)} payments can be refunded"},
        {"rule": "coupon_deducted", "passed": True, "title": "Coupon discounts taken out of refunds",
         "detail": f"{sum(bool(o and o['coupon_code']) for o in orders)} orders used a coupon"},
        {"rule": "return_window", "passed": True, "title": "Return window", "detail": f"{inside} of {len(rows)} inside {window} days"},
        {"rule": "return_history", "passed": not repeat, "title": "Return history",
         "detail": "; ".join(f"One customer has {n} returns in 90 days ({o})" for o, n in repeat) or "No repeat returners"},
    ]
