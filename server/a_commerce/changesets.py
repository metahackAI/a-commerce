"""Change sets: previews grouped into one reviewable unit (docs/plan.md §3.5, §5.3).

Lifecycle: waiting_for_approval → running → verifying → live | failed; live → undone.
Approve and undo are person-only; they are called from HTTP endpoints, never from a model.
"""

from __future__ import annotations

import json
from datetime import datetime, timedelta

from pydantic import BaseModel, Field

from .db import Store
from .tools.base import ToolError, commit, read_tool

AUDIENCE_LABEL = {"vip": "VIP", "gold": "Gold", "member": "Member", "members": "Members", "everyone": "Everyone"}
APPLY_ORDER = ["set_sale_prices", "create_coupon", "schedule_banner"]


def _previews(store: Store, preview_ids: list[str]) -> list[dict]:
    out = []
    for pid in preview_ids:
        row = store.one("SELECT * FROM previews WHERE id = ?", (pid,))
        if row is None:
            raise ToolError("unknown_preview", f"No preview {pid}.", None)
        out.append({"id": pid, "tool": row["tool"], "args": json.loads(row["args"]), "result": json.loads(row["result"]),
                    "status": row["status"], "committed_id": row["committed_id"]})
    return out


def create(store: Store, title: str, request: str, preview_ids: list[str]) -> int:
    _previews(store, preview_ids)  # validates
    cs_id = int(store.new_id("cs").split("_")[1])
    now = store.now().isoformat(timespec="minutes")
    store.execute("INSERT INTO change_sets VALUES (?, ?, 'waiting_for_approval', 1, ?, ?, NULL, NULL, ?, ?)",
                  (cs_id, title, json.dumps([request]), json.dumps(preview_ids), now, now))
    store.log("agent", "change_set:create", {"id": cs_id, "previews": preview_ids})
    return cs_id


def revise(store: Store, cs_id: int, request: str, preview_ids: list[str]) -> int:
    cs = _get(store, cs_id)
    if cs["status"] != "waiting_for_approval":
        raise ToolError("not_editable", f"Change set #{cs_id} is {cs['status']} and can no longer be edited.", None)
    for old in json.loads(cs["preview_ids"]):
        if old not in preview_ids:
            store.execute("UPDATE previews SET status = 'discarded' WHERE id = ? AND status = 'open'", (old,))
    requests = json.loads(cs["requests"]) + [request]
    store.execute("UPDATE change_sets SET version = version + 1, requests = ?, preview_ids = ?, updated_at = ? WHERE id = ?",
                  (json.dumps(requests), json.dumps(preview_ids), store.now().isoformat(timespec="minutes"), cs_id))
    store.log("agent", "change_set:revise", {"id": cs_id, "version": cs["version"] + 1})
    return cs["version"] + 1


def _get(store: Store, cs_id: int):
    cs = store.one("SELECT * FROM change_sets WHERE id = ?", (cs_id,))
    if cs is None:
        raise ToolError("unknown_change_set", f"No change set #{cs_id}.", None)
    return cs


# -- person-only ----------------------------------------------------------

def approve(store: Store, cs_id: int, version: int, actor: str = "operator") -> dict:
    cs = _get(store, cs_id)
    if cs["status"] != "waiting_for_approval":
        raise ToolError("not_waiting", f"Change set #{cs_id} is {cs['status']}.", None)
    if version != cs["version"]:
        raise ToolError("stale_version", f"You approved v{version}, but #{cs_id} is now v{cs['version']}.", None)
    previews = sorted(_previews(store, json.loads(cs["preview_ids"])), key=lambda p: APPLY_ORDER.index(p["tool"]))
    if any(not c["passed"] for p in previews for c in p["result"]["checks"]):
        raise ToolError("checks_failing", "A check is failing; fix it before approving.", None)
    store.log(actor, "change_set:approve", {"id": cs_id, "version": version})
    _set_status(store, cs_id, "running")
    try:
        for p in previews:
            commit(store, p["id"], actor="executor")
    except ToolError as e:
        _set_status(store, cs_id, "failed")
        store.execute("UPDATE change_sets SET verification = ? WHERE id = ?",
                      (json.dumps({"error": e.payload()["error"]}), cs_id))
        raise
    _set_status(store, cs_id, "verifying")
    result = verify(store, cs_id)
    undo_until = _undo_until(store, previews)
    store.execute("UPDATE change_sets SET status = ?, verification = ?, undo_until = ? WHERE id = ?",
                  ("live" if result["all_match"] else "failed", json.dumps(result), undo_until, cs_id))
    return view(store, cs_id)


def _undo_until(store: Store, previews: list[dict]) -> str | None:
    ends = [p["args"]["ends_at"] for p in previews if p["args"].get("ends_at")]
    if not ends:
        return None
    last = max(datetime.fromisoformat(e) for e in ends)
    days = store.policy_value("undo_window_after_end_days")
    return (last + timedelta(days=days)).replace(hour=23, minute=59).isoformat(timespec="minutes")


def undo(store: Store, cs_id: int, actor: str = "operator") -> dict:
    cs = _get(store, cs_id)
    if cs["status"] not in ("live", "failed"):
        raise ToolError("not_live", f"Change set #{cs_id} is {cs['status']}; nothing to undo.", None)
    if cs["undo_until"] and store.now().isoformat(timespec="minutes") > cs["undo_until"]:
        raise ToolError("undo_expired", f"Change set #{cs_id} could be undone until {cs['undo_until']}.", None)
    table = {"set_sale_prices": "promotions", "create_coupon": "coupons", "schedule_banner": "displays"}
    for p in _previews(store, json.loads(cs["preview_ids"])):
        if p["committed_id"]:
            store.execute(f"UPDATE {table[p['tool']]} SET status = 'undone' WHERE id = ?", (p["committed_id"],))
    _set_status(store, cs_id, "undone")
    store.log(actor, "change_set:undo", {"id": cs_id})
    return view(store, cs_id)


def _set_status(store: Store, cs_id: int, status: str) -> None:
    store.execute("UPDATE change_sets SET status = ?, updated_at = ? WHERE id = ?",
                  (status, store.now().isoformat(timespec="minutes"), cs_id))


# -- read back --------------------------------------------------------------

def verify(store: Store, cs_id: int) -> dict:
    """Read every committed change back from the database and compare it with what was approved."""
    cs = _get(store, cs_id)
    results = []
    for p in _previews(store, json.loads(cs["preview_ids"])):
        if not p["committed_id"]:
            results.append({"tool": p["tool"], "matches": False, "detail": "Not run"})
            continue
        if p["tool"] == "set_sale_prices":
            expected = {c["id"]: c["to"] for c in p["result"]["changes"]}
            got = {r["product_id"]: r["sale_price"] for r in store.all(
                "SELECT product_id, sale_price FROM promotion_items WHERE promotion_id = ?", (p["committed_id"],))}
            promo = store.one("SELECT * FROM promotions WHERE id = ?", (p["committed_id"],))
            ok = got == expected and promo["status"] == "live" and promo["audience"] == p["args"]["audience"]
            results.append({"tool": p["tool"], "matches": ok,
                            "detail": f"{sum(1 for k in expected if got.get(k) == expected[k])} of {len(expected)} sale prices match"})
        elif p["tool"] == "create_coupon":
            c = store.one("SELECT * FROM coupons WHERE id = ?", (p["committed_id"],))
            want = p["result"]["changes"][0]
            ok = c is not None and c["status"] == "live" and c["code"] == want["code"] and c["issued_count"] == want["issued_to"]
            results.append({"tool": p["tool"], "matches": ok, "detail": f"{c['code']} issued to {c['issued_count']:,} members"})
        elif p["tool"] == "schedule_banner":
            d = store.one("SELECT * FROM displays WHERE id = ?", (p["committed_id"],))
            want = p["result"]["changes"][0]
            ok = d is not None and d["status"] == "live" and d["headline"] == want["headline"] and d["audience"] == want["audience"]
            results.append({"tool": p["tool"], "matches": ok, "detail": f"“{d['headline']}” for {d['audience']}, {d['starts_at']} – {d['ends_at']}"})
    return {"all_match": all(r["matches"] for r in results) and bool(results), "changes": results}


class VerifyIn(BaseModel):
    change_set_id: int = Field(description="Change set number.", examples=[1042])


@read_tool("verify_change_set", "Read every change in a change set back from the store and say whether each matches what was approved.",
           VerifyIn, consumes=["change_set_id"], produces=["verification"])
def verify_change_set(store: Store, args: VerifyIn) -> dict:
    return verify(store, args.change_set_id)


# -- the change_set UI block -----------------------------------------------

def view(store: Store, cs_id: int) -> dict:
    cs = _get(store, cs_id)
    previews = _previews(store, json.loads(cs["preview_ids"]))
    by_tool = {p["tool"]: p for p in previews}
    groups, understood = [], []
    checks = [c for p in previews for c in p["result"]["checks"]]

    sale = by_tool.get("set_sale_prices")
    if sale:
        a, rows = sale["args"], sale["result"]["changes"]
        start, end = datetime.fromisoformat(a["starts_at"]), datetime.fromisoformat(a["ends_at"])
        members = store.one("SELECT COUNT(*) AS n FROM members WHERE tier_id = ?", (a["audience"],))["n"]
        categories = sorted({store.one("SELECT category_id FROM products WHERE id = ?", (r["id"],))["category_id"] for r in rows})
        in_cats = store.all(f"SELECT id, name FROM products WHERE category_id IN ({','.join('?' * len(categories))})", categories)
        chosen = {r["id"] for r in rows}
        left_out = []
        for r in in_cats:
            if r["id"] not in chosen:
                stock = store.one("SELECT SUM(stock) AS n FROM variants WHERE product_id = ?", (r["id"],))["n"]
                left_out.append({"name": r["name"], "note": f"{stock} left"})
        pcts = sorted({r["percent_off"] for r in rows}, reverse=True)
        cat_name = store.one("SELECT name FROM categories WHERE id = ?", (categories[0],))["name"]
        exceptions = [r for r in rows if r["percent_off"] != pcts[0]]
        understood += [
            {"label": "Who", "value": f"{AUDIENCE_LABEL[a['audience']]} tier", "detail": f"{members:,} members"},
            {"label": "Products", "value": cat_name,
             "detail": f"{len(rows)} of {len(in_cats)}" + (f" · {len(left_out)} excluded" if left_out else "")},
            {"label": "Benefit", "value": f"{pcts[0]:g}% off",
             "detail": ", ".join(f"{r['name'].split()[-1].title()} {r['percent_off']:g}%" for r in exceptions)},
            {"label": "When", "value": f"{start:%b %-d}–{end:%-d}", "detail": f"{start:%a %H:%M} – {end:%a %H:%M}"},
        ]
        groups.append({
            "op": "update", "title": "Sale prices", "summary": f"{len(rows)} products, {start:%b %-d}–{end:%-d}",
            "undo": "Undo restores current prices", "columns": ["Product", "Stock", "Discount", "VIP price"],
            "rows": [{"product_id": r["id"], "name": r["name"], "stock": r["stock"], "discount": f"{r['percent_off']:g}%",
                      "note": "floor" if r["percent_off"] != pcts[0] else None, "was": r["from"], "now": r["to"]} for r in rows],
            "total_rows": len(rows),
        })
    coupon = by_tool.get("create_coupon")
    if coupon:
        c = coupon["result"]["changes"][0]
        groups.append({"op": "create", "title": "Coupon", "summary": "1 new", "fields": {
            "Code": c["code"], "Issued to": f"{c['issued_to']:,} {AUDIENCE_LABEL[c['audience']]} members",
            "Applies": "Automatically at checkout" if c["auto_apply"] else "When the code is entered",
            "Stacking": "Not with tier coupons" if c["stacking"] == "no_tier_coupons" else "Stacks"}})
    if sale and left_out:
        groups.append({"op": "exclude", "title": "Left out", "summary": f"{len(left_out)} products", "rows": left_out})
    banner = by_tool.get("schedule_banner")
    if banner:
        b = banner["result"]["changes"][0]
        bs, be = datetime.fromisoformat(b["starts_at"]), datetime.fromisoformat(b["ends_at"])
        understood.append({"label": "Where", "value": "Main page hero", "detail": f"Shown to {AUDIENCE_LABEL[b['audience']]} only"})
        groups.append({"op": "schedule", "title": "Main page", "summary": f"Hero slot, {bs:%b %-d}–{be:%-d}", "preview": {
            "audiences": [f"{AUDIENCE_LABEL[b['audience']]} members", "Everyone else"], "date": f"{bs:%a %b %-d}",
            "eyebrow": b["eyebrow"], "headline": b["headline"], "cta": b["cta"]}})

    tools_after = [t for t in APPLY_ORDER if t in by_tool] + ["verify_change_set"]
    return {
        "id": cs["id"], "version": cs["version"], "status": cs["status"], "title": cs["title"],
        "request": json.loads(cs["requests"]), "understood": understood,
        "checks": checks, "checks_passed": sum(1 for c in checks if c["passed"]), "checks_total": len(checks),
        "groups": groups,
        "after_approval": {"tools": tools_after, "undo_until": cs["undo_until"] or _undo_until(store, previews)},
        "verification": json.loads(cs["verification"]) if cs["verification"] else None,
    }
