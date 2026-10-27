"""Acceptance: the VIP weekend story from docs/plan.md §4.6 and §4.7, end to end, without a model.

The tool calls here are the plan a model should produce. If this test passes, the store side of the demo works;
what is left for the harness is to produce these calls from the request.
"""

import pytest

from a_commerce import call, changesets
from a_commerce.tools.base import ToolError
from a_commerce.tools.shopper import place_order

from .conftest import JAMIE

REQUEST_1 = "Run a 20% weekend sale on summer outerwear for VIP members and put it on the main page."
REQUEST_2 = "Drop the low-stock ones. Give the overshirt 15% instead."
WEEKEND = {"starts_at": "2026-11-07T00:00", "ends_at": "2026-11-08T23:59"}


def previews(store, items):
    sale = call(store, "set_sale_prices", {"name": "VIP weekend", "audience": "vip", **WEEKEND, "items": items})
    coupon = call(store, "create_coupon", {"code": "VIPWEEKEND20", "promotion_id": f"ref:{sale['preview_id']}",
                                           "audience": "vip", "auto_apply": True, "stacking": "no_tier_coupons"})
    banner = call(store, "schedule_banner", {"slot_id": "slot_main_hero", "audience": "vip", **WEEKEND,
                                             "eyebrow": "VIP WEEKEND", "headline": "20% off summer outerwear",
                                             "cta": "Shop the sale", "promotion_id": f"ref:{sale['preview_id']}"})
    return sale, coupon, banner


def summer(store):
    return call(store, "search_products", {"category": "summer_outerwear", "limit": 100})["items"]


def build_v1_v2(store):
    store.set_clock("2026-11-06T11:02")
    v1 = previews(store, [{"product_id": p["id"], "percent_off": 20} for p in summer(store)])
    cs = changesets.create(store, "VIP weekend sale: summer outerwear", REQUEST_1, [p["preview_id"] for p in v1])

    store.set_clock("2026-11-06T11:05")
    items = [{"product_id": p["id"], "percent_off": 15 if p["id"] == "prd_overshirt" else 20}
             for p in summer(store) if p["stock"] >= 10]
    v2 = previews(store, items)
    changesets.revise(store, cs, REQUEST_2, [p["preview_id"] for p in v2])
    return cs, v1, v2


def test_v1_flags_low_stock_and_price_floor(store):
    cs, v1, _ = build_v1_v2(store)
    failing = {c["rule"]: c["detail"] for c in v1[0]["checks"] if not c["passed"]}
    assert set(failing) == {"min_stock", "price_floor"}
    assert "Seersucker overshirt would be $63.20, floor $64.40" in failing["price_floor"]
    assert "Cotton chore coat (7 left)" in failing["min_stock"] and "Light rain parka (4 left)" in failing["min_stock"]
    assert v1[0]["needs_approval"] is True


def test_v2_matches_the_mockup(store):
    cs, _, _ = build_v1_v2(store)
    view = changesets.view(store, cs)
    assert (view["id"], view["version"], view["status"]) == (1042, 2, "waiting_for_approval")
    assert (view["checks_passed"], view["checks_total"]) == (6, 6)
    details = {c["rule"]: c["detail"] for c in view["checks"]}
    assert details["max_discount"] == "Highest is 20%; policy allows 30%"
    assert details["min_stock"] == "2 products under 10 units excluded"
    assert details["promotion_overlap"] == "Checked 4 running promotions"
    assert details["coupon_stacking"] == "Stacking policy: one member benefit per order"
    assert details["slot_free"] == "Hero slot open Nov 7–8"

    understood = {u["label"]: (u["value"], u["detail"]) for u in view["understood"]}
    assert understood["Who"] == ("VIP tier", "1,284 members")
    assert understood["Products"] == ("Summer outerwear", "40 of 42 · 2 excluded")
    assert understood["Benefit"] == ("20% off", "Overshirt 15%")
    assert understood["When"] == ("Nov 7–8", "Sat 00:00 – Sun 23:59")
    assert understood["Where"] == ("Main page hero", "Shown to VIP only")

    groups = {g["op"]: g for g in view["groups"]}
    prices = {r["name"]: (r["stock"], r["discount"], r["was"], r["now"]) for r in groups["update"]["rows"]}
    assert groups["update"]["total_rows"] == 40
    assert prices["Linen blend field jacket"] == (86, "20%", 128.00, 102.40)
    assert prices["Packable windbreaker"] == (214, "20%", 89.00, 71.20)
    assert prices["Seersucker overshirt"] == (53, "15%", 79.00, 67.15)
    assert prices["Cotton utility jacket"] == (31, "20%", 118.00, 94.40)
    assert groups["create"]["fields"]["Issued to"] == "1,284 VIP members"
    assert {r["name"]: r["note"] for r in groups["exclude"]["rows"]} == {"Cotton chore coat": "7 left", "Light rain parka": "4 left"}
    assert view["after_approval"]["undo_until"] == "2026-11-09T23:59"
    assert {c["rule"]: c["title"] for c in view["checks"]}["max_discount"] == "Discount within the VIP limit"
    assert view["after_approval"]["tools"] == ["set_sale_prices", "create_coupon", "schedule_banner", "verify_change_set"]


def test_nothing_changes_before_approval(store):
    build_v1_v2(store)
    store.set_clock("2026-11-07T10:00")
    p = call(store, "compare_products", {"product_ids": ["prd_windbreaker", "prd_field_jacket"], "attributes": ["price"]},
             context=JAMIE)
    assert p["rows"][0]["values"][0]["now"] == 89.00


def test_agent_cannot_apply_its_own_preview(store):
    _, _, v2 = build_v1_v2(store)
    res = call(store, "set_sale_prices", {"preview_id": v2[0]["preview_id"]})
    assert res["error"]["code"] == "person_only"


def test_approving_an_old_version_is_refused(store):
    cs, _, _ = build_v1_v2(store)
    with pytest.raises(ToolError) as e:
        changesets.approve(store, cs, version=1)
    assert e.value.code == "stale_version"


def test_approve_run_read_back_then_jamie_pays_71_20_then_undo(store):
    cs, _, _ = build_v1_v2(store)
    view = changesets.approve(store, cs, version=2)
    assert view["status"] == "live"
    assert view["verification"]["all_match"] is True
    assert [c["detail"] for c in view["verification"]["changes"]] == [
        "40 of 40 sale prices match", "VIPWEEKEND20 issued to 1,284 members",
        "“20% off summer outerwear” for vip, 2026-11-07T00:00 – 2026-11-08T23:59"]
    assert view["after_approval"]["undo_until"] == "2026-11-09T23:59"

    # Friday: the sale has not started.
    cmp = call(store, "compare_products", {"product_ids": ["prd_windbreaker"] * 2, "attributes": ["price"]}, context=JAMIE)
    assert cmp["rows"][0]["values"][0]["now"] == 89.00

    # Saturday: Jamie compares and buys.
    store.set_clock("2026-11-07T10:00")
    cmp = call(store, "compare_products", {"product_ids": ["prd_windbreaker", "prd_field_jacket", "prd_overshirt"],
                                           "attributes": ["packed", "weight", "price"], "sort": "weight"}, context=JAMIE)
    assert [r["values"] for r in cmp["rows"] if r["label"] == "Weight, M"] == [["190 g", "340 g", "360 g"]]
    assert [v["now"] for v in cmp["rows"][-1]["values"]] == [71.20, 102.40, 67.15]
    assert cmp["rows"][-1]["values"][0]["note"] == "VIP weekend, 20%"

    call(store, "add_to_cart", {"product_id": "prd_windbreaker", "size": "M"}, context=JAMIE)
    order = call(store, "prepare_order", {}, context=JAMIE)
    assert (order["subtotal"], order["benefit"]["amount"], order["shipping"], order["total"]) == (89.00, -17.80, 0.0, 71.20)
    assert order["benefit"]["label"] == "VIP weekend" and order["benefit"]["coupon"] == "VIPWEEKEND20"
    assert order["ship_to"] == "Home · 12 Harbor St, Apt 3"
    assert order["arrives"] == "Tue, Nov 10" and order["pay_with"] == "Card ending 4821"
    placed = place_order(store, order["order_preview_id"], "mem_jamie", JAMIE)
    assert placed["total"] == 71.20 and placed["coupon"] == "VIPWEEKEND20"

    # A shopper who is not VIP pays the list price.
    mina = {"shopper": True, "member_id": None, "tier": None, "guest_id": "mina"}
    cmp = call(store, "compare_products", {"product_ids": ["prd_windbreaker", "prd_overshirt"], "attributes": ["price"]}, context=mina)
    assert [v["now"] for v in cmp["rows"][0]["values"]] == [89.00, 79.00]

    # Undo together restores list prices for VIPs, removes the coupon and the banner.
    view = changesets.undo(store, cs)
    assert view["status"] == "undone"
    cmp = call(store, "compare_products", {"product_ids": ["prd_windbreaker", "prd_overshirt"], "attributes": ["price"]}, context=JAMIE)
    assert [v["now"] for v in cmp["rows"][0]["values"]] == [89.00, 79.00]
    assert store.one("SELECT status FROM coupons WHERE code = 'VIPWEEKEND20'")["status"] == "undone"
    assert store.one("SELECT COUNT(*) AS n FROM displays WHERE status = 'live' AND audience = 'vip'")["n"] == 0


def test_undo_window_closes(store):
    cs, _, _ = build_v1_v2(store)
    changesets.approve(store, cs, version=2)
    store.set_clock("2026-11-10T00:00")
    with pytest.raises(ToolError) as e:
        changesets.undo(store, cs)
    assert e.value.code == "undo_expired"
