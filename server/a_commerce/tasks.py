"""The back office task list (layout E, docs/plan.md §4.8): open change sets, work the agent noticed, and what is done.

Change sets and waiting returns come from the store. The other rows are the seeded morning of Fri Nov 6, so the list
looks like the operator's real day; they are marked `seeded` and do nothing when opened.
"""

from __future__ import annotations

import json

from . import changesets
from .db import Store

SEEDED = [
    {"title": "Linen shirts will sell out in about 4 days", "source": "Agent noticed", "at": "09:12", "state": "needs_you",
     "diff": ["Reorder draft: 3 sizes, 240 units", "61 sold in 7 days, 38 left"], "status_label": "Draft ready"},
    {"title": "List 30 products from the supplier sheet", "source": "You asked", "at": "10:58", "state": "running",
     "diff": ["+18 drafts", "12 to go"], "status_label": "Running · about 2 min"},
    {"title": "Add missing size charts", "source": "Agent noticed", "at": "11:00", "state": "running",
     "diff": ["15 of 18 checked"], "status_label": "Running · no issues so far"},
    {"title": "Fix 4 products listed below cost", "source": "Automation", "at": "08:31", "state": "done",
     "diff": ["4 prices raised to the floor"], "status_label": "Live · read back, all match"},
    {"title": "Swap the main page banner to “Fall layers”", "source": "You asked", "at": "08:10", "state": "done",
     "diff": ["1 display slot"], "status_label": "Live · undo until Nov 7"},
    {"title": "Answer order-status questions", "source": "Shopper agent", "at": "08:02", "state": "done",
     "diff": ["46 answered", "2 handed to you, both closed"], "status_label": "Read only"},
]

STATE = {"waiting_for_approval": "needs_you", "running": "running", "verifying": "running",
         "live": "done", "failed": "needs_you", "undone": "done"}


def task_list(store: Store) -> dict:
    items = []
    for row in store.all("SELECT id, created_at FROM change_sets ORDER BY updated_at DESC"):
        v = changesets.view(store, row["id"])
        requests = json.loads(store.one("SELECT requests FROM change_sets WHERE id = ?", (row["id"],))["requests"])
        if v.get("kind") == "refund_review":
            s = v["summary"]
            diff = [f"{s['refund']['count']} refunds · ${s['refund']['amount']:,.2f}", f"{s['ask_for_photos'] + s['decline']} messages"]
        else:
            groups = {g["op"]: g for g in v["groups"]}
            diff = [f"~{groups['update']['total_rows']} prices"] if "update" in groups else []
            diff += ["+1 coupon"] * ("create" in groups) + ["+1 banner"] * ("schedule" in groups)
            diff += [f"{len(groups['exclude']['rows'])} left out"] if "exclude" in groups else []
        label = {"waiting_for_approval": "Ready to review", "live": "Live · verified", "undone": "Undone",
                 "failed": "Read-back failed"}.get(v["status"], v["status"])
        items.append({"change_set_id": v["id"], "title": v["title"], "source": "You asked" if not requests[0].startswith("Review") else "Agent noticed",
                      "at": row["created_at"][11:16], "state": STATE[v["status"]], "diff": diff,
                      "checks": f"{v['checks_passed']}/{v['checks_total']} checks", "status_label": label,
                      "version": v["version"], "kind": v.get("kind", "change_set")})
    has_review = any(i.get("kind") == "refund_review" for i in items)
    waiting = store.all("SELECT amount FROM return_requests WHERE status = 'waiting' AND amount > ?",
                        (store.policy_value("refund_without_person_max"),))
    if waiting and not has_review:
        items.append({"title": f"{len(waiting)} return requests over $200", "source": "Agent noticed", "at": "10:40",
                      "state": "needs_you", "diff": [f"{len(waiting)} refunds over $200 each"],
                      "status_label": "Refund policy asks for a person", "action": "Check returns waiting over $200"})
    items += [{**s, "seeded": True} for s in SEEDED]
    order = {"needs_you": 0, "running": 1, "done": 2}
    items.sort(key=lambda i: order[i["state"]])
    counts = {k: sum(i["state"] == k for i in items) for k in order}
    return {"items": items, "counts": counts}
