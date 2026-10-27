"""The harness turns the operator's words into change set #1042, with no hand-written tool calls.

Runs with the scripted stand-in (no model key needed) and with a fake OpenAI-compatible server to
prove the real-model path speaks the protocol.
"""

import json

import httpx

from a_commerce import changesets
from a_commerce.harness import ScriptedOperator, run_operator
from a_commerce.harness.models import OpenAICompatible
from a_commerce.harness.search import search

REQUEST_1 = "Run a 20% weekend sale on summer outerwear for VIP members and put it on the main page."
REQUEST_2 = "Drop the low-stock ones. Give the overshirt 15% instead."


def test_tool_search_finds_the_sale_tools_without_showing_everything():
    names = [t.name for t in search(REQUEST_1)]
    assert {"set_sale_prices", "create_coupon", "schedule_banner", "list_display_slots"} <= set(names)
    assert "compare_products" not in names and "add_to_cart" not in names


def test_request_to_v1_then_v2_from_words(store):
    store.set_clock("2026-11-06T11:02")
    model = ScriptedOperator(store)
    first = run_operator(store, REQUEST_1, model)
    assert first["ok"]
    step, block = first["events"]
    assert step["tool_calls"] == 9 and step["model"] == "scripted"
    assert step["text"] == ("I put this together as a change set. Nothing has changed in the store yet. Two things need your "
                            "call: 2 products have fewer than 10 left, and at 20% the Seersucker overshirt would go below its price floor.")
    v1 = block["data"]
    assert (v1["id"], v1["version"], v1["checks_passed"], v1["checks_total"]) == (1042, 1, 4, 6)

    store.set_clock("2026-11-06T11:05")
    second = run_operator(store, REQUEST_2, model, change_set_id=first["change_set_id"])
    step, block = second["events"]
    assert step["text"] == "Done. 40 products now, the overshirt at 15%. All 6 checks pass."
    v2 = block["data"]
    assert (v2["id"], v2["version"], v2["checks_passed"]) == (1042, 2, 6)
    assert v2["request"] == [REQUEST_1, REQUEST_2]
    rows = {r["name"]: r["now"] for r in v2["groups"][0]["rows"]}
    assert (len(rows), rows["Seersucker overshirt"], rows["Packable windbreaker"]) == (40, 67.15, 71.20)
    assert block["actions"][0] == {"intent": "approve_change_set", "label": "Approve and run", "by": "operator",
                                   "args": {"change_set_id": 1042, "version": 2}}

    live = changesets.approve(store, 1042, version=2)
    assert live["status"] == "live" and live["verification"]["all_match"]


def test_the_agent_never_approves(store):
    run_operator(store, REQUEST_1, ScriptedOperator(store))
    assert store.one("SELECT COUNT(*) AS n FROM previews WHERE status = 'committed'")["n"] == 0
    assert store.one("SELECT status FROM change_sets WHERE id = 1042")["status"] == "waiting_for_approval"


def fake_server(script):
    """An OpenAI-compatible endpoint that replays tool calls, to test the real client end to end."""
    turns = iter(script)
    last = [script[-1]]

    def handler(request):
        body = json.loads(request.content)
        assert body["model"] == "muse-glimmer-test" and body["tools"]
        calls = last[0] = next(turns, last[0])  # after the script, keep giving the last answer
        msg = {"role": "assistant", "content": calls if isinstance(calls, str) else None}
        if not isinstance(calls, str):
            msg["tool_calls"] = [{"id": f"c{i}", "type": "function",
                                  "function": {"name": n, "arguments": json.dumps(a)}} for i, (n, a) in enumerate(calls)]
        return httpx.Response(200, json={"choices": [{"message": msg}], "usage": {"total_tokens": 10}})
    return httpx.MockTransport(handler)


def test_openai_compatible_client_drives_the_same_flow(store):
    weekend = {"starts_at": "2026-11-07T00:00", "ends_at": "2026-11-08T23:59"}
    script = [
        [("set_sale_prices", {"name": "VIP weekend", "audience": "vip", **weekend, "category": "summer_outerwear",
                              "percent_off": 20, "min_stock": 10, "overrides": [{"product_id": "prd_overshirt", "percent_off": 15}]})],
        [("create_coupon", {"code": "VIPWEEKEND20", "promotion_id": "ref:pv_1", "audience": "vip"})],
        [("propose_change_set", {"title": "VIP weekend", "preview_ids": ["pv_1", "pv_2"], "reply": "Ready."})],
        "Ready.",
    ]
    model = OpenAICompatible("http://fake/v1", "k", "muse-glimmer-test", transport=fake_server(script))
    out = run_operator(store, REQUEST_1, model)
    assert out["ok"] and out["events"][0]["model"] == "muse-glimmer-test" and out["events"][0]["tool_calls"] == 3
    assert out["events"][1]["data"]["checks_passed"] == 5


def test_a_worker_that_gives_up_hands_over_to_the_planner(store):
    lazy = OpenAICompatible("http://fake/v1", "k", "muse-glimmer-test", transport=fake_server(["I can't do that."]))
    out = run_operator(store, REQUEST_1, lazy, fallback=ScriptedOperator(store))
    assert out["ok"] and out["events"][0]["type"] == "handoff"
    assert out["events"][0]["from"] == "muse-glimmer-test" and out["events"][1]["model"] == "scripted"


def test_acceptance_judge_passes_with_the_scripted_stand_in(monkeypatch):
    for k in ("WORKER_BASE_URL", "PLANNER_BASE_URL"):
        monkeypatch.delenv(k, raising=False)
    from a_commerce.harness.acceptance import main
    assert main([]) == 0


def test_a_model_that_times_out_fails_cleanly_and_hands_over(store):
    def slow(request):
        raise httpx.ReadTimeout("timed out")
    broken = OpenAICompatible("http://fake/v1", "k", "muse-glimmer-test", transport=httpx.MockTransport(slow))
    out = run_operator(store, REQUEST_1, broken)
    assert not out["ok"] and "ReadTimeout" in out["error"]
    out = run_operator(store, REQUEST_1, broken, fallback=ScriptedOperator(store))
    assert out["ok"] and out["events"][0]["type"] == "handoff"


def test_plan_sale_makes_the_whole_change_set_in_one_call(store):
    """What small models need: one business call, plain words, named period, product by name."""
    from a_commerce import call
    store.set_clock("2026-11-06T11:02")
    out = call(store, "plan_sale", {"category": "summer outerwear", "percent_off": "20", "audience": "vip",
                                    "period": "this_weekend", "on_main_page": "true", "leave_out_low_stock": True,
                                    "overrides": '[{"product_id": "seersucker overshirt", "percent_off": 15}]'})
    assert len(out["preview_ids"]) == 3 and out["products"] == 40 and all(c["passed"] for c in out["checks"])
    cs = changesets.create(store, "VIP weekend sale: summer outerwear", REQUEST_1, out["preview_ids"])
    from a_commerce.harness.acceptance import judge
    assert all(ok for _, ok in judge(changesets.view(store, cs)))
    assert changesets.view(store, cs)["groups"][1]["fields"]["Code"] == "VIPWEEKEND20"


def test_a_period_name_in_a_date_field_and_bad_dates_never_crash(store):
    from a_commerce import call
    store.set_clock("2026-11-06T11:02")
    out = call(store, "plan_sale", {"category": "summer outerwear", "percent_off": 10, "audience": "gold", "starts_at": "next_weekend"})
    assert "error" not in out and out["products"] == 42
    view = changesets.view(store, changesets.create(store, "x", "y", out["preview_ids"]))
    assert {u["label"]: u["value"] for u in view["understood"]}["When"] == "Nov 14–15"
    bad = call(store, "plan_sale", {"category": "summer outerwear", "percent_off": 10, "audience": "gold",
                                    "starts_at": "next friday", "ends_at": "sunday"})
    assert bad["error"]["code"] == "invalid_input" and "this_weekend" in bad["error"]["message"]
