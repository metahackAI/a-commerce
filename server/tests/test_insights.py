"""Answer a question: "Why did sales drop yesterday?" is read only, and its numbers come from the data (plan §4.10)."""

from fastapi.testclient import TestClient

from a_commerce import call
from a_commerce.api import app
from a_commerce.harness.grounding import check_sales_reply

client = TestClient(app)


def test_the_explanation_adds_up(store):
    store.set_clock("2026-11-06T11:20")
    d = call(store, "explain_sales_change", {})
    assert (d["day"], d["revenue"], d["orders"], d["usual_revenue"], d["usual_orders"], d["difference"], d["difference_pct"]) == (
        "2026-11-05", 18240.0, 201, 22900.0, 247, -4660.0, -20)
    assert [(c["cause"], c["estimated"]) for c in d["causes"]] == [
        ("3 bestsellers out of M and L", -2700.0), ("Weekly email not sent", -1300.0), ("Not explained", -660.0)]
    assert sum(c["estimated"] for c in d["causes"]) == d["difference"]
    assert d["causes"][1]["detail"] == "List import failed Thu 08:00"
    assert d["next_steps"] == ["Reorder M and L for the 3 products", "Send the weekly email today",
                               "Show “back Nov 12” on the 3 product pages"]
    assert len(d["history"]) == 14 and d["history"][-1]["day"] == "2026-11-05"


def test_asking_changes_nothing_and_comes_back_as_a_read_only_block():
    client.post("/demo/reset")
    client.post("/demo/clock", json={"at": "2026-11-06T11:20"})
    before = client.get("/bo/tasks").json()
    out = client.post("/bo/chat", json={"request": "Why did sales drop yesterday?"}).json()
    step, block = out["events"]
    assert out["change_set_id"] is None and step["read_only"] and "corrected" not in step
    assert step["text"].startswith("Revenue was $18,240, 20% under a normal Thursday.")
    assert "cover about $4,000 of the $4,660" in step["text"]
    assert block["ui"] == "sales_explainer" and [a["label"] for a in block["actions"]][1] == "Send the weekly email today"
    assert client.get("/bo/tasks").json() == before


def test_made_up_numbers_are_caught(store):
    store.set_clock("2026-11-06T11:20")
    d = call(store, "explain_sales_change", {})
    assert check_sales_reply("Revenue was $18,240, 20% under. Stockouts cost about $2,700 and the email $1,300.", d) == []
    assert check_sales_reply("Revenue fell to $17,900, 25% down, mostly a $3,500 stockout.", d) == [
        "It said $17,900, which is not in the sales data.", "It said $3,500, which is not in the sales data.",
        "It said 25% down, but the data says 20%."]
