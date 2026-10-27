"""Fit check and gift picker (design/storefront fo-fit, fo-gift) over HTTP, as the storefront drives them."""

from fastapi.testclient import TestClient

from a_commerce.api import app

client = TestClient(app)


def fo(shopper, message):
    events = client.post("/fo/chat", json={"message": message}, headers={"X-Shopper": shopper}).json()["events"]
    return events[0], {e["ui"]: e for e in events if e["type"] == "ui"}


def buy_windbreaker_on_saturday():
    client.post("/demo/clock", json={"at": "2026-11-07T10:00"})
    fo("jamie", "I need a light jacket for spring trips.")
    _, ui = fo("jamie", "The windbreaker. Same size as last time.")
    client.post("/orders", json={"order_preview_id": ui["cart"]["data"]["order_preview_id"]}, headers={"X-Shopper": "jamie"})


def test_fit_compares_with_the_windbreaker_once_it_has_arrived():
    client.post("/demo/reset")
    buy_windbreaker_on_saturday()
    client.post("/demo/clock", json={"at": "2026-11-20T10:00"})
    step, ui = fo("jamie", "Will M fit in the wool overcoat? I want to wear it over a hoodie.")
    assert step["text"] == ("Probably not over a hoodie. M is about the size of your windbreaker, which you kept, but over a hoodie "
                            "you'll want roughly 3 cm more in the chest. I'd go with L.")
    assert ui["memory_note"]["data"]["text"] == "Remembered: you layer hoodies under coats"
    fit = ui["fit_check"]["data"]
    assert [(r["wearing"], r["size"]) for r in fit["recommendations"]] == [("Over a hoodie", "L"), ("Over a shirt or sweater", "M")]
    assert fit["columns"] == ["This overcoat, M", "This overcoat, L", "Your windbreaker, M"]
    assert fit["rows"][0]["values"] == ["56 cm", "59 cm", "57 cm"]
    assert fit["used"][0] == "Your orders: Windbreaker M, kept" and fit["not_used"] == ["Height and weight (you haven't shared them)"]
    assert (fit["reviews"]["people"], fit["reviews"]["true_to_size"], fit["reviews"]["phrases"]) == (41, 29, {"size up for a hoodie": 9})
    step, _ = fo("jamie", "What if I just wear a sweater?")
    assert step["text"] == "Then M. I put both on the right so you can see the difference."


def test_before_anything_arrived_it_does_not_claim_an_item_you_kept():
    client.post("/demo/reset")
    client.post("/demo/clock", json={"at": "2026-11-07T10:00"})
    step, ui = fo("jamie", "Will M fit in the wool overcoat? I want to wear it over a hoodie.")
    assert "which you kept" not in step["text"] and "your usual size" in step["text"]
    assert ui["fit_check"]["data"]["columns"] == ["This overcoat, M", "This overcoat, L"]


def test_gift_for_dad_saves_nothing_and_the_address_is_used_once():
    client.post("/demo/reset")
    client.post("/demo/clock", json={"at": "2026-11-06T11:02"})
    first = client.post("/bo/chat", json={"request": "Run a 20% weekend sale on summer outerwear for VIP members and put it on the main page."}).json()
    client.post("/bo/chat", json={"request": "Drop the low-stock ones. Give the overshirt 15% instead.", "change_set_id": first["change_set_id"]})
    client.post(f"/change-sets/{first['change_set_id']}/approve", json={"version": 2})
    client.post("/demo/clock", json={"at": "2026-11-07T10:00"})
    before = client.get("/fo/profile", headers={"X-Shopper": "jamie"}).json()["preferences"]

    step, ui = fo("jamie", "A gift for my dad. He hikes a lot. Under $100, and it has to arrive by Nov 14.")
    assert step["text"].startswith("Here are three that hikers keep using. I won't save any of this to your profile")
    gift = ui["gift_picker"]["data"]
    assert [(p["name"], p["price"]["now"]) for p in gift["picks"]] == [
        ("Packable windbreaker", 71.20), ("Fleece half-zip", 84.00), ("Merino trail tee", 58.00)]
    assert gift["delivery"] == {"arrives": "Tue Nov 10", "on_time": True, "note": "Four days early"}
    assert gift["options"]["receipt"] == "Gift receipt, no prices shown" and gift["options"]["exchange"] == "They can swap the size until Dec 31"

    step, ui = fo("jamie", "He's about my size, maybe a bit bigger.")
    gift = ui["gift_picker"]["data"]
    assert (gift["size"], gift["total"], gift["total_detail"]) == ("L", 76.20, "windbreaker $71.20 + wrap $5.00")

    placed = client.post("/gift-orders", json={"product_id": "prd_windbreaker", "size": "L", "wrap": True,
                                               "message": "For the next trail. Love, J.", "recipient": "Dad", "address": "48 Pine Rd, Lakeview"},
                         headers={"X-Shopper": "jamie"}).json()
    assert placed["total"] == 76.20
    assert client.get("/fo/profile", headers={"X-Shopper": "jamie"}).json()["preferences"] == before
    profile = client.get("/fo/profile", headers={"X-Shopper": "jamie"}).json()
    assert "Pine" not in str(profile)
