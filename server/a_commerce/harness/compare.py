"""Same model, same tasks, two tool layers: the A-Commerce tools ("acommerce") vs the screen-shaped facade (design.md §7).

`make compare RUNS=3` with a model in .env. Tasks start from a fresh seed at Fri Nov 6 11:02. Five of the six tasks are
not the demo sentence; all six were written by the person who built the tools. The arms differ in more than API shape;
eval/README.md lists how. Results are printed and saved to eval/results with the model settings.
"""

from __future__ import annotations

import argparse
import json
import sys
import time
from pathlib import Path

from .. import changesets
from ..seed import fresh_store
from . import operator_models, run_operator

RESULTS = Path(__file__).resolve().parents[3] / "eval" / "results"

TASKS = [
    {"id": "vip_weekend", "request": "Run a 20% weekend sale on summer outerwear for VIP members and put it on the main page.",
     "category": "summer_outerwear", "pct": "20%", "audience": "VIP tier", "when": "Nov 7–8", "banner": True},
    {"id": "gold_next_weekend", "request": "Give Gold members 10% off all summer outerwear next weekend. No banner.",
     "category": "summer_outerwear", "pct": "10%", "audience": "Gold tier", "when": "Nov 14–15", "banner": False},
    {"id": "everyone_shirts", "request": "Put shirts on a 15% sale for everyone this weekend.",
     "category": "shirts", "pct": "15%", "audience": "Everyone tier", "when": "Nov 7–8", "banner": False},
    # Added Oct 28 after the first comparison, so they were not used to tune the tools.
    {"id": "vip_without_parka", "request": "VIP members get 15% off summer outerwear this weekend, except the rain parka.",
     "category": "summer_outerwear", "pct": "15%", "audience": "VIP tier", "when": "Nov 7–8", "banner": False,
     "count": 41, "left_out": ["Light rain parka"]},
    {"id": "gold_shirts_tomorrow", "request": "Gold members: 10% off shirts tomorrow only.",
     "category": "shirts", "pct": "10%", "audience": "Gold tier", "when": "Nov 7–7", "banner": False},
    {"id": "everyone_banner", "request": "Everyone gets 10% off summer outerwear next weekend. Put it on the main page too.",
     "category": "summer_outerwear", "pct": "10%", "audience": "Everyone tier", "when": "Nov 14–15", "banner": True},
]


def judge(store, view: dict | None, task: dict) -> list[tuple[str, bool]]:
    """Six criteria per task, all stated in the request. A coupon is not scored: only plan_sale knows that tier sales
    get one, and the facade arm cannot see that rule."""
    n = task.get("count") or store.one("SELECT COUNT(*) AS n FROM products WHERE category_id = ?", (task["category"],))["n"]
    names_ = ["change set proposed",
              f"{n} products in {task['category']}" + (f" without {', '.join(task['left_out'])}" if task.get("left_out") else ""),
              f"at {task['pct']}", f"for {task['audience']}", f"on {task['when']}",
              "banner " + ("prepared" if task["banner"] else "not made")]
    if view is None:
        return [(name, False) for name in names_]
    groups = {g["op"]: g for g in view.get("groups", [])}
    rows = groups.get("update", {}).get("rows", [])
    understood = {u["label"]: u for u in view.get("understood", [])}
    names = {r["name"] for r in rows}
    return list(zip(names_, [
        True,
        groups.get("update", {}).get("total_rows") == n and not names & set(task.get("left_out", [])),
        bool(rows) and all(r["discount"] == task["pct"] for r in rows),
        understood.get("Who", {}).get("value") == task["audience"],
        understood.get("When", {}).get("value") == task["when"],
        ("schedule" in groups) == task["banner"],
    ]))


def run(task: dict, toolset: str) -> dict:
    store = fresh_store()
    store.set_clock("2026-11-06T11:02")
    worker, _ = operator_models(store)
    start = time.monotonic()
    out = run_operator(store, task["request"], worker, toolset=toolset)
    view = changesets.view(store, out["change_set_id"]) if out.get("change_set_id") else None
    criteria = judge(store, view, task)
    step = next((e for e in out["events"] if e["type"] == "agent_step"), {})
    return {"task": task["id"], "toolset": toolset, "model": worker.name, "settings": getattr(worker, "settings", {}),
            "passed": all(ok for _, ok in criteria),
            "criteria": criteria, "tool_calls": step.get("tool_calls", 0),
            "errors": sum(1 for c in step.get("calls", []) if c.get("error")),
            "seconds": round(time.monotonic() - start, 1), "calls": step.get("calls", [])}


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--runs", type=int, default=1)
    args = ap.parse_args(argv)
    results = []
    for toolset in ("acommerce", "facade"):
        for task in TASKS:
            for _ in range(args.runs):
                r = run(task, toolset)
                results.append(r)
                failed = [name for name, ok in r["criteria"] if not ok]
                print(f"{toolset:9} {task['id']:20} {'PASS' if r['passed'] else 'FAIL'}  calls={r['tool_calls']:2} "
                      f"errors={r['errors']}  {r['seconds']}s  {'; '.join(failed)}")
    print()
    print(f"{'tool layer':10} {'tasks passed':>13} {'criteria met':>13} {'calls/task':>11} {'errors/task':>12} {'s/task':>7}")
    for toolset in ("acommerce", "facade"):
        rs = [r for r in results if r["toolset"] == toolset]
        met = sum(ok for r in rs for _, ok in r["criteria"])
        total = sum(len(r["criteria"]) for r in rs)
        print(f"{toolset:10} {sum(r['passed'] for r in rs):>6}/{len(rs):<6} {met:>6}/{total:<6} "
              f"{sum(r['tool_calls'] for r in rs) / len(rs):>11.1f} {sum(r['errors'] for r in rs) / len(rs):>12.1f} "
              f"{sum(r['seconds'] for r in rs) / len(rs):>7.1f}")
    if results and results[0]["model"] != "scripted":
        RESULTS.mkdir(parents=True, exist_ok=True)
        path = RESULTS / f"{time.strftime('%Y%m%d-%H%M%S')}-compare-{results[0]['model'].replace(':', '_').replace('/', '_')}.json"
        path.write_text(json.dumps(results, indent=1, default=str))
        print(f"saved {path.relative_to(RESULTS.parents[1])}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
