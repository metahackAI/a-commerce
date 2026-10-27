"""Tool search: the model never sees every tool (design.md §5).

Score tools by word overlap with the request, then add graph neighbours: tools that consume what a
candidate produces, and tools that produce what a candidate consumes.
"""

from __future__ import annotations

import math
import re
from collections import Counter

from ..tools.base import REGISTRY, Tool

# Words people use for a business action, mapped to words in tool names and descriptions.
SYNONYMS = {
    "sale": ["sale", "price", "discount"], "discount": ["sale", "discount", "price"], "off": ["discount", "sale"],
    "promotion": ["promotion", "sale"], "promo": ["promotion", "sale"],
    "vip": ["tier", "audience", "coupon", "members"], "members": ["tier", "members", "audience"],
    "coupon": ["coupon"], "code": ["coupon"],
    "main": ["banner", "display", "slot"], "page": ["banner", "display", "slot"], "banner": ["banner", "display"],
    "homepage": ["banner", "display"], "hero": ["banner", "slot"],
    "weekend": ["period", "starts", "ends"], "stock": ["stock"], "low": ["stock"],
    "compare": ["compare", "side"], "cart": ["cart"], "buy": ["cart", "order"], "order": ["order"],
    "remember": ["remember", "preferences"],
}
STOP = {"a", "an", "the", "and", "for", "on", "of", "to", "it", "put", "run", "in", "with", "is", "my", "me", "i", "please"}


def words(text: str) -> list[str]:
    return [w for w in re.findall(r"[a-z0-9]+", text.lower()) if w not in STOP]


def tool_text(tool: Tool) -> list[str]:
    fields = " ".join(f"{k} {v.get('description', '')}" for k, v in tool.input.model_json_schema().get("properties", {}).items())
    return words(tool.name.replace("_", " ")) * 3 + words(tool.description) + words(fields)


def search(request: str, *, scope: str = "operator", k: int = 5, limit: int = 10) -> list[Tool]:
    tools = [t for t in REGISTRY.values() if t.scope in (scope, "both")]
    query = []
    for w in words(request):
        query += SYNONYMS.get(w, [w])
    docs = {t.name: Counter(tool_text(t)) for t in tools}
    df = Counter(w for c in docs.values() for w in c)
    n = len(tools)

    def score(t: Tool) -> float:
        c = docs[t.name]
        return sum(math.log(1 + n / df[w]) * (1 + math.log(c[w])) for w in set(query) if c[w])

    ranked = sorted(tools, key=score, reverse=True)
    picked = [t for t in ranked[:k] if score(t) > 0]
    # Graph neighbours: what comes before or after the picked tools.
    for t in list(picked):
        for other in tools:
            if other in picked:
                continue
            feeds = set(t.produces) & set(other.consumes)
            needs = set(t.consumes) & set(other.produces)
            if feeds or needs:
                picked.append(other)
    return picked[:limit]
