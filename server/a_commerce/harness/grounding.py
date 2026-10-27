"""Check what the agent says against what the change set does.

A small model's reply can say "the sale is now live" before anyone approved, or "low-stock products are left out"
when they are not. The change set is the source of truth: if the reply contradicts it, the operator gets a summary
built from the change set instead, plus a note of what was corrected.
"""

from __future__ import annotations

import re

LIVE = re.compile(r"\b(is|are|now|went|gone|been)\s+(now\s+)?(live|active|running|published|launched|applied)\b"
                  r"|\bhas been (applied|published|launched|activated|scheduled and is live)\b", re.I)
EXCLUDED = re.compile(r"(exclud\w*|dropp?\w*|remov\w*|left out|leave out|leaving out)[^.]{0,60}(low[- ]stock|fewer than|less than \d+)"
                      r"|(low[- ]stock)[^.]{0,60}(exclud\w*|dropp?\w*|remov\w*|left out)", re.I)
# Only claims about the size of the sale ("40 prices", "40 products now"), not "2 products have fewer than 10 left".
COUNT = re.compile(r"\b(\d{1,4})\s+(prices|products now|products on sale|products in the sale|products at \d)", re.I)
CHECKS = re.compile(r"\ball\s+(\d+)\s+checks?\s+pass", re.I)


def check_reply(text: str, view: dict) -> list[str]:
    """Contradictions between the reply and the change set, in words for the operator."""
    issues = []
    groups = {g["op"]: g for g in view["groups"]}
    rows = groups.get("update", {}).get("rows", [])
    failing = {c["rule"] for c in view["checks"] if not c["passed"]}
    if view["status"] != "live" and LIVE.search(text):
        issues.append("It said the change is live, but nothing changes until you approve.")
    if "min_stock" in failing and EXCLUDED.search(text):
        issues.append("It said low-stock products are left out, but they are still in this version.")
    total = groups.get("update", {}).get("total_rows")
    for m in COUNT.finditer(text):
        n = int(m.group(1))
        if total is not None and n != total:
            issues.append(f"It said {n} {m.group(2).split()[0].lower()}, but the change set has {total}.")
            break
    # A product named in the reply: full name first; a last word ("overshirt") only if one product has it.
    last_words = [r["name"].split()[-1].lower() for r in rows]
    for r in rows:
        short = r["name"].split()[-1].lower()
        names = [r["name"]] + ([short] if last_words.count(short) == 1 or r["discount"] != "20%" and r.get("note") else [])
        for name in names:
            m = re.search(rf"\b{re.escape(name)}\b[^.%]{{0,30}}?(\d{{1,2}})\s*%", text, re.I)
            if m and f"{m.group(1)}%" != r["discount"]:
                issues.append(f"It said the {short} is at {m.group(1)}%, but it is at {r['discount']}.")
                break
    m = CHECKS.search(text)
    if m and (int(m.group(1)) != view["checks_total"] or failing):
        issues.append(f"It said all checks pass, but {view['checks_passed']} of {view['checks_total']} do.")
    return issues


def summary(view: dict) -> str:
    """A reply built only from the change set."""
    groups = {g["op"]: g for g in view["groups"]}
    understood = {u["label"]: u for u in view["understood"]}
    parts = [f"Change set #{view['id']} v{view['version']} is ready for review. Nothing has changed in the store yet."]
    if "update" in groups:
        prod, ben = understood.get("Products", {}), understood.get("Benefit", {})
        extra = f" ({ben['detail']})" if ben.get("detail") else ""
        parts.append(f"{groups['update']['total_rows']} products in {prod.get('value', 'the sale')} at {ben.get('value', '')}{extra}.")
    if "create" in groups:
        parts.append(f"Coupon {groups['create']['fields'].get('Code')} for {groups['create']['fields'].get('Issued to')}.")
    if "schedule" in groups:
        parts.append(f"Main page hero, {groups['schedule']['summary'].split(', ', 1)[-1]}.")
    failing = [c for c in view["checks"] if not c["passed"]]
    if failing:
        parts.append(f"{view['checks_passed']} of {view['checks_total']} checks pass. Needs your call: "
                     + "; ".join(f"{c['title'].lower()} ({c['detail']})" for c in failing) + ".")
    else:
        parts.append(f"All {view['checks_total']} checks pass.")
    return " ".join(parts)
