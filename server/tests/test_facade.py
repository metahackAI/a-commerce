"""The screen-shaped facade can do the demo task when driven by hand, so a model failing on it is about its shape."""

from a_commerce import changesets, facade
from a_commerce.harness.acceptance import judge_v1


def test_facade_by_hand_makes_the_same_change_set(store):
    store.set_clock("2026-11-06T11:02")
    ctg = next(c["cd"] for c in facade.call(store, "getCodeList", {"grpCd": "CTG"})["list"] if c["cdNm"] == "Summer outerwear")
    vip = next(c["cd"] for c in facade.call(store, "getCodeList", {"grpCd": "MBR_GRD"})["list"] if c["cdNm"] == "VIP")
    assert facade.call(store, "getPrdGridListCount", {"schCtgCd": ctg})["totCnt"] == 42
    prd = []
    for page in (1, 2, 3):  # 20 rows per page at most
        prd += [r["prdNo"] for r in facade.call(store, "getPrdGridList", {"schCtgCd": ctg, "pageNo": page, "pageSize": 20})["list"]]
    assert len(prd) == 42
    saved = facade.call(store, "savePrmtList", {"rows": [{"rowStatus": "I", "prmtNm": "VIP weekend", "prmtTpCd": "T03", "dcRt": 20,
                                                          "tgtMbrGrdCd": vip, "strtDtm": "20261107000000", "endDtm": "20261108235900",
                                                          "prdNoList": prd}]})
    prmt = saved["prmtNoList"][0]
    cpn = facade.call(store, "saveCpnIssue", {"cpnCd": "VIPWEEKEND20", "prmtNo": prmt, "tgtMbrGrdCd": vip})
    bnr = facade.call(store, "saveDispBnr", {"dispAreaCd": "MAIN_HERO", "tgtMbrGrdCd": vip, "strtDtm": "20261107000000",
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
