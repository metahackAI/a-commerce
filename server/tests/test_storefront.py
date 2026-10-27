"""Storefront story from docs/plan.md §4.7 over HTTP: Jamie compares and buys at the VIP price; Mina asks for photos."""

from fastapi.testclient import TestClient

from a_commerce.api import app

client = TestClient(app)
WEEKEND = {"starts_at": "2026-11-07T00:00", "ends_at": "2026-11-08T23:59"}


def fo(shopper, message):
    r = client.post("/fo/chat", json={"message": message}, headers={"X-Shopper": shopper})
    assert r.status_code == 200, r.text
    events = r.json()["events"]
    return events[0], {e["ui"]: e for e in events if e["type"] == "ui"}


def approve_vip_weekend():
    client.post("/demo/clock", json={"at": "2026-11-06T11:02"})
    first = client.post("/bo/chat", json={"request": "Run a 20% weekend sale on summer outerwear for VIP members and put it on the main page."}).json()
    client.post("/bo/chat", json={"request": "Drop the low-stock ones. Give the overshirt 15% instead.", "change_set_id": first["change_set_id"]})
    assert client.post(f"/change-sets/{first['change_set_id']}/approve", json={"version": 2}).json()["status"] == "live"


def setup_function(_):
    client.post("/demo/reset")


def test_jamie_compares_remembers_and_buys_at_the_vip_price(monkeypatch):
    for k in ("WORKER_BASE_URL", "PLANNER_BASE_URL"):
        monkeypatch.delenv(k, raising=False)
    approve_vip_weekend()
    client.post("/demo/clock", json={"at": "2026-11-07T10:00"})

    step, ui = fo("jamie", "I need a light jacket for spring trips. Not too warm, and it has to fit in a carry-on.")
    assert step["model"] == "scripted"
    assert set(ui) == {"understood_chips", "compare_table"}
    assert ui["compare_table"]["why"].startswith("Shown as a table because you compare specs")
    chips = {c["label"]: c["source"] for c in ui["understood_chips"]["data"]["chips"]}
    assert chips == {"Light jacket": "conversation", "Spring travel": "conversation", "Packs small": "conversation",
                     "Size M": "profile", "Natural fabrics": "profile"}
    table = ui["compare_table"]["data"]
    assert [p["name"] for p in table["products"]] == ["Packable windbreaker", "Linen blend field jacket", "Seersucker overshirt"]
    prices = next(r for r in table["rows"] if r["label"] == "Your price")["values"]
    assert [(p["now"], p["was"], p["note"]) for p in prices] == [
        (71.20, 89.00, "VIP weekend, 20%"), (102.40, 128.00, "VIP weekend, 20%"), (67.15, 79.00, "VIP weekend, 15%")]

    step, ui = fo("jamie", "Which one packs smallest? I hate bulky bags.")
    assert ui["memory_note"]["data"]["text"] == "Remembered: weight and packed size matter to you when traveling"
    assert next(r for r in ui["compare_table"]["data"]["rows"] if r["label"] == "Packed")["values"][0] == "Into its own pocket, 14 × 18 cm"

    step, ui = fo("jamie", "The windbreaker. Same size as last time.")
    assert step["text"] == "Added the windbreaker in M, slate. Your VIP weekend price is applied. Take a look at the cart when you're ready."
    cart = ui["cart"]["data"]
    assert (cart["subtotal"], cart["benefit"]["amount"], cart["shipping"], cart["total"]) == (89.00, -17.80, 0.0, 71.20)
    assert cart["benefit"]["coupon"] == "VIPWEEKEND20"
    assert ui["cart"]["actions"][0]["by"] == "shopper"

    # The agent prepared it; nothing is ordered until Jamie taps Place order.
    assert "place_order" not in [a["action"] for a in client.get("/actions", params={"limit": 200}).json()]
    placed = client.post("/orders", json={"order_preview_id": cart["order_preview_id"]}, headers={"X-Shopper": "jamie"})
    assert placed.status_code == 200 and placed.json()["total"] == 71.20

    profile = client.get("/fo/profile", headers={"X-Shopper": "jamie"}).json()
    assert "Weight and packed size matter to you when traveling" in [p["value"] for p in profile["preferences"]]


def test_mina_gets_a_list_then_asks_for_photos_and_can_undo_the_memory():
    step, ui = fo("mina", "I need a light jacket for spring trips.")
    assert "product_list" in ui and "compare_table" not in ui
    items = ui["product_list"]["data"]["items"]
    assert [(i["name"], i["price"]["now"]) for i in items] == [
        ("Packable windbreaker", 89.00), ("Linen blend field jacket", 128.00), ("Seersucker overshirt", 79.00)]
    assert items[0]["facts"][:3] == ["190 g", "into its own pocket, 14 × 18 cm", "light rain"]

    step, ui = fo("mina", "Less text. Just show me bigger photos.")
    assert [i["product_id"] for i in ui["look_gallery"]["data"]["items"]] == [i["product_id"] for i in items]
    note = ui["memory_note"]
    assert note["data"]["text"] == "Remembered: photos over details"
    r = client.post(f"/memories/{note['data']['memory_id']}/undo", headers={"X-Shopper": "mina"})
    assert r.status_code == 200
    assert client.get("/fo/profile", headers={"X-Shopper": "mina"}).json()["preferences"] == []


def test_shoppers_never_see_cost_or_operator_tools():
    for name, args in (("search_products", {"query": "windbreaker"}), ("get_product", {"product_id": "prd_windbreaker"})):
        out = client.post(f"/tools/{name}", json={"args": args}, headers={"X-Shopper": "jamie"}).json()
        text = str(out)
        assert "cost" not in text, name
    names = {d["function"]["name"] for d in client.get("/tools", params={"scope": "shopper"}).json()}
    assert not names & {"set_sale_prices", "create_coupon", "schedule_banner"}
