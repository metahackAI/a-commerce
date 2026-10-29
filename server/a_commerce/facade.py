"""A screen-shaped API over the same store, for the comparison in docs/design.md §7.

It reproduces the patterns of design.md §2 (generic examples, written by us): screen and grid names, abbreviations,
codes instead of meaning, list reads without a total plus a separate count call, bulk saves with a row-status flag,
a V2 duplicate, terse descriptions, and errors that say nothing useful. Writes still become previews underneath, so
the change set, approval and read-back are the same for both tool layers; only what the model sees differs.
"""

from __future__ import annotations

from datetime import datetime

from .db import Store
from .tools.base import REGISTRY, ToolError, loose_json, make_preview, normalize

PAGE_MAX = 20
GRADES = {"G00": "everyone", "G01": "vip", "G02": "gold", "G03": "member"}
PROMO_TYPES = {"T01": "Amount off", "T02": "Free shipping", "T03": "Rate off", "T04": "Bundle", "T05": "Coupon"}
DISP_AREAS = {"MAIN_HERO": "Main hero banner"}


def _ctg_codes(store: Store) -> dict[str, str]:
    return {f"C{i:04d}": r["id"] for i, r in enumerate(store.all("SELECT id FROM categories ORDER BY rowid"), start=1)}


def _prd_codes(store: Store) -> dict[str, str]:
    return {f"P{i:04d}": r["id"] for i, r in enumerate(store.all("SELECT id FROM products ORDER BY rowid"), start=1)}


def _dtm(value: str) -> str:
    try:
        return datetime.strptime(str(value), "%Y%m%d%H%M%S").isoformat(timespec="minutes")
    except ValueError:
        raise ToolError("E100", "Invalid parameter", None) from None


def _fail(_e: Exception | None = None) -> dict:
    return {"resultCd": "E999", "resultMsg": "System error"}


def get_code_list(store: Store, a: dict) -> dict:
    grp = a.get("grpCd")
    if grp == "CTG":
        names = {r["id"]: r["name"] for r in store.all("SELECT id, name FROM categories")}
        return {"resultCd": "0000", "list": [{"cd": c, "cdNm": names[i]} for c, i in _ctg_codes(store).items()]}
    if grp == "MBR_GRD":
        return {"resultCd": "0000", "list": [{"cd": c, "cdNm": v.upper()} for c, v in GRADES.items()]}
    if grp == "PRMT_TP":
        return {"resultCd": "0000", "list": [{"cd": c, "cdNm": v} for c, v in PROMO_TYPES.items()]}
    if grp == "DISP_AREA":
        return {"resultCd": "0000", "list": [{"cd": c, "cdNm": v} for c, v in DISP_AREAS.items()]}
    return {"resultCd": "E100", "resultMsg": "Invalid parameter"}


def get_prd_grid_list(store: Store, a: dict) -> dict:
    ctg = _ctg_codes(store).get(a.get("schCtgCd", ""))
    page, size = int(a.get("pageNo", 1)), min(int(a.get("pageSize", 10)), PAGE_MAX)
    codes = {v: k for k, v in _prd_codes(store).items()}
    rows = store.all("SELECT p.id, p.name, p.price, (SELECT SUM(stock) FROM variants v WHERE v.product_id = p.id) AS stk "
                     "FROM products p WHERE (? IS NULL OR p.category_id = ?) ORDER BY p.rowid", (ctg, ctg))
    rows = rows[(page - 1) * size: page * size]
    return {"resultCd": "0000", "list": [{"prdNo": codes[r["id"]], "prdNm": r["name"], "sllPrc": r["price"], "stkQty": r["stk"]}
                                         for r in rows]}


def get_prd_grid_list_count(store: Store, a: dict) -> dict:
    ctg = _ctg_codes(store).get(a.get("schCtgCd", ""))
    n = store.one("SELECT COUNT(*) AS n FROM products WHERE (? IS NULL OR category_id = ?)", (ctg, ctg))["n"]
    return {"resultCd": "0000", "totCnt": n}


def get_prmt_tgt_mbr_mgmt_list(store: Store, a: dict) -> dict:
    rows = store.all("SELECT tier_id, COUNT(*) AS n FROM members GROUP BY tier_id")
    codes = {v: k for k, v in GRADES.items()}
    return {"resultCd": "0000", "list": [{"mbrGrdCd": codes.get(r["tier_id"]), "mbrCnt": r["n"]} for r in rows]}


def save_prmt_list(store: Store, a: dict) -> dict:
    out = []
    prd = _prd_codes(store)
    for row in a.get("rows") or []:
        if row.get("rowStatus") != "I" or row.get("prmtTpCd") != "T03":
            return _fail()
        try:
            items = [{"product_id": prd[p], "percent_off": float(row["dcRt"])} for p in row["prdNoList"]]
            args = {"name": row["prmtNm"], "audience": GRADES[row["tgtMbrGrdCd"]], "starts_at": _dtm(row["strtDtm"]),
                    "ends_at": _dtm(row["endDtm"]), "items": items}
            pv = make_preview(store, REGISTRY["set_sale_prices"], normalize(store, REGISTRY["set_sale_prices"], args), actor=a.get("_actor", "agent"))
        except (KeyError, TypeError, ValueError, ToolError) as e:
            return _fail(e)
        out.append(pv["preview_id"])  # no rule checks come back: the screen API never had them
    return {"resultCd": "0000", "prmtNoList": out}


def save_cpn_issue(store: Store, a: dict) -> dict:
    try:
        args = {"code": a["cpnCd"], "promotion_id": f"ref:{a['prmtNo']}", "audience": GRADES[a["tgtMbrGrdCd"]],
                "auto_apply": a.get("autoApplyYn", "Y") == "Y", "stacking": "no_tier_coupons" if a.get("dupUseYn", "N") == "N" else "stacks"}
        pv = make_preview(store, REGISTRY["create_coupon"], normalize(store, REGISTRY["create_coupon"], args), actor=a.get("_actor", "agent"))
    except (KeyError, TypeError, ValueError, ToolError) as e:
        return _fail(e)
    return {"resultCd": "0000", "cpnNo": pv["preview_id"]}


def save_disp_bnr(store: Store, a: dict) -> dict:
    if a.get("dispAreaCd") != "MAIN_HERO":
        return _fail()
    try:
        args = {"slot_id": "slot_main_hero", "audience": GRADES[a["tgtMbrGrdCd"]], "starts_at": _dtm(a["strtDtm"]),
                "ends_at": _dtm(a["endDtm"]), "headline": a["bnrTtl"], "link": a.get("lnkUrl")}
        if a.get("prmtNo"):
            args["promotion_id"] = f"ref:{a['prmtNo']}"
        pv = make_preview(store, REGISTRY["schedule_banner"], normalize(store, REGISTRY["schedule_banner"], args), actor=a.get("_actor", "agent"))
    except (KeyError, TypeError, ValueError, ToolError) as e:
        return _fail(e)
    return {"resultCd": "0000", "bnrNo": pv["preview_id"]}


def _fn(name: str, desc: str, props: dict, required: list[str]) -> dict:
    return {"type": "function", "function": {"name": name, "description": desc,
                                             "parameters": {"type": "object", "properties": props, "required": required}}}


def s(desc: str) -> dict:
    return {"type": "string", "description": desc}


def n(desc: str) -> dict:
    return {"type": "number", "description": desc}


DTM = "yyyyMMddHHmmss"
TOOLS = {
    "getCodeList": (get_code_list, _fn("getCodeList", "Get code list",
                                       {"grpCd": s("CTG(category) / MBR_GRD(member grade) / PRMT_TP(prmt type) / DISP_AREA(disp area)")},
                                       ["grpCd"])),
    "getPrdGridList": (get_prd_grid_list, _fn("getPrdGridList", "Get prd grid list (POST)",
                                              {"schCtgCd": s("CTG cd"), "pageNo": n("page no"), "pageSize": n("page size, max 20")},
                                              ["pageNo"])),
    "getPrdGridListCount": (get_prd_grid_list_count, _fn("getPrdGridListCount", "Get prd grid list count",
                                                         {"schCtgCd": s("CTG cd")}, [])),
    "getPrmtTgtMbrMgmtList": (get_prmt_tgt_mbr_mgmt_list, _fn("getPrmtTgtMbrMgmtList", "Get prmt tgt mbr mgmt list", {}, [])),
    "savePrmtList": (save_prmt_list, _fn("savePrmtList", "Save prmt list", {"rows": {"type": "array", "items": {"type": "object", "properties": {
        "rowStatus": s("I:insert U:update D:delete"), "prmtNm": s("prmt nm"), "prmtTpCd": s("PRMT_TP cd"), "dcRt": n("dc rate (%)"),
        "tgtMbrGrdCd": s("MBR_GRD cd"), "strtDtm": s(DTM), "endDtm": s(DTM),
        "prdNoList": {"type": "array", "items": {"type": "string"}, "description": "prdNo list"}}}}}, ["rows"])),
    "savePrmtListV2": (save_prmt_list, _fn("savePrmtListV2", "Save prmt list v2", {"rows": {"type": "array", "items": {"type": "object"}}}, ["rows"])),
    "saveCpnIssue": (save_cpn_issue, _fn("saveCpnIssue", "Save cpn issue", {
        "cpnCd": s("cpn cd"), "prmtNo": s("prmt no (savePrmtList)"), "tgtMbrGrdCd": s("MBR_GRD cd"), "autoApplyYn": s("Y/N"),
        "dupUseYn": s("Y/N")}, ["cpnCd", "prmtNo", "tgtMbrGrdCd"])),
    "saveDispBnr": (save_disp_bnr, _fn("saveDispBnr", "Save disp bnr", {
        "dispAreaCd": s("DISP_AREA cd"), "tgtMbrGrdCd": s("MBR_GRD cd"), "strtDtm": s(DTM), "endDtm": s(DTM), "bnrTtl": s("bnr ttl"),
        "lnkUrl": s("lnk url"), "prmtNo": s("prmt no (savePrmtList)")}, ["dispAreaCd", "tgtMbrGrdCd", "strtDtm", "endDtm", "bnrTtl"])),
}


def definitions() -> list[dict]:
    return [d for _, d in TOOLS.values()]


def call(store: Store, name: str, args: dict, actor: str = "agent") -> dict:
    if name not in TOOLS:
        return {"resultCd": "E404", "resultMsg": "Not found"}
    try:
        # Same leniency as the A-Commerce tools, so the comparison is about the API's shape, not string parsing.
        return TOOLS[name][0](store, {**{k: loose_json(v) for k, v in (args or {}).items()}, "_actor": actor})
    except ToolError:
        return {"resultCd": "E100", "resultMsg": "Invalid parameter"}
    except Exception:  # the legacy API answers everything else the same way
        return _fail()
