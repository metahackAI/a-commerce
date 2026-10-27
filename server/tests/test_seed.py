"""The seed matches docs/plan.md §4."""

from a_commerce import call


def test_summer_outerwear_has_42_products_and_two_under_10_units(store):
    all_ = call(store, "search_products", {"category": "summer_outerwear", "limit": 100})
    assert all_["total"] == 42
    low = call(store, "search_products", {"category": "summer_outerwear", "max_stock": 9, "fields": ["name", "stock"]})
    assert sorted((i["name"], i["stock"]) for i in low["items"]) == [("Cotton chore coat", 7), ("Light rain parka", 4)]


def test_named_products(store):
    p = {i["id"]: i for i in call(store, "search_products", {"category": "summer_outerwear", "limit": 100})["items"]}
    assert (p["prd_field_jacket"]["price"], p["prd_field_jacket"]["stock"]) == (128.00, 86)
    assert (p["prd_windbreaker"]["price"], p["prd_windbreaker"]["stock"]) == (89.00, 214)
    assert (p["prd_overshirt"]["price"], p["prd_overshirt"]["stock"], p["prd_overshirt"]["cost"]) == (79.00, 53, 56.00)
    assert (p["prd_utility_jacket"]["price"], p["prd_utility_jacket"]["stock"]) == (118.00, 31)
    assert p["prd_chore_coat"]["price"] == 142.00


def test_vip_tier_has_1284_members(store):
    tiers = {t["id"]: t["members"] for t in call(store, "list_tiers")["items"]}
    assert tiers["vip"] == 1284


def test_four_running_promotions_on_the_weekend(store):
    res = call(store, "list_active_promotions", {"starts_at": "2026-11-07T00:00", "ends_at": "2026-11-08T23:59"})
    running = [p["name"] for p in res["items"] if p["state"] == "running"]
    assert sorted(running) == ["Autumn knitwear 15%", "Birthday coupon", "Free shipping over $80", "Gold tier 10%"]


def test_policy_is_data(store):
    res = call(store, "get_policy", {"name": "max_discount"})
    assert res["items"][0]["value"]["vip"] == 30
