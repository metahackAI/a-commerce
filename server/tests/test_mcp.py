"""Outside agents over MCP use the same tools and policies, and still cannot approve anything."""

import anyio
import httpx
from fastapi.testclient import TestClient
from mcp.shared.memory import create_connected_server_and_client_session

from a_commerce.api import app
from a_commerce.mcp_server import build_server

PERSON_ONLY = {"approve_change_set", "undo_change_set", "place_order", "send_return_request", "refund_now"}


def structured(result):
    return result.structuredContent


async def operator_flow():
    async with httpx.AsyncClient(transport=httpx.ASGITransport(app=app), base_url="http://api") as api:
        await api.post("/demo/reset")
        await api.post("/demo/clock", json={"at": "2026-11-06T11:02"})
        async with create_connected_server_and_client_session(build_server(api, "operator")) as client:
            names = {t.name for t in (await client.list_tools()).tools}
            assert {"plan_sale", "plan_return_review", "propose_change_set"} <= names and not names & PERSON_ONLY
            assert "add_to_cart" not in names
            plan = structured(await client.call_tool("plan_sale", {
                "category": "summer_outerwear", "percent_off": 20, "audience": "vip", "period": "this_weekend",
                "on_main_page": True, "leave_out_low_stock": True,
                "overrides": [{"product_id": "seersucker overshirt", "percent_off": 15}]}))
            assert plan["products"] == 40 and all(c["passed"] for c in plan["checks"])
            cs = structured(await client.call_tool("propose_change_set", {
                "title": "VIP weekend sale: summer outerwear", "request": "From an outside agent", "preview_ids": plan["preview_ids"]}))
            assert (cs["change_set_id"], cs["status"], cs["checks"]) == (1042, "waiting_for_approval", "6 of 6 pass")
        # It shows up for the operator, waiting for a person.
        tasks = (await api.get("/bo/tasks")).json()["items"]
        assert tasks[0]["change_set_id"] == 1042 and tasks[0]["status_label"] == "Ready to review"


async def shopper_flow():
    async with httpx.AsyncClient(transport=httpx.ASGITransport(app=app), base_url="http://api") as api:
        await api.post("/demo/reset")
        async with create_connected_server_and_client_session(build_server(api, "shopper", "outside")) as client:
            names = {t.name for t in (await client.list_tools()).tools}
            assert {"search_products", "compare_products", "add_to_cart", "prepare_order"} <= names
            assert not names & {"set_sale_prices", "plan_sale", "refund_return", "propose_change_set"} and not names & PERSON_ONLY
            found = structured(await client.call_tool("search_products", {"query": "windbreaker"}))
            assert found["items"][0]["price"] == 89.00 and "cost" not in found["items"][0]


def test_an_outside_operator_agent_can_only_propose():
    anyio.run(operator_flow)


def test_an_outside_shopper_agent_gets_shopper_tools_only():
    anyio.run(shopper_flow)
