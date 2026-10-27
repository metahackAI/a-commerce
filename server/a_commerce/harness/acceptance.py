"""The demo's definition of done, usable against any model: `make try-model`.

Runs the two operator messages through the configured model and judges the change set against
docs/plan.md §4.6. Exit code 0 only when every criterion passes.
"""

from __future__ import annotations

import sys

from .. import changesets
from ..seed import fresh_store
from . import operator_models, run_operator

REQUEST_1 = "Run a 20% weekend sale on summer outerwear for VIP members and put it on the main page."
REQUEST_2 = "Drop the low-stock ones. Give the overshirt 15% instead."


def judge(view: dict) -> list[tuple[str, bool]]:
    groups = {g["op"]: g for g in view["groups"]}
    rows = {r["name"]: r for r in groups.get("update", {}).get("rows", [])}
    fields = groups.get("create", {}).get("fields", {})
    banner = groups.get("schedule", {}).get("preview", {})
    return [
        ("40 products on sale", groups.get("update", {}).get("total_rows") == 40),
        ("chore coat and rain parka left out", "Cotton chore coat" not in rows and "Light rain parka" not in rows),
        ("overshirt at 15% = $67.15", rows.get("Seersucker overshirt", {}).get("now") == 67.15),
        ("windbreaker at 20% = $71.20", rows.get("Packable windbreaker", {}).get("now") == 71.20),
        ("coupon to 1,284 VIP members", fields.get("Issued to") == "1,284 VIP members"),
        ("hero banner for VIP Nov 7–8", banner.get("audiences", [None])[0] == "VIP members" and "Nov 7" in banner.get("date", "")),
        ("6 of 6 checks pass", view["checks_passed"] == view["checks_total"] == 6),
    ]


def main() -> int:
    store = fresh_store()
    worker, fallback = operator_models(store)
    print(f"worker: {worker.name}  fallback: {fallback.name if fallback else '-'}")
    store.set_clock("2026-11-06T11:02")
    first = run_operator(store, REQUEST_1, worker, fallback=fallback)
    if not first["ok"]:
        print("FAIL: no change set from the first message:", first.get("error"))
        return 1
    store.set_clock("2026-11-06T11:05")
    second = run_operator(store, REQUEST_2, worker, change_set_id=first["change_set_id"], fallback=fallback)
    for e in first["events"] + second["events"]:
        if e["type"] == "agent_step":
            print(f"[{e['model']}] {e['tool_calls']} tool calls, {e['duration_ms']} ms: {e['text']}")
    results = judge(changesets.view(store, first["change_set_id"]))
    for name, ok in results:
        print(("PASS " if ok else "FAIL ") + name)
    return 0 if all(ok for _, ok in results) else 1


if __name__ == "__main__":
    sys.exit(main())
