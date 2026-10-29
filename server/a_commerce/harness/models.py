"""Model clients. Every model is reached through one OpenAI-compatible chat API (docs/plan.md §2).

`Reply` is what the harness needs from any model: some text and some tool calls.
"""

from __future__ import annotations

import json
import os
import time
from dataclasses import dataclass, field
from typing import Protocol

import httpx


@dataclass
class ToolCallReq:
    id: str
    name: str
    arguments: dict


@dataclass
class Reply:
    text: str
    tool_calls: list[ToolCallReq]
    model: str
    duration_ms: int
    usage: dict = field(default_factory=dict)


class ModelError(Exception):
    """The model endpoint failed (timeout, HTTP error, bad response). The harness treats it as a failed attempt."""


class Model(Protocol):
    name: str

    def complete(self, messages: list[dict], tools: list[dict]) -> Reply: ...


class OpenAICompatible:
    """Baseten, Fireworks, Modal (vLLM), or any server that speaks /chat/completions with tools."""

    def __init__(self, base_url: str, api_key: str, model: str, *, timeout: float = 120, max_tokens: int = 1500,
                 transport=None):
        self.name = model
        self.max_tokens = max_tokens  # small models sometimes never stop; cap every reply
        self.client = httpx.Client(base_url=base_url.rstrip("/"), timeout=timeout, transport=transport,
                                   headers={"Authorization": f"Bearer {api_key}"})

    def complete(self, messages: list[dict], tools: list[dict]) -> Reply:
        body = {"model": self.name, "messages": messages, "temperature": 0, "max_tokens": self.max_tokens}
        if tools:
            body["tools"] = [{"type": "function", "function": t["function"]} for t in tools]
            body["tool_choice"] = "auto"
        start = time.monotonic()
        try:
            r = self.client.post("/chat/completions", json=body)
            r.raise_for_status()
            data = r.json()
            msg = data["choices"][0]["message"]
        except (httpx.HTTPError, KeyError, ValueError) as e:
            raise ModelError(f"{self.name}: {type(e).__name__}: {str(e)[:200]}") from e
        calls = []
        for c in msg.get("tool_calls") or []:
            if not isinstance(c, dict) or not isinstance(c.get("function"), dict) or not isinstance(c["function"].get("name"), str):
                raise ModelError(f"{self.name}: malformed tool call {str(c)[:200]}")
            raw = c["function"].get("arguments") or "{}"
            try:
                args = json.loads(raw) if isinstance(raw, str) else raw
            except json.JSONDecodeError:
                args = {"_unparsed": raw}
            calls.append(ToolCallReq(c.get("id") or f"call_{len(calls)}", c["function"]["name"], args))
        return Reply(msg.get("content") or "", calls, self.name, int((time.monotonic() - start) * 1000), data.get("usage", {}))


def from_env(role: str) -> Model | None:
    """role is WORKER or PLANNER. Returns None when that model is not configured."""
    url, key, model = (os.environ.get(f"{role}_{k}") for k in ("BASE_URL", "API_KEY", "MODEL"))
    if url and key and model:
        return OpenAICompatible(url, key, model)
    return None
