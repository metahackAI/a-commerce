"""The back office home lists open change sets, waiting returns the agent noticed, and the seeded morning."""

from fastapi.testclient import TestClient

from a_commerce.api import app

client = TestClient(app)


def test_task_list_follows_the_store():
    client.post("/demo/reset")
    tasks = client.get("/bo/tasks").json()
    returns = next(t for t in tasks["items"] if t["title"] == "11 return requests over $200")
    assert returns["state"] == "needs_you" and returns["action"] == "Check returns waiting over $200"
    assert tasks["counts"] == {"needs_you": 2, "running": 2, "done": 3}

    client.post("/demo/clock", json={"at": "2026-11-06T11:02"})
    first = client.post("/bo/chat", json={"request": "Run a 20% weekend sale on summer outerwear for VIP members and put it on the main page."}).json()
    client.post("/bo/chat", json={"request": "Drop the low-stock ones. Give the overshirt 15% instead.", "change_set_id": first["change_set_id"]})
    sale = client.get("/bo/tasks").json()["items"][0]
    assert (sale["change_set_id"], sale["title"], sale["state"], sale["status_label"], sale["checks"]) == (
        1042, "VIP weekend sale: summer outerwear", "needs_you", "Ready to review", "6/6 checks")
    assert sale["diff"] == ["~40 prices", "+1 coupon", "+1 banner", "2 left out"]

    client.post("/change-sets/1042/approve", json={"version": 2})
    sale = next(t for t in client.get("/bo/tasks").json()["items"] if t.get("change_set_id") == 1042)
    assert (sale["state"], sale["status_label"]) == ("done", "Live · verified")

    review = client.post("/bo/chat", json={"request": "Check returns waiting over $200"}).json()
    items = client.get("/bo/tasks").json()["items"]
    assert not any(t["title"].endswith("return requests over $200") for t in items)
    row = next(t for t in items if t.get("change_set_id") == review["change_set_id"])
    assert row["diff"] == ["8 refunds · $2,151.00", "3 messages"] and row["checks"] == "3/4 checks"
