"""What a real model or a real shopper can send that must come back as an answer, never as a crash or a wrong result."""

import httpx
import pytest
from fastapi.testclient import TestClient

from a_commerce import changesets
from a_commerce.api import app
from a_commerce.harness import run_operator
from a_commerce.harness.models import OpenAICompatible, Reply, ToolCallReq
from a_commerce.harness.operator import calls_in_text
from a_commerce.harness.shopper import run_shopper
from a_commerce.tools.base import REGISTRY, ToolError, call, loose_json

client = TestClient(app)
GUEST = {"shopper": True, "member_id": None, "tier": None, "guest_id": "mina"}


def test_a_change_set_holds_only_write_previews(store):
    store.set_clock("2026-11-07T10:00")
    call(store, "add_to_cart", {"product_id": "prd_windbreaker", "size": "M", "qty": 1}, context=GUEST)
    order = call(store, "prepare_order", {}, context=GUEST)
    with pytest.raises(ToolError) as e:
        changesets.create(store, "x", "x", [order["order_preview_id"]])
    assert e.value.payload()["error"]["code"] == "not_a_change"


def test_a_guest_can_place_the_order_the_agent_prepared():
    client.post("/demo/reset")
    client.post("/demo/clock", json={"at": "2026-11-07T10:00"})
    headers = {"X-Shopper": "mina"}
    client.post("/tools/add_to_cart", json={"args": {"product_id": "prd_windbreaker", "size": "M", "qty": 1}}, headers=headers)
    order = client.post("/tools/prepare_order", json={"args": {}}, headers=headers).json()
    r = client.post("/orders", json={"order_preview_id": order["order_preview_id"]}, headers=headers)
    assert r.status_code == 200, r.text


def test_checks_run_again_at_commit(store):
    """A preview whose checks passed is refused at approval if the store changed underneath it."""
    from a_commerce.tools.base import commit

    store.set_clock("2026-11-06T11:05")
    pv = call(store, "set_sale_prices", {"name": "VIP weekend", "audience": "vip", "starts_at": "2026-11-07T00:00",
                                         "ends_at": "2026-11-08T23:59",
                                         "items": [{"product_id": "prd_windbreaker", "percent_off": 20}]})
    assert all(c["passed"] for c in pv["checks"])
    store.execute("UPDATE variants SET stock = 0 WHERE product_id = 'prd_windbreaker'")
    with pytest.raises(ToolError) as e:
        commit(store, pv["preview_id"], actor="executor")
    assert e.value.payload()["error"]["code"] == "check_failed"


def test_a_coupon_for_all_members_counts_all_members(store):
    store.set_clock("2026-11-06T11:05")
    total = store.one("SELECT COUNT(*) AS n FROM members")["n"]
    for audience in ("members", "everyone"):
        sale = call(store, "set_sale_prices", {"name": "x", "audience": audience, "starts_at": "2026-11-07T00:00",
                                               "ends_at": "2026-11-08T23:59",
                                               "items": [{"product_id": "prd_windbreaker", "percent_off": 10}]})
        coupon = call(store, "create_coupon", {"code": f"ALL{audience.upper()}", "promotion_id": f"ref:{sale['preview_id']}",
                                               "audience": audience})
        assert coupon["changes"][0]["issued_to"] == total, audience


def test_a_category_form_that_matches_nothing_says_so(store):
    out = call(store, "set_sale_prices", {"name": "x", "audience": "vip", "starts_at": "2026-11-07T00:00",
                                          "ends_at": "2026-11-08T23:59", "category": "summer_outerwear",
                                          "percent_off": 10, "min_stock": 100000})
    assert out["error"]["code"] == "no_products"


def test_an_unknown_period_is_an_error_the_model_can_read(store):
    out = call(store, "plan_sale", {"audience": "vip", "category": "summer_outerwear", "percent_off": 20,
                                    "period": "this weekend"})
    assert out["error"]["code"] == "invalid_input" and "this weekend" in out["error"]["message"]


def test_text_fields_stay_text_and_odd_strings_do_not_crash(store):
    assert "error" not in call(store, "search_products", {"query": "2026"})
    assert loose_json("{[1]: 2}") == "{[1]: 2}"
    assert "error" in call(store, "set_sale_prices", {"name": "x", "audience": "vip", "percent_off": "{[1]: 2}"})


def test_plan_sale_says_it_prepares_previews_and_who_asked(store):
    assert REGISTRY["plan_sale"].definition()["x-kind"] == "prepare"
    call(store, "plan_sale", {"audience": "vip", "category": "summer_outerwear", "percent_off": 20,
                              "period": "this_weekend"}, actor="agent:test-model")
    actors = {r["actor"] for r in store.all("SELECT actor FROM action_log WHERE action LIKE '%:preview'")}
    assert actors == {"agent:test-model"}


def test_tool_calls_written_as_text_in_other_shapes():
    names = {"plan_sale", "propose_change_set"}
    openai_shape = '{"type": "function", "function": {"name": "plan_sale", "arguments": "{\\"audience\\": \\"vip\\"}"}}'
    assert [(c.name, c.arguments) for c in calls_in_text(openai_shape, names)] == [("plan_sale", {"audience": "vip"})]
    two = 'First {"name": "plan_sale", "parameters": {"audience": "vip"}} then {"name": "propose_change_set", "parameters": {}}'
    assert [c.name for c in calls_in_text(two, names)] == ["plan_sale", "propose_change_set"]
    assert calls_in_text('{"name": {"oops": 1}, "parameters": {}}', names) == []


def test_a_model_server_error_hands_the_request_to_the_planner(store):
    store.set_clock("2026-11-06T11:02")
    down = OpenAICompatible("http://fake/v1", "k", "down-model", transport=httpx.MockTransport(lambda r: httpx.Response(500)))

    class Planner:
        name = "planner"

        def complete(self, messages, tools):
            return Reply("I can't.", [], self.name, 1)

    out = run_operator(store, "Run a 20% weekend sale on summer outerwear for VIP members.", down, fallback=Planner())
    assert out["events"][0]["type"] == "handoff" and out["events"][0]["from"] == "down-model"


def test_a_malformed_tool_call_is_a_model_error(store):
    body = {"choices": [{"message": {"content": "", "tool_calls": [{"id": "1", "type": "function"}]}}]}
    model = OpenAICompatible("http://fake/v1", "k", "odd-model", transport=httpx.MockTransport(lambda r: httpx.Response(200, json=body)))
    out = run_operator(store, "Run a 20% weekend sale on summer outerwear for VIP members.", model)
    assert out["ok"] is False and "malformed tool call" in out["error"]


def test_a_view_outside_the_list_is_an_error_the_shopper_model_sees(store):
    class Model:
        name = "view-model"
        turns = 0

        def complete(self, messages, tools):
            self.turns += 1
            if self.turns == 1:
                return Reply("", [ToolCallReq("c1", "show_products", {"view": "gallery", "product_ids": ["prd_windbreaker"],
                                                                      "chips": [], "reply": "ok"})], self.name, 1)
            self.seen = messages[-1]["content"]
            return Reply("Here you go.", [], self.name, 1)

    model = Model()
    out = run_shopper(store, "jackets", model, GUEST)
    assert out["events"][0]["text"] == "Here you go." and "view must be one of" in model.seen
