"""The store's tools over MCP, for outside agents (ChatGPT, Gemini, Claude, …): one tool layer for every agent.

A thin proxy over the HTTP API, so outside agents see the same data, tools and policies as our own agents.
Person-only actions (approve, undo, place an order, send a return) are not tools here either: an outside operator
agent can propose a change set, and a person approves it in the back office.

    uv run python -m a_commerce.mcp_server --role operator            # stdio, API on 127.0.0.1:8000
    uv run python -m a_commerce.mcp_server --role shopper --shopper outside
"""

from __future__ import annotations

import argparse
from typing import Any

import anyio
import httpx
from mcp import types
from mcp.server.lowlevel import Server
from mcp.server.stdio import stdio_server

PROPOSE = types.Tool(
    name="propose_change_set",
    description="Bundle the previews you made into one change set for a person to review in the back office. "
                "Nothing changes in the store until they approve it.",
    inputSchema={"type": "object", "required": ["title", "request", "preview_ids"], "properties": {
        "title": {"type": "string", "description": "Short title, e.g. 'VIP weekend sale: summer outerwear'."},
        "request": {"type": "string", "description": "What was asked, in the asker's words."},
        "preview_ids": {"type": "array", "items": {"type": "string"}, "description": "preview_id of every write preview."},
    }},
)


def build_server(api: httpx.AsyncClient, role: str = "operator", shopper: str | None = None) -> Server:
    server = Server(f"a-commerce-{role}")
    headers = {"X-Shopper": shopper} if role == "shopper" and shopper else {}

    @server.list_tools()
    async def list_tools() -> list[types.Tool]:
        defs = (await api.get("/tools", params={"scope": role})).json()
        tools = [types.Tool(name=d["function"]["name"], description=d["function"]["description"],
                            inputSchema=d["function"]["parameters"]) for d in defs]
        return tools + ([PROPOSE] if role == "operator" else [])

    # Input is validated by the tools themselves (with the same leniency our own agents get), not by JSON Schema here.
    @server.call_tool(validate_input=False)
    async def call_tool(name: str, arguments: dict[str, Any]) -> dict:
        if name == "propose_change_set":
            if role != "operator":
                return {"error": {"code": "operator_only", "message": "Only the operator role can propose change sets."}}
            r = await api.post("/change-sets", json=arguments)
            view = r.json()
            if r.status_code != 200:
                return {"error": view.get("detail")}
            return {"change_set_id": view["id"], "version": view["version"], "status": view["status"],
                    "checks": f"{view['checks_passed']} of {view['checks_total']} pass",
                    "next": "A person reviews and approves this in the back office."}
        r = await api.post(f"/tools/{name}", json={"args": arguments}, headers=headers)
        return r.json() if r.status_code == 200 else {"error": {"code": str(r.status_code), "message": r.text[:300]}}

    return server


async def _serve(role: str, base_url: str, shopper: str | None) -> None:
    async with httpx.AsyncClient(base_url=base_url, timeout=60) as api:
        server = build_server(api, role, shopper)
        async with stdio_server() as (read, write):
            await server.run(read, write, server.create_initialization_options())


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--role", choices=["operator", "shopper"], default="operator")
    ap.add_argument("--api", default="http://127.0.0.1:8000")
    ap.add_argument("--shopper", default="outside", help="Shopper identity for --role shopper (outside = a guest).")
    args = ap.parse_args()
    anyio.run(_serve, args.role, args.api, args.shopper)


if __name__ == "__main__":
    main()

