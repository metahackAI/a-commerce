"""Returns story from docs/plan.md §4.7 and §4.9: Alex sends a defect return from the storefront; it joins the
operator's review of 12 returns over $200; one approval refunds 9 and sends 3 messages; refunds cannot be undone."""

import pytest
from fastapi.testclient import TestClient

from a_commerce import call, changesets
from a_commerce.api import app
from a_commerce.tools.base import ToolError
from a_commerce.tools.shopper import send_return_request

ALEX = {"shopper": True, "member_id": "mem_alex", "tier": "member"}


def test_alex_sees_two_options_and_only_alex_can_send(store):
    store.set_clock("2026-11-06T09:30")
    out = call(store, "get_return_options", {"order_id": "#20402", "reason": "defect"}, context=ALEX)
    assert out["order"] == {"id": "#20402", "delivered": "Sat Oct 31", "item": "Leather chelsea boots",
                            "variant": "Size 9 · Tan", "amount": 312.00}
    new, refund = out["options"]
    assert (new["kind"], new["title"], new["ships"], new["arrives"], new["you_pay"]) == (
        "replacement", "New pair, same size", "Today, 6 in stock", "Tue Nov 10", 0.0)
    assert (refund["amount"], refund["to"], refund["confirmed_by"]) == (
        312.00, "card ending 0937", "A person at Northwind, usually within 1 business day")
    assert out["send_back"]["cost"] == "free, defect"
    # There is no tool that sends a return; only the shopper's button does.
    assert call(store, "send_return_request", {}, context=ALEX)["error"]["code"] == "unknown_tool"


def review(store):
    store.set_clock("2026-11-06T10:40")
    plan = call(store, "plan_return_review", {"min_amount": 200})
    cs = changesets.create(store, "Returns over $200", "Review returns over $200", plan["preview_ids"])
    return plan, changesets.view(store, cs)


def test_the_review_matches_the_mockup_once_alex_sends(store):
    store.set_clock("2026-11-06T09:30")
    send_return_request(store, "mem_alex", "#20402", "refund", "defect", "sole coming apart", 2)
    plan, view = review(store)
    assert view["kind"] == "refund_review"
    assert view["summary"] == {"waiting": 12, "refund": {"count": 9, "amount": 2463.00}, "ask_for_photos": 2, "decline": 1}
    rows = {r["order_id"]: r for r in view["rows"]}
    assert (rows["#20402"]["suggested"], rows["#20402"]["evidence"], rows["#20402"]["rule"]) == (
        "refund", "2 photos · Day 6", "Defects are refunded in full, return shipping free")
    assert (rows["#20431"]["suggested"], rows["#20431"]["evidence"]) == ("refund", "1 photo · Day 4")
    assert (rows["#20366"]["suggested"], rows["#20366"]["rule"]) == ("ask_for_photos", "Damage claims need a photo")
    assert (rows["#20290"]["suggested"], rows["#20290"]["evidence"], rows["#20290"]["rule"]) == (
        "decline", "None · Day 39", "Outside the 30-day window")
    checks = {c["rule"]: (c["passed"], c["detail"]) for c in view["checks"]}
    assert checks == {
        "refund_to_original_payment": (True, "12 of 12 payments can be refunded"),
        "coupon_deducted": (True, "3 orders used a coupon"),
        "return_window": (True, "11 of 12 inside 30 days"),
        "return_history": (False, "One customer has 4 returns in 90 days (#20431)"),
    }
    assert view["approve_label"] == "Approve 9 refunds and send 3 messages"
    assert view["after_approval"]["text"] == "refund 9 · send 2 photo requests · send 1 decline note · read back each order"
    decline = next(m for m in view["messages"] if m["kind"] == "decline")
    assert decline["order_id"] == "#20290" and "39 days after delivery" in decline["text"]


def test_without_alex_there_are_11(store):
    _, view = review(store)
    assert view["summary"]["waiting"] == 11 and view["summary"]["refund"] == {"count": 8, "amount": 2151.00}


def test_approve_refunds_reads_back_and_cannot_undo(store):
    store.set_clock("2026-11-06T09:30")
    send_return_request(store, "mem_alex", "#20402", "refund", "defect", "", 2)
    _, view = review(store)
    store.set_clock("2026-11-06T11:15")
    live = changesets.approve(store, view["id"], version=1)
    assert live["status"] == "live" and live["verification"]["all_match"]
    assert len(live["verification"]["changes"]) == 12
    assert store.one("SELECT SUM(amount) AS s FROM refunds")["s"] == 2463.00
    assert store.one("SELECT COUNT(*) AS n FROM messages")["n"] == 3
    assert store.one("SELECT status FROM return_requests WHERE order_id = '#20402'")["status"] == "refunded"
    assert store.one("SELECT status FROM return_requests WHERE order_id = '#20290'")["status"] == "declined"
    with pytest.raises(ToolError) as e:
        changesets.undo(store, view["id"])
    assert e.value.code == "refunds_final"
    # A second review finds nothing left over $200.
    assert call(store, "plan_return_review", {"min_amount": 200})["error"]["code"] == "nothing_waiting"


def test_alex_over_http():
    client = TestClient(app)
    client.post("/demo/reset")
    client.post("/demo/clock", json={"at": "2026-11-06T09:30"})
    r = client.post("/returns", json={"order_id": "#20402", "option": "refund", "reason": "defect", "photos": 2},
                    headers={"X-Shopper": "alex"})
    assert r.status_code == 200 and r.json()["status"] == "waiting"
    again = client.post("/returns", json={"order_id": "#20402"}, headers={"X-Shopper": "alex"})
    assert again.status_code == 409 and again.json()["detail"]["code"] == "already_sent"
    other = client.post("/returns", json={"order_id": "#20402"}, headers={"X-Shopper": "jamie"})
    assert other.status_code == 409 and other.json()["detail"]["code"] == "unknown_order"


def test_alex_and_the_operator_through_the_agents():
    client = TestClient(app)
    client.post("/demo/reset")
    client.post("/demo/clock", json={"at": "2026-11-06T09:30"})
    events = client.post("/fo/chat", json={"message": "The sole on my chelsea boots is coming apart after a week."},
                         headers={"X-Shopper": "alex"}).json()["events"]
    block = next(e for e in events if e.get("ui") == "return_options")
    assert [o["kind"] for o in block["data"]["options"]] == ["replacement", "refund"]
    events = client.post("/fo/chat", json={"message": "Refund, please. I'll pick a different pair later."},
                         headers={"X-Shopper": "alex"}).json()["events"]
    assert events[0]["text"] == "OK. Check the details on the right and send it. Nothing is sent until you do."
    assert next(e for e in events if e.get("ui") == "return_options")["data"]["chosen"] == "refund"
    # Nothing was sent by the agent.
    assert client.post("/tools/search_return_requests", json={"args": {"min_amount": 200}}).json()["total"] == 11
    client.post("/returns", json={"order_id": "#20402", "option": "refund", "reason": "defect", "photos": 2}, headers={"X-Shopper": "alex"})

    client.post("/demo/clock", json={"at": "2026-11-06T10:40"})
    out = client.post("/bo/chat", json={"request": "Check returns waiting over $200"}).json()
    step, block = out["events"]
    assert step["text"].startswith("12 return requests over $200 are waiting.") and "corrected" not in step
    assert block["ui"] == "refund_review" and block["data"]["summary"]["refund"] == {"count": 9, "amount": 2463.00}
    live = client.post(f"/change-sets/{out['change_set_id']}/approve", json={"version": 1}).json()
    assert live["status"] == "live" and live["verification"]["all_match"]


def test_the_sale_is_1042_even_after_the_returns_review(store):
    from a_commerce.harness import ScriptedOperator, run_operator
    _, view = review(store)
    assert view["id"] == 1031
    store.set_clock("2026-11-06T11:02")
    out = run_operator(store, "Run a 20% weekend sale on summer outerwear for VIP members and put it on the main page.",
                       ScriptedOperator(store))
    assert out["change_set_id"] == 1042
