"""The screen-shaped facade can do the demo task using only what it shows, so a model failing on it is about its shape."""

import json

from a_commerce import changesets, facade
from a_commerce.harness.acceptance import judge_v1


def test_facade_by_hand_makes_the_same_change_set_from_what_it_shows(store):
    """Every code, flag and format used here appears in the tool definitions or in an earlier response."""
    seen = [json.dumps(facade.definitions())]

    codes = ("grpCd", "schCtgCd", "rowStatus", "prmtTpCd", "tgtMbrGrdCd", "dispAreaCd", "prmtNo")

    def ask(name, args):
        rows = args.get("rows", [])
        used = [a[k] for a in [args, *rows] for k in codes if k in a] + [p for r in rows for p in r.get("prdNoList", [])]
        for v in used:
            assert v in "".join(seen), f"{v!r} was never shown to the caller"
        out = facade.call(store, name, args)
        seen.append(json.dumps(out))
        return out

    def code(grp, name):
        return next(c["cd"] for c in ask("getCodeList", {"grpCd": grp})["list"] if c["cdNm"] == name)

    store.set_clock("2026-11-06T11:02")
    ctg, vip, rate, hero = code("CTG", "Summer outerwear"), code("MBR_GRD", "VIP"), code("PRMT_TP", "Rate off"), \
        code("DISP_AREA", "Main hero banner")
    assert ask("getPrdGridListCount", {"schCtgCd": ctg})["totCnt"] == 42
    prd = []
    for page in (1, 2, 3):  # 20 rows per page at most
        prd += [r["prdNo"] for r in ask("getPrdGridList", {"schCtgCd": ctg, "pageNo": page, "pageSize": 20})["list"]]
    assert len(prd) == 42 and facade.DTM in seen[0] and "I:insert" in seen[0]
    saved = ask("savePrmtList", {"rows": [{"rowStatus": "I", "prmtNm": "VIP weekend", "prmtTpCd": rate, "dcRt": 20,
                                           "tgtMbrGrdCd": vip, "strtDtm": "20261107000000", "endDtm": "20261108235900",
                                           "prdNoList": prd}]})
    prmt = saved["prmtNoList"][0]
    cpn = ask("saveCpnIssue", {"cpnCd": "VIPWEEKEND20", "prmtNo": prmt, "tgtMbrGrdCd": vip})
    bnr = ask("saveDispBnr", {"dispAreaCd": hero, "tgtMbrGrdCd": vip, "strtDtm": "20261107000000",
                              "endDtm": "20261108235900", "bnrTtl": "20% off summer outerwear", "prmtNo": prmt})
    cs = changesets.create(store, "VIP weekend", "…", [prmt, cpn["cpnNo"], bnr["bnrNo"]])
    assert all(ok for _, ok in judge_v1(changesets.view(store, cs)))


def test_facade_errors_say_nothing_useful(store):
    assert facade.call(store, "savePrmtList", {"rows": [{"rowStatus": "I", "prmtTpCd": "T03"}]}) == {"resultCd": "E999", "resultMsg": "System error"}
    assert facade.call(store, "getCodeList", {"grpCd": "CATEGORY"})["resultMsg"] == "Invalid parameter"


def test_every_comparison_task_can_be_expressed_on_the_facade(store):
    """Fairness: each task's audience, category and dates have a facade code or format."""
    from a_commerce.harness.compare import TASKS
    grades = {c["cdNm"] for c in facade.call(store, "getCodeList", {"grpCd": "MBR_GRD"})["list"]}
    cats = {c["cdNm"].lower().replace(" ", "_") for c in facade.call(store, "getCodeList", {"grpCd": "CTG"})["list"]}
    for t in TASKS:
        assert t["audience"].split()[0].upper() in grades, t["id"]
        assert t["category"] in cats, t["id"]
