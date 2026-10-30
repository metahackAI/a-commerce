//! Contract tests for the screens (closed schemas, screen names, no meaning in the schemas), and
//! a database test of a grid, its count, and a direct write.

use std::collections::BTreeSet;

use serde_json::{Map, Value, json};

use super::*;
use crate::codes::Grades;
use crate::screens::Paging;

fn facade() -> Facade {
    Facade::build().unwrap_or_else(|e| panic!("facade: {e:#?}"))
}

/// Every description in a schema, with the field it belongs to.
fn descriptions(schema: &Value, path: &str, out: &mut Vec<(String, String)>) {
    if let Some(d) = schema.get("description").and_then(Value::as_str) {
        out.push((path.to_owned(), d.to_owned()));
    }
    if let Some(Value::Object(props)) = schema.get("properties") {
        for (k, v) in props {
            descriptions(v, &format!("{path}.{k}"), out);
        }
    }
    if let Some(items) = schema.get("items") {
        descriptions(items, &format!("{path}[]"), out);
    }
}

#[test]
fn every_screen_has_a_closed_schema_and_a_screen_name() {
    let f = facade();
    assert!((15..=25).contains(&f.len()), "{} screens", f.len());
    for s in f.iter() {
        let m = s.meta();
        let schema = s.input_schema();
        assert_eq!(
            acom_contract::schema::violations(schema),
            Vec::<String>::new(),
            "{}",
            m.tool
        );
        assert_eq!(schema["type"], "object", "{}", m.tool);
        assert_eq!(schema["additionalProperties"], false, "{}", m.tool);
        assert!(
            m.tool.starts_with("get") || m.tool.starts_with("save"),
            "{}",
            m.tool
        );
        assert!(!m.tool.contains('_'), "{} reads like a tool name", m.tool);
        assert_eq!(
            m.method,
            acom_contract::Method::Post,
            "{}: reads are POST",
            m.tool
        );
        assert_eq!(m.path, format!("/v1/tools/{}", m.tool));
        assert!(m.checks.is_empty() && m.consumes.is_empty() && m.produces.is_empty());
        assert!(matches!(m.kind, Kind::Read | Kind::Write), "{}", m.tool);
    }
}

#[test]
fn schemas_carry_codes_but_never_their_meaning_or_a_rule() {
    // Words that would explain a code or hint at a policy.
    const TELLING: &[&str] = &[
        "vip",
        "gold",
        "everyone",
        "tier",
        "approval",
        "approve",
        "policy",
        "limit",
        "floor",
        "cost",
        "minimum",
        "maximum",
        "stack",
        "return window",
        "days",
        "must",
        "only",
        "percent",
        "in_stock",
        "running",
        "requested",
        "defect",
        "email",
        ":",
        ";",
        "(",
    ];
    let f = facade();
    for s in f.iter() {
        let m = s.meta();
        let mut all = vec![("summary".to_owned(), m.summary.to_owned())];
        descriptions(s.input_schema(), "$", &mut all);
        for (field, text) in all {
            let lower = text.to_lowercase();
            for w in TELLING {
                // "(v2)" is the one parenthesis a summary uses: the version twin.
                if *w == "(" && lower.contains("(v2)") {
                    continue;
                }
                assert!(
                    !lower.contains(w),
                    "{} {field}: `{text}` says `{w}`",
                    m.tool
                );
            }
            assert!(
                text.split_whitespace().count() <= 6,
                "{} {field}: `{text}` is not terse",
                m.tool
            );
        }
    }
}

#[test]
fn code_fields_list_codes_only() {
    let f = facade();
    let schema = f
        .get("savePrmtList")
        .map(|s| s.input_schema().clone())
        .unwrap();
    let row = &schema["properties"]["rows"]["items"]["properties"];
    assert_eq!(
        row["prmtTpCd"]["enum"],
        json!(["T01", "T02", "T03", "T04", "T05"])
    );
    assert_eq!(row["rowStat"]["enum"], json!(["I", "U", "D"]));
    assert_eq!(row["prmtTpCd"]["description"], "Promotion type code.");
    // No code list carries a description per value.
    let mut found = Vec::new();
    for s in f.iter() {
        let text = s.input_schema().to_string();
        assert!(!text.contains("\"oneOf\""), "{}", s.meta().tool);
        assert!(!text.contains("x-aliases"), "{}", s.meta().tool);
        assert!(!text.contains("examples"), "{}", s.meta().tool);
        if text.contains("\"enum\"") {
            found.push(s.meta().tool);
        }
    }
    assert!(found.len() >= 8, "{found:?}");
}

#[test]
fn the_patterns_are_all_there() {
    let f = facade();
    let names: BTreeSet<&str> = f.iter().map(|s| s.meta().tool).collect();
    // A grid read and its separate count.
    for grid in [
        "getPrdGridList",
        "getPrmtGridList",
        "getClmGridList",
        "getPrmtTgtMbrList",
    ] {
        assert!(names.contains(grid), "{grid}");
        assert!(names.contains(format!("{grid}Cnt").as_str()), "{grid}Cnt");
    }
    // A version twin, with the same input.
    let v1 = f.get("getPrdDtlInfo").unwrap();
    let v2 = f.get("getPrdDtlInfoV2").unwrap();
    assert_eq!(v1.input_schema(), v2.input_schema());
    // One refund across a grid, a popup init and a save.
    for s in ["getClmGridList", "getRfndPopInitData", "saveRfndProc"] {
        assert!(names.contains(s), "{s}");
    }
    // Bulk saves with a row status.
    for s in [
        "savePrmtList",
        "saveCpnList",
        "saveDispBnrList",
        "saveCartList",
    ] {
        let schema = f.get(s).unwrap().input_schema();
        assert_eq!(schema["required"], json!(["rows"]), "{s}");
        assert_eq!(
            schema["properties"]["rows"]["items"]["required"],
            json!(["rowStat"]),
            "{s}"
        );
    }
    // No screen answers a grid with its total: the count screen does.
    let manifest = f.manifest(|_| true);
    assert_eq!(manifest.len(), f.len());
}

#[test]
fn the_manifest_keeps_the_tool_shape() {
    let f = facade();
    let entries = f.manifest(|m| m.audience == acom_contract::Audience::Shopper);
    let names: Vec<&str> = entries.iter().map(|e| e.meta.tool).collect();
    assert_eq!(names, ["getCartList", "getOrdShtInitData", "saveCartList"]);
    let v = serde_json::to_value(&entries[0]).unwrap();
    for key in [
        "tool",
        "address",
        "kind",
        "summary",
        "input_schema",
        "audience",
    ] {
        assert!(v.get(key).is_some(), "{key}: {v}");
    }
    assert_eq!(v["kind"], "read");
}

#[test]
fn pages_become_cursors_and_grades_become_tiers() {
    let p = Paging::new(Some(3), Some(20)).unwrap();
    let mut q = Map::new();
    p.apply(&mut q);
    assert_eq!(q["limit"], 20);
    assert_eq!(q["cursor"], "o40");
    let mut first = Map::new();
    Paging::new(None, None).unwrap().apply(&mut first);
    assert!(first.get("cursor").is_none());
    assert!(Paging::new(Some(0), None).is_err());
    assert!(Paging::new(None, Some(101)).is_err());

    let g = Grades::from_tiers(&json!({"items": [
        {"id": "vip", "name": "VIP", "rank": 3, "member_count": 1284},
        {"id": "member", "name": "Member", "rank": 1, "member_count": 9816}]}));
    assert_eq!(g.code(Some("vip")), "G03");
    assert_eq!(g.code(None), "G00");
    assert_eq!(g.tier("G01").as_deref(), Some("member"));
    assert_eq!(g.tier("G00").as_deref(), Some("everyone"));
    assert_eq!(g.tier("G07"), None);
    assert_eq!(codes::prmt_tp("item_price", Some("vip")), "T03");
    assert_eq!(codes::prmt_tp("item_price", Some("everyone")), "T01");
    assert_eq!(codes::SellStatCd::code("out_of_stock"), "20");
}

#[test]
fn a_real_error_loses_the_tool_it_suggests() {
    let e = ToolError::new(ErrorCode::NOT_FOUND, "No such product.")
        .next("search_products")
        .detail(json!({"entity": "product"}));
    let l = legacy(e);
    assert_eq!(l.code, "not_found");
    assert_eq!(l.message, "No such product.");
    assert!(l.next_tool.is_none() && l.detail.is_none());
}

/// Database test against the demo store at `DATABASE_URL` (migrated and seeded), cleaning up
/// with `MIGRATION_DATABASE_URL`. Run on request:
/// `cargo test -p acom-facade -- --ignored --test-threads=1`.
#[tokio::test]
#[ignore = "integration: needs a PostgreSQL database"]
async fn a_grid_its_count_and_a_direct_write_against_the_demo_store() {
    use acom_auth::{Principal, Surface, scopes};
    use acom_core::TenantId;
    use acom_core::enums::ActorKind;

    const REQUEST: &str = "req_facade_it";
    let url = std::env::var("DATABASE_URL").expect("DATABASE_URL");
    let owner_url = std::env::var("MIGRATION_DATABASE_URL").expect("MIGRATION_DATABASE_URL");
    let db = acom_store::Db::connect(&url, 4).await.expect("connect");
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../policies");
    let texts: Vec<(&str, String)> = acom_policy::FILES
        .iter()
        .map(|f| {
            (
                *f,
                std::fs::read_to_string(format!("{dir}/{f}.yml")).unwrap(),
            )
        })
        .collect();
    let policy = acom_policy::PolicySet::from_defaults(texts.iter().map(|(f, t)| (*f, t.as_str())))
        .expect("policies");
    let runtime = acom_store::Runtime::new(db, policy, true);
    let registry = acom_registry::build().expect("registry");
    let f = facade();
    let operator = Principal {
        subject: "adm_sj".into(),
        kind: ActorKind::Operator,
        tenant: TenantId::parse("store_northwind").unwrap(),
        surface: Surface::Bo,
        scopes: scopes::admin(),
        name: Some("Son SeongJun".into()),
        session_id: Some("ses_facade_it".into()),
    }
    .on_behalf();
    let cx = runtime
        .context(operator.clone(), REQUEST.into())
        .await
        .expect("context");
    let guard = |m: &OpMeta| -> Result<(), ToolError> {
        let ok = match m.kind {
            Kind::Read => operator.can_read(m.domain()),
            _ => operator.can_write(m.domain()),
        };
        if ok {
            Ok(())
        } else {
            Err(ToolError::new(ErrorCode::FORBIDDEN, m.tool))
        }
    };
    let call = |tool: &'static str, input: Value| {
        let (f, registry, cx, guard) = (&f, &registry, &cx, &guard);
        async move { f.call(tool, registry, cx, guard, input).await }
    };

    // A grid read: rows with codes, and no total.
    let grid = call(
        "getPrdGridList",
        json!({"ctgNo": "cat_summer_outerwear", "pageSize": 5}),
    )
    .await
    .expect("grid");
    let rows = grid["list"].as_array().expect("list");
    assert_eq!(rows.len(), 5, "{grid:#}");
    assert!(grid.get("total").is_none() && grid.get("totCnt").is_none());
    assert!(
        rows[0]["prdNo"].as_str().unwrap().starts_with("prd_"),
        "{grid:#}"
    );
    assert!(rows.iter().all(|r| r["sellStatCd"].is_string()), "{grid:#}");
    let next = call(
        "getPrdGridList",
        json!({"ctgNo": "cat_summer_outerwear", "pageSize": 5, "pageNo": 2}),
    )
    .await
    .expect("page 2");
    assert_ne!(next["list"][0]["prdNo"], rows[0]["prdNo"]);

    // Its count: the total comes only from here.
    let cnt = call(
        "getPrdGridListCnt",
        json!({"ctgNo": "cat_summer_outerwear", "pageSize": 5}),
    )
    .await
    .expect("count");
    assert_eq!(cnt, json!({"totCnt": 42}));

    // A direct write: a member price row saves at once, with no preview and no change set.
    let name = format!("Facade test {}", acom_core::id::new_suffix());
    let saved = call(
        "savePrmtList",
        json!({"rows": [{"rowStat": "I", "prmtNm": name, "prmtTpCd": "T03", "tgtMbrGrdCd": "G03",
                         "strtDtm": "2026-12-05T00:00", "endDtm": "2026-12-06T23:59",
                         "aplyTgtCd": "P", "prdNoList": ["prd_windbreaker", "prd_field_jacket"],
                         "dcRt": 10}]}),
    )
    .await;
    let owner = sqlx::PgPool::connect(&owner_url)
        .await
        .expect("connect as owner");
    let promotion = saved
        .as_ref()
        .ok()
        .and_then(|s| s["list"][0]["prmtNo"].as_str())
        .map(str::to_owned);
    // Read back before cleaning up, then always clean up.
    let found = match &promotion {
        Some(_) => call(
            "getPrmtGridList",
            json!({"prmtNm": name, "tgtMbrGrdCd": "G03"}),
        )
        .await
        .ok(),
        None => None,
    };
    let previews: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM platform.preview WHERE tenant_id = 'store_northwind' AND request_id = $1",
    )
    .bind(REQUEST)
    .fetch_one(&owner)
    .await
    .expect("previews");
    if let Some(id) = &promotion {
        for sql in [
            "DELETE FROM promotion.unit_price_specification WHERE tenant_id = 'store_northwind' AND promotion_id = $1",
            "DELETE FROM promotion.promotion WHERE tenant_id = 'store_northwind' AND id = $1",
        ] {
            sqlx::query(sql)
                .bind(id)
                .execute(&owner)
                .await
                .expect("clean up");
        }
    }
    sqlx::query(
        "DELETE FROM platform.action_log WHERE tenant_id = 'store_northwind' AND request_id = $1",
    )
    .bind(REQUEST)
    .execute(&owner)
    .await
    .expect("clean up the log");

    let saved = saved.expect("save");
    assert_eq!(saved["resultCd"], "0000", "{saved:#}");
    assert_eq!(saved["procCnt"], 1);
    assert_eq!(previews, 0, "a screen write leaves no preview behind");
    let found = found.expect("read back");
    let row = &found["list"][0];
    assert_eq!(row["prmtNo"].as_str(), promotion.as_deref(), "{found:#}");
    assert_eq!(row["prmtTpCd"], "T03");
    assert_eq!(row["prmtStatCd"], "10", "scheduled: {found:#}");
}
