"""Replies are checked against the change set.

The first two false replies below are what Llama 3.2 3B said on Oct 28 (eval/results/). The count and
"all checks pass" claims were never seen from a real model; those sentences are written by hand.
"""

from a_commerce import call, changesets
from a_commerce.harness import ScriptedOperator, run_operator
from a_commerce.harness.grounding import check_reply, summary

from .test_harness import REQUEST_1, REQUEST_2


def v1_view(store):
    store.set_clock("2026-11-06T11:02")
    out = call(store, "plan_sale", {"category": "summer outerwear", "percent_off": 20, "audience": "vip",
                                    "period": "this_weekend", "on_main_page": True})
    return changesets.view(store, changesets.create(store, "VIP weekend sale: summer outerwear", REQUEST_1, out["preview_ids"]))


def test_real_false_replies_are_caught(store):
    view = v1_view(store)
    said = ("A 20% weekend sale on summer outerwear is now live for VIP members on the main page. However, some products "
            "may not be discounted due to the price floor rule. Additionally, the Cotton chore coat and Light rain parka "
            "will be excluded from the sale if they have less than 10 units in stock.")
    issues = check_reply(said, view)
    assert "It said the change is live, but nothing changes until you approve." in issues
    assert "It said low-stock products are left out, but they are still in this version." in issues
    assert check_reply("Dropped low-stock products, updated Seersucker overshirt discount to 15%.", view) == [
        "It said low-stock products are left out, but they are still in this version.",
        "It said the overshirt is at 15%, but it is at 20%."]
    assert check_reply("Changes 40 prices. All 6 checks pass.", view) == [
        "It said 40 prices, but the change set has 42.", "It said all checks pass, but 4 of 6 do."]


def test_questions_offers_and_negations_are_not_flagged(store):
    view = v1_view(store)
    for said in ("The coupon VIPWEEKEND20 is applied automatically at checkout.",
                 "Nothing is live until you approve.",
                 "Should I exclude the low-stock products?",
                 "Two products are low-stock; I can drop them if you want."):
        assert check_reply(said, view) == [], said


def test_the_main_rate_comes_from_the_change_set(store):
    store.set_clock("2026-11-06T11:02")
    out = call(store, "plan_sale", {"category": "summer_outerwear", "percent_off": 25, "audience": "gold",
                                    "period": "this_weekend", "overrides": [{"product_id": "Seersucker overshirt", "percent_off": 10}]})
    view = changesets.view(store, changesets.create(store, "Gold sale", "x", out["preview_ids"]))
    assert check_reply("The Seersucker overshirt is at 10%, everything else at 25%.", view) == []
    assert check_reply("The Seersucker overshirt is at 25%.", view) == ["It said the overshirt is at 25%, but it is at 10%."]


def test_true_replies_pass_through(store):
    store.set_clock("2026-11-06T11:02")
    model = ScriptedOperator(store)
    first = run_operator(store, REQUEST_1, model)
    assert "corrected" not in first["events"][0]
    second = run_operator(store, REQUEST_2, model, change_set_id=first["change_set_id"])
    assert "corrected" not in second["events"][0]
    assert second["events"][0]["text"] == "Done. 40 products now, the overshirt at 15%. All 6 checks pass."


def test_summary_is_built_from_the_change_set(store):
    assert summary(v1_view(store)) == (
        "Change set #1042 v1 is ready for review. Nothing has changed in the store yet. 42 products in Summer outerwear "
        "at 20% off. Coupon VIPWEEKEND20 for 1,284 VIP members. Main page hero, Nov 7–8. 4 of 6 checks pass. "
        "Needs your call: no price below the floor (Seersucker overshirt would be $63.20, floor $64.40); "
        "enough stock (Under 10 units: Cotton chore coat (7 left), Light rain parka (4 left)).")
