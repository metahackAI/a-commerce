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


REFUND_COUNT = re.compile(r"\b(\d{1,3})\s+refunds?\b", re.I)
MONEY = re.compile(r"\$\s?([\d,]+\.\d{2})")


def check_reply(text: str, view: dict) -> list[str]:
    """Contradictions between the reply and the change set, in words for the operator."""
    if view.get("kind") == "refund_review":
        return check_refund_reply(text, view)
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


def check_refund_reply(text: str, view: dict) -> list[str]:
    issues = []
    if view["status"] != "live" and re.search(r"\b(have been|has been|were|was)\s+(refunded|sent|issued)\b", text, re.I):
        issues.append("It said refunds or messages went out, but nothing goes out until you approve.")
    m = REFUND_COUNT.search(text)
    if m and int(m.group(1)) != view["summary"]["refund"]["count"]:
        issues.append(f"It said {m.group(1)} refunds, but the policy allows {view['summary']['refund']['count']}.")
    amounts = [float(x.replace(",", "")) for x in MONEY.findall(text)]
    rows = {r["amount"] for r in view["rows"]} | {view["summary"]["refund"]["amount"]}
    wrong = [a for a in amounts if a not in rows]
    if wrong:
        issues.append(f"It said ${wrong[0]:,.2f}, which is not an amount in this review.")
    return issues


def refund_summary(view: dict) -> str:
    s = view["summary"]
    failing = [c for c in view["checks"] if not c["passed"]]
    parts = [f"Change set #{view['id']}: {s['waiting']} returns waiting. The policy allows {s['refund']['count']} refunds "
             f"(${s['refund']['amount']:,.2f}), asks for photos on {s['ask_for_photos']} and declines {s['decline']}. "
             "Nothing is refunded or sent until you approve."]
    if failing:
        parts.append(f"{len(failing)} to look at: " + "; ".join(c["detail"] for c in failing) + ".")
    return " ".join(parts)


def summary(view: dict) -> str:
    """A reply built only from the change set."""
    if view.get("kind") == "refund_review":
        return refund_summary(view)
    if not view["understood"]:  # next-step change sets: reorder, campaign, notices
        parts = [f"Change set #{view['id']} v{view['version']} is ready for review. Nothing has changed yet."]
        parts += [f"{g['title']}: {g['summary']}." for g in view["groups"]]
        if view["after_approval"].get("final"):
            parts.append("Cannot be undone once run: " + "; ".join(view["after_approval"]["final"]).lower() + ".")
        return " ".join(parts)
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


def check_sales_reply(text: str, data: dict) -> list[str]:
    """Every dollar amount in an answer about sales must come from the data (allowing a rounded "about $4,000")."""
    known = {abs(data["revenue"]), abs(data["usual_revenue"]), abs(data["difference"])}
    known |= {abs(c["estimated"]) for c in data["causes"]}
    explained = abs(sum(c["estimated"] for c in data["causes"] if c["cause"] != "Not explained"))
    known.add(explained)
    issues = []
    for m in re.finditer(r"\$\s?([\d,]+(?:\.\d+)?)\s*(k)?", text):
        value = float(m.group(1).replace(",", "")) * (1000 if m.group(2) else 1)
        about = re.search(r"(about|around|roughly|nearly)\s*$", text[:m.start()], re.I)
        if not any(abs(value - k) <= (max(k * 0.05, 50) if about else 0.5) for k in known):
            issues.append(f"It said ${value:,.0f}, which is not in the sales data.")
    m = re.search(r"(\d{1,2})\s*%\s*(under|below|down|less)", text, re.I)
    if m and int(m.group(1)) != abs(data["difference_pct"]):
        issues.append(f"It said {m.group(1)}% down, but the data says {abs(data['difference_pct'])}%.")
    return issues


def sales_summary(data: dict) -> str:
    parts = [f"Revenue on {data['day']} was ${data['revenue']:,.0f}, ${abs(data['difference']):,.0f} "
             f"({abs(data['difference_pct'])}%) {'under' if data['difference'] < 0 else 'over'} {data['baseline']} "
             f"(${data['usual_revenue']:,.0f} on average)."]
    for c in data["causes"]:
        parts.append(f"{c['cause']}: ${abs(c['estimated']):,.0f}.")
    parts.append("Nothing was changed.")
    return " ".join(parts)
