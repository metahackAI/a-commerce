"""From an answer to an action: each next step of the sales answer becomes a change set a person approves."""

import pytest
from fastapi.testclient import TestClient

from a_commerce import call, changesets
from a_commerce.api import app
from a_commerce.tools.base import ToolError

client = TestClient(app)


def run(step):
    out = client.post("/bo/chat", json={"request": step}).json()
    assert out["change_set_id"], out
    return out


def test_every_next_step_becomes_an_approvable_change_set():
    client.post("/demo/reset")
    client.post("/demo/clock", json={"at": "2026-11-06T11:20"})
    answer = client.post("/bo/chat", json={"request": "Why did sales drop yesterday?"}).json()
    steps = [a["label"] for a in answer["events"][1]["actions"]]
    assert steps == ["Reorder M and L for the 3 products", "Send the weekly email today", "Show “back Nov 12” on the 3 product pages"]

    reorder = run(steps[0])
    view = reorder["events"][1]["data"]
    assert view["groups"][0]["title"] == "Purchase order" and len(view["groups"][0]["fields"]) == 6
    assert reorder["events"][0]["text"].startswith("Reorder ") and "corrected" not in reorder["events"][0]
    live = client.post(f"/change-sets/{view['id']}/approve", json={"version": 1}).json()
    assert live["status"] == "live" and live["verification"]["all_match"]
    assert client.post(f"/change-sets/{view['id']}/undo").json()["status"] == "undone"  # a PO can be cancelled

    email = run(steps[1])["events"][1]["data"]
    assert email["groups"][0]["fields"]["Undo"] == "Not possible once sent"
    live = client.post(f"/change-sets/{email['id']}/approve", json={"version": 1}).json()
    assert live["verification"]["changes"][0]["detail"] == "Weekly email sent 2026-11-06"
    r = client.post(f"/change-sets/{email['id']}/undo")
    assert r.status_code == 409 and r.json()["detail"]["code"] == "already_sent"

    notice = run(steps[2])["events"][1]["data"]
    assert list(notice["groups"][0]["fields"].values()) == ["“M and L back Nov 12”"] * 3
    assert client.post(f"/change-sets/{notice['id']}/approve", json={"version": 1}).json()["verification"]["all_match"]


def test_sending_the_same_email_twice_is_flagged(store):
    store.set_clock("2026-11-06T11:25")
    first = call(store, "send_campaign", {"name": "Weekly email"})
    cs = changesets.create(store, "Send", "Send the weekly email today", [first["preview_id"]])
    changesets.approve(store, cs, version=1)
    again = call(store, "send_campaign", {"name": "Weekly email"})
    assert next(c for c in again["checks"] if c["rule"] == "not_sent_today")["passed"] is False
    cs2 = changesets.create(store, "Send again", "again", [again["preview_id"]])
    with pytest.raises(ToolError) as e:
        changesets.approve(store, cs2, version=1)
    assert e.value.code == "checks_failing"
