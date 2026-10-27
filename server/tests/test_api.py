"""The same story over HTTP, the way the web app and the harness will drive it."""

from fastapi.testclient import TestClient

from a_commerce.api import app

client = TestClient(app)
WEEKEND = {"starts_at": "2026-11-07T00:00", "ends_at": "2026-11-08T23:59"}


def tool(name, args=None, shopper=None):
    headers = {"X-Shopper": shopper} if shopper else {}
    r = client.post(f"/tools/{name}", json={"args": args or {}}, headers=headers)
    assert r.status_code == 200, r.text
    return r.json()


def test_tool_definitions_are_complete():
    client.post("/demo/reset")
    defs = client.get("/tools").json()
    names = {d["function"]["name"] for d in defs}
    assert {"search_products", "set_sale_prices", "create_coupon", "schedule_banner", "verify_change_set",
            "compare_products", "add_to_cart", "prepare_order"} <= names
    for d in defs:
        params = d["function"]["parameters"].get("properties", {})
        for field, spec in params.items():
            if field == "preview":
                continue
            assert spec.get("description"), f"{d['function']['name']}.{field} has no description"
    shopper_names = {d["function"]["name"] for d in client.get("/tools", params={"scope": "shopper"}).json()}
    assert "set_sale_prices" not in shopper_names


def test_change_set_over_http():
    client.post("/demo/reset")
    client.post("/demo/clock", json={"at": "2026-11-06T11:05"})
    items = [{"product_id": p["id"], "percent_off": 15 if p["id"] == "prd_overshirt" else 20}
             for p in tool("search_products", {"category": "summer_outerwear", "min_stock": 10, "limit": 100})["items"]]
    sale = tool("set_sale_prices", {"name": "VIP weekend", "audience": "vip", **WEEKEND, "items": items})
    coupon = tool("create_coupon", {"code": "VIPWEEKEND20", "promotion_id": f"ref:{sale['preview_id']}", "audience": "vip"})
    banner = tool("schedule_banner", {"slot_id": "slot_main_hero", "audience": "vip", **WEEKEND,
                                      "headline": "20% off summer outerwear"})
    cs = client.post("/change-sets", json={"title": "VIP weekend sale: summer outerwear", "request": "…",
                                           "preview_ids": [sale["preview_id"], coupon["preview_id"], banner["preview_id"]]}).json()
    assert cs["checks_passed"] == cs["checks_total"] == 6

    bad = client.post(f"/change-sets/{cs['id']}/approve", json={"version": 9})
    assert bad.status_code == 409 and bad.json()["detail"]["code"] == "stale_version"

    live = client.post(f"/change-sets/{cs['id']}/approve", json={"version": cs["version"]}).json()
    assert live["status"] == "live" and live["verification"]["all_match"]

    client.post("/demo/clock", json={"at": "2026-11-07T10:00"})
    tool("add_to_cart", {"product_id": "prd_windbreaker", "size": "M"}, shopper="jamie")
    order = tool("prepare_order", {}, shopper="jamie")
    assert order["total"] == 71.20
    placed = client.post("/orders", json={"order_preview_id": order["order_preview_id"]}, headers={"X-Shopper": "jamie"})
    assert placed.status_code == 200 and placed.json()["total"] == 71.20

    log = [a["action"] for a in client.get("/actions").json()]
    assert "change_set:approve" in log and "place_order" in log


def test_operator_chat_builds_and_edits_the_change_set(monkeypatch):
    for k in ("WORKER_BASE_URL", "PLANNER_BASE_URL"):
        monkeypatch.delenv(k, raising=False)
    client.post("/demo/reset")
    client.post("/demo/clock", json={"at": "2026-11-06T11:02"})
    first = client.post("/bo/chat", json={"request": "Run a 20% weekend sale on summer outerwear for VIP members and put it on the main page."}).json()
    assert first["change_set_id"] == 1042 and first["events"][1]["data"]["checks_passed"] == 4
    second = client.post("/bo/chat", json={"request": "Drop the low-stock ones. Give the overshirt 15% instead.",
                                            "change_set_id": 1042}).json()
    view = second["events"][1]["data"]
    assert (view["version"], view["checks_passed"]) == (2, 6)
    live = client.post("/change-sets/1042/approve", json={"version": 2}).json()
    assert live["status"] == "live"
