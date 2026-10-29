"""The demo's definition of done, usable against any model: `make try-model [RUNS=3]`.

Runs the two operator messages through the configured model and judges the change set against
docs/plan.md §4.6. Each run starts from a fresh seed. Every tool call is saved to eval/results so a
failure can be read afterwards. Exit code 0 only when every criterion passes in every run (pass^k).
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

REQUEST_1 = "Run a 20% weekend sale on summer outerwear for VIP members and put it on the main page."
REQUEST_2 = "Drop the low-stock ones. Give the overshirt 15% instead."
RESULTS = Path(__file__).resolve().parents[3] / "eval" / "results"


def judge(view: dict | None) -> list[tuple[str, bool]]:
    view = view or {"groups": [], "checks_passed": 0, "checks_total": 0}
    groups = {g["op"]: g for g in view["groups"]}
    rows = {r["name"]: r for r in groups.get("update", {}).get("rows", [])}
    fields = groups.get("create", {}).get("fields", {})
    banner = groups.get("schedule", {}).get("preview", {})
    return [
        ("40 products on sale", groups.get("update", {}).get("total_rows") == 40),
        ("chore coat and rain parka left out", bool(rows) and "Cotton chore coat" not in rows and "Light rain parka" not in rows),
        ("overshirt at 15% = $67.15", rows.get("Seersucker overshirt", {}).get("now") == 67.15),
        ("windbreaker at 20% = $71.20", rows.get("Packable windbreaker", {}).get("now") == 71.20),
        ("coupon to 1,284 VIP members", fields.get("Issued to") == "1,284 VIP members"),
        ("hero banner for VIP Nov 7–8", banner.get("audiences", [None])[0] == "VIP members" and "Nov 7" in banner.get("date", "")),
        ("6 of 6 checks pass", view["checks_passed"] == view["checks_total"] == 6),
    ]


def judge_v1(view: dict | None) -> list[tuple[str, bool]]:
    """After the first message: every product at 20%, coupon and banner prepared, and the two problems flagged."""
    view = view or {"groups": [], "checks": []}
    groups = {g["op"]: g for g in view["groups"]}
    rows = {r["name"]: r for r in groups.get("update", {}).get("rows", [])}
    failing = {c["rule"] for c in view["checks"] if not c["passed"]}
    return [
        ("v1: 42 products at 20%", groups.get("update", {}).get("total_rows") == 42
         and all(r["discount"] == "20%" for r in rows.values())),
        ("v1: coupon and VIP banner prepared", "create" in groups and "schedule" in groups),
        ("v1: flags low stock and the overshirt floor", failing == {"min_stock", "price_floor"}),
    ]


def run_once() -> dict:
    store = fresh_store()
    worker, fallback = operator_models(store)
    start = time.monotonic()
    store.set_clock("2026-11-06T11:02")
    first = run_operator(store, REQUEST_1, worker, fallback=fallback)
    v1 = judge_v1(changesets.view(store, first["change_set_id"]) if first["ok"] else None)
    second = None
    if first["ok"]:
        store.set_clock("2026-11-06T11:05")
        second = run_operator(store, REQUEST_2, worker, change_set_id=first["change_set_id"], fallback=fallback)
    view = changesets.view(store, first["change_set_id"]) if first["ok"] else None
    results = judge(view)
    v1_passed = all(ok for _, ok in v1)
    steps = [e for r in (first, second) if r for e in r["events"] if e["type"] in ("agent_step", "handoff")]
    return {"worker": worker.name, "fallback": fallback.name if fallback else None, "seconds": round(time.monotonic() - start, 1),
            "passed": all(ok for _, ok in results), "v1_passed": v1_passed, "criteria": v1 + results, "steps": steps,
            "error": None if first["ok"] else first.get("error")}


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--runs", type=int, default=1)
    ap.add_argument("--allow-scripted", action="store_true", help="Run the rule-based stand-in (tests only; not a model run).")
    args = ap.parse_args(argv)
    if operator_models(fresh_store())[0].name == "scripted" and not args.allow_scripted:
        print("No model configured (WORKER_* or PLANNER_* in .env), so only the scripted stand-in could run. "
              "That is not a model run. Set a model, or pass --allow-scripted to test the judge.")
        return 2
    runs = []
    for i in range(args.runs):
        r = run_once()
        runs.append(r)
        print(f"run {i + 1}/{args.runs}  worker={r['worker']}  {'PASS' if r['passed'] else 'FAIL'}  {r['seconds']}s")
        for s in r["steps"]:
            if s["type"] == "handoff":
                print(f"  handoff {s['from']} → {s['to']}: {s['reason']}")
                continue
            print(f"  [{s['model']}] {s['tool_calls']} tool calls: {s['text'][:160]}")
            for c in s.get("calls", []):
                if c.get("error"):
                    print(f"    ! {c['tool']}: {c['error']['code']}: {c['error']['message'][:120]}")
        for name, ok in r["criteria"]:
            print(("    PASS " if ok else "    FAIL ") + name)
    passed = sum(r["passed"] for r in runs)
    print(f"first message (v1): {sum(r['v1_passed'] for r in runs)}/{args.runs} runs right")
    print(f"pass^{args.runs}: {passed}/{args.runs} runs passed every criterion")
    if runs and runs[0]["worker"] != "scripted":
        RESULTS.mkdir(parents=True, exist_ok=True)
        out = RESULTS / f"{time.strftime('%Y%m%d-%H%M%S')}-{runs[0]['worker'].replace('/', '_').replace(':', '_')}.json"
        out.write_text(json.dumps(runs, indent=1, default=str))
        print(f"saved {out.relative_to(RESULTS.parents[1])}")
    return 0 if passed == len(runs) else 1


if __name__ == "__main__":
    sys.exit(main())
