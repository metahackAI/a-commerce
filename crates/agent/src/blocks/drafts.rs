//! The `product_drafts` block (docs/api/chat.md §3.3): the drafts a supplier sheet becomes
//! (scene B4), built from the `batch_import` preview. Every number comes from the preview: the
//! progress counts and rules from its `product_import` change, each draft's cost, price, sizes,
//! state and note from its `product` change, and the checks as commerce ran them.
//!
//! Drafts are invisible to shoppers. Saving them is approving the change set the import is in
//! (a `person` button); publishing each one is a later change set (`approve_product`).

use acom_contract::{ChangeOp, Check, Preview};
use serde_json::{Value, json};

use super::change_set::ChangeSetView;
use super::{Action, Block, SuggestedIntent};

/// What the person reads for each check of an import.
fn check_label(rule: &str) -> &'static str {
    match rule {
        "product.sku_unique" => "No duplicate SKUs in the catalog",
        "price_floor" => "All prices at or above the floor",
        "import.required_fields" => "Required fields",
        "product.required_on_create" => "Every row has a name and a category",
        _ => "Rule check",
    }
}

/// `passed`, `failed`, or `open`: a passed required-fields check that still leaves drafts for
/// a person (the import keeps them as flagged drafts instead of refusing them).
fn check_status(c: &Check, needs_you: u64) -> &'static str {
    if !c.passed {
        "failed"
    } else if c.rule == "import.required_fields" && needs_you > 0 {
        "open"
    } else {
        "passed"
    }
}

/// `["S", "M", "L", "XL"]` → "S–XL"; no sizes → "One size".
fn sizes_text(sizes: &Value) -> String {
    let list: Vec<&str> = sizes
        .as_array()
        .map(|a| a.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    match list.as_slice() {
        [] => "One size".into(),
        [one] => (*one).to_owned(),
        [first, .., last] => format!("{first}–{last}"),
    }
}

fn money(v: &Value, currency: &Value) -> Value {
    if v.is_null() {
        Value::Null
    } else {
        json!({"value": v, "currency": currency})
    }
}

/// What a person may say about one draft.
fn row_intent(open: &[&str]) -> &'static str {
    if open.contains(&"material") {
        "Add the fabric mix"
    } else if open.contains(&"image") {
        "Pick the photo for this one"
    } else {
        "Is this name right?"
    }
}

/// The block from a `batch_import` preview, with the change set it went into when known.
pub fn product_drafts(p: &Preview, view: Option<&ChangeSetView>) -> Option<Block> {
    let summary = p
        .changes
        .iter()
        .find(|c| c.entity == "product_import" && c.op == ChangeOp::Create)?;
    let s = &summary.to;
    let count = |k: &str| s.get(k).and_then(Value::as_u64).unwrap_or(0);
    let (total, ready, needs_you, working) = (
        count("drafts"),
        count("ready"),
        count("needs_you"),
        count("working"),
    );
    let mut intents = Vec::new();
    let drafts: Vec<Value> = p
        .changes
        .iter()
        .filter(|c| c.entity == "product" && matches!(c.op, ChangeOp::Create | ChangeOp::Update))
        .map(|c| {
            let t = &c.to;
            let id = t.get("product_id").and_then(Value::as_str).unwrap_or_default();
            let open: Vec<&str> = t
                .get("acom:open_fields")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(Value::as_str).collect())
                .unwrap_or_default();
            let element = format!("draft_row:{id}");
            intents.push(SuggestedIntent::new(element.clone(), row_intent(&open)));
            json!({
                "element": element, "draft_id": id, "product_id": id,
                "row": t.get("row"), "name": t.get("name"),
                "source_code": t.get("supplier_code"), "source_text": t.get("acom:supplier_text"),
                "category": t.get("category"),
                "cost": money(t.get("cost").unwrap_or(&Value::Null), t.get("price_currency").unwrap_or(&Value::Null)),
                "price": money(t.get("price").unwrap_or(&Value::Null), t.get("price_currency").unwrap_or(&Value::Null)),
                "sizes": sizes_text(t.get("sizes").unwrap_or(&Value::Null)),
                "status": t.get("acom:draft_status"), "open_fields": open, "note": t.get("note"),
                "op": c.op,
            })
        })
        .collect();
    let checks: Vec<Value> = p
        .checks
        .iter()
        .map(|c| {
            let element = format!("check_row:{}", c.rule);
            let status = check_status(c, needs_you);
            if status != "passed" {
                intents.push(SuggestedIntent::new(
                    element.clone(),
                    if status == "open" {
                        "Show me the drafts that need me"
                    } else {
                        "What do I fix in the sheet?"
                    },
                ));
            }
            json!({"rule": c.rule, "passed": c.passed, "status": status,
                   "label": check_label(&c.rule), "detail": c.detail, "fix_tool": c.fix_tool,
                   "tool": "batch_import", "element": element})
        })
        .collect();
    let failed = p.checks.iter().filter(|c| !c.passed).count();
    let data = json!({
        "import_id": s.get("import_id"), "upload_id": s.get("upload_id"),
        "file_name": s.get("file_name"), "provider_name": s.get("provider_name"),
        "rows": s.get("rows"), "preview_id": p.preview_id,
        "change_set_id": view.map(|v| v.id.clone()).or_else(|| p.change_set_id.clone()),
        "state": view.map(|v| v.state.clone()),
        "progress": {"total": total, "ready": ready, "needs_you": needs_you, "working": working},
        "drafts": drafts,
        "rules": s.get("rules").cloned().unwrap_or_else(|| json!([])),
        "checks": checks,
        "visibility": summary.note,
    });
    let file = s
        .get("file_name")
        .and_then(Value::as_str)
        .unwrap_or("supplier sheet");
    let mut b = Block::new(
        "product_drafts",
        format!("{total} drafts from {file}"),
        data,
    );
    b.why = Some(if failed > 0 {
        format!("{failed} check(s) failed; fix the sheet and import it again.")
    } else if needs_you > 0 {
        format!(
            "{ready} ready, {needs_you} need you. Drafts are not visible to shoppers; publishing is a separate change set."
        )
    } else {
        "All drafts are ready. Drafts are not visible to shoppers; publishing is a separate change set.".into()
    });
    if let Some(v) = view.filter(|v| v.state == "waiting_for_approval" && failed == 0) {
        b.actions.push(Action::person(
            "save_drafts",
            format!("Save {total} drafts"),
            format!("/v1/platform/change-sets/{}/approve", v.id),
            json!({"version": v.version}),
            format!(
                "Save {total} product drafts ({needs_you} still need you)? Shoppers do not see drafts; each one is published separately."
            ),
        ));
    }
    if needs_you > 0 {
        b.actions.push(Action::say(
            "needs_you",
            format!("Show the {needs_you} that need me"),
            json!({"import_id": s.get("import_id"), "draft_status": "needs_you"}),
        ));
    }
    b.actions.push(Action::open(
        "review",
        "Open the drafts page",
        "/bo/products/drafts",
    ));
    if let Some(v) = view {
        b.actions.push(Action::open(
            "change_set",
            "Open the change set",
            format!("/bo/change-sets/{}", v.id),
        ));
    }
    b.suggested_intents = intents;
    Some(b)
}

#[cfg(test)]
pub(crate) mod fixtures {
    use super::*;

    /// The `batch_import` preview of scene B4, shortened to three of the 30 drafts.
    pub fn b4_preview() -> Preview {
        serde_json::from_value(json!({
            "preview_id": "pv_b4", "expires_at": null, "needs_approval": true,
            "approval_reason": "Creates 30 product drafts", "change_set_id": "cs_1044",
            "changes": [
                {"op": "create", "entity": "product_import", "id": null, "note": "Drafts are not visible to shoppers. Publishing is a separate change set.",
                 "to": {"import_id": "imp_fw26_supplier", "upload_id": "upl_fw26_supplier", "file_name": "fw26_supplier.xlsx",
                        "rows": 30, "drafts": 30, "ready": 26, "needs_you": 4, "working": 0,
                        "rules": [{"rule": "pricing_rule", "label": "Price", "text": "Cost × 2.5, to the nearest price ending in 9 (a tie goes up)"}]}},
                {"op": "create", "entity": "product", "id": null,
                 "to": {"row": 1, "product_id": "prd_fw26_ot_014_chr", "supplier_code": "FW26-OT-014", "name": "Wool blend overcoat, charcoal",
                        "cost": 96.00, "price": 239.00, "price_currency": "USD", "sizes": ["S", "M", "L", "XL"],
                        "acom:draft_status": "ready", "acom:open_fields": [], "note": null}},
                {"op": "create", "entity": "product", "id": null, "note": "Fabric mix is missing in the sheet",
                 "to": {"row": 4, "product_id": "prd_fw26_ot_022_olv", "supplier_code": "FW26-OT-022", "name": "Quilted liner jacket, olive",
                        "cost": 52.00, "price": 129.00, "price_currency": "USD", "sizes": ["S", "M", "L", "XL"],
                        "acom:draft_status": "needs_you", "acom:open_fields": ["material"], "note": "Fabric mix is missing in the sheet"}}
            ],
            "checks": [
                {"rule": "product.required_on_create", "passed": true, "detail": "Every row has a name and a category", "fix_tool": null},
                {"rule": "product.sku_unique", "passed": true, "detail": "No duplicate SKU", "fix_tool": null},
                {"rule": "price_floor", "passed": true, "detail": "30 of 30 prices at or above the floor (cost + 15%)", "fix_tool": null},
                {"rule": "import.required_fields", "passed": true, "detail": "4 drafts need you", "fix_tool": "update_product_group"}
            ]
        }))
        .unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::b4_preview;
    use super::*;

    #[test]
    fn b4_drafts_show_progress_rules_and_the_open_check() {
        let view = ChangeSetView {
            id: "cs_1044".into(),
            number: Some(1044),
            version: 1,
            state: "waiting_for_approval".into(),
            ..ChangeSetView::default()
        };
        let b = product_drafts(&b4_preview(), Some(&view)).unwrap();
        assert_eq!(b.ui, "product_drafts");
        assert_eq!(b.title, "30 drafts from fw26_supplier.xlsx");
        assert_eq!(
            b.data["progress"],
            json!({"total": 30, "ready": 26, "needs_you": 4, "working": 0})
        );
        let coat = &b.data["drafts"][0];
        assert_eq!(coat["sizes"], "S–XL");
        assert_eq!(coat["price"], json!({"value": 239.00, "currency": "USD"}));
        assert_eq!(b.data["drafts"][1]["status"], "needs_you");
        let status: Vec<&str> = b.data["checks"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["status"].as_str().unwrap())
            .collect();
        assert_eq!(status, ["passed", "passed", "passed", "open"]);
        assert_eq!(b.data["rules"][0]["label"], "Price");
        let save = b.person_actions().next().unwrap();
        match save {
            Action::Person {
                path, body, label, ..
            } => {
                assert_eq!(path, "/v1/platform/change-sets/cs_1044/approve");
                assert_eq!(body, &json!({"version": 1}));
                assert_eq!(label, "Save 30 drafts");
            }
            other => panic!("{other:?}"),
        }
        assert!(
            b.suggested_intents
                .iter()
                .any(|i| i.element == "draft_row:prd_fw26_ot_022_olv"
                    && i.say == "Add the fabric mix")
        );
    }

    #[test]
    fn without_an_import_change_there_is_no_block_and_failed_checks_hide_the_save() {
        let mut p = b4_preview();
        p.checks[1].passed = false;
        let view = ChangeSetView {
            id: "cs_1044".into(),
            state: "waiting_for_approval".into(),
            version: 1,
            ..ChangeSetView::default()
        };
        let b = product_drafts(&p, Some(&view)).unwrap();
        assert_eq!(b.person_actions().count(), 0);
        p.changes.remove(0);
        assert!(product_drafts(&p, None).is_none());
        assert_eq!(sizes_text(&json!([])), "One size");
        assert_eq!(sizes_text(&json!(["28", "36"])), "28–36");
    }
}
