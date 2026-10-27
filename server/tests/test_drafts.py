"""New products from a supplier sheet: codes into names, prices from the rule, rows that need a person (intent-listing)."""

from fastapi.testclient import TestClient

from a_commerce import call
from a_commerce.api import app
from a_commerce.tools.drafts import price_from_rule

client = TestClient(app)


def test_pricing_rule_matches_the_plan(store):
    assert [price_from_rule(store, c) for c in (96, 38, 30, 52, 34, 44)] == [239.0, 99.0, 79.0, 129.0, 89.0, 109.0]


def test_codes_become_names_and_four_rows_need_a_person(store):
    sheet = call(store, "read_supplier_sheet", {})
    assert (sheet["rows"], sheet["ready"], sheet["needs_you"]) == (30, 26, 4)
    first = sheet["items"][:5]
    assert [(d["name"], d["category"], d["price"], d["sizes"]) for d in first] == [
        ("Wool blend overcoat, charcoal", "coats", 239.0, "S–XL"), ("Merino crew-neck sweater, navy", "knitwear", 99.0, "XS–XL"),
        ("Flannel overshirt, check", "shirts", 79.0, "S–XL"), ("Quilted liner jacket, olive", "coats", 129.0, "S–XL"),
        ("Corduroy trousers, sand", "trousers", 89.0, "28–36")]
    assert [d["problem"] for d in sheet["items"] if d["problem"]] == [
        "Fill % missing in the sheet", "Photo shows two items, pick one", "Fill % missing in the sheet", "Photo shows two items, pick one"]


def test_from_the_request_to_saved_drafts_that_shoppers_cannot_see():
    client.post("/demo/reset")
    out = client.post("/bo/chat", json={"request": "List the 30 products in this supplier sheet. Use our usual pricing and write the descriptions in our tone."}).json()
    step, block = out["events"]
    assert block["ui"] == "product_drafts" and "corrected" not in step
    view = block["data"]
    assert (view["rows"], view["ready"], view["needs_you"], view["approve_label"]) == (30, 26, 4, "Save 26 ready as drafts")
    assert {c["rule"]: c["passed"] for c in view["checks"]} == {"no_duplicates": True, "above_floor": True, "required_fields": True}
    live = client.post(f"/change-sets/{view['id']}/approve", json={"version": 1}).json()
    assert live["status"] == "live" and live["verification"]["changes"][0]["detail"] == "26 drafts saved, not visible to shoppers"
    found = client.post("/tools/search_products", json={"args": {"query": "overcoat"}}, headers={"X-Shopper": "jamie"}).json()
    assert [i["name"] for i in found["items"]] == ["Wool overcoat"]  # the draft "Wool blend overcoat" is not in the store
    assert client.post(f"/change-sets/{view['id']}/undo").json()["status"] == "undone"
