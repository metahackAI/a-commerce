//! Turn loop tests against a scripted model and a fake commerce service: no network, no
//! database, no real model.

use std::sync::Mutex;

use acom_auth::Surface;
use acom_core::enums::ActorKind;
use acom_core::{StoreTime, TenantId};
use acom_llm::{ChatResponse, DeltaSink, ModelInfo, Usage};
use futures::future::BoxFuture;

use super::*;
use crate::blocks::Action;
use crate::client::ChangeSetHead;
use crate::profile::fixtures::repo;
use crate::store::{FixedStore, MemoryLog};
use crate::tools::fixtures;

/// Answers each purpose (`route`, `plan`, `reply`, …) with the next scripted text for it.
#[derive(Debug)]
struct Scripted {
    info: ModelInfo,
    answers: Mutex<Vec<(String, String)>>,
    asked: Mutex<Vec<String>>,
}

impl Scripted {
    fn new(alias: &str, display: &str, answers: Vec<(&str, String)>) -> Arc<Self> {
        Arc::new(Self {
            info: ModelInfo {
                alias: alias.into(),
                model: format!("{alias}-test"),
                display_name: display.into(),
                price: None,
            },
            answers: Mutex::new(
                answers
                    .into_iter()
                    .map(|(p, a)| (p.to_owned(), a))
                    .collect(),
            ),
            asked: Mutex::new(Vec::new()),
        })
    }

    fn asked(&self) -> Vec<String> {
        self.asked.lock().unwrap().clone()
    }
}

impl ChatModel for Scripted {
    fn info(&self) -> &ModelInfo {
        &self.info
    }

    fn chat<'a>(
        &'a self,
        req: &'a ChatRequest,
        _: Option<DeltaSink<'a>>,
    ) -> BoxFuture<'a, Result<ChatResponse, LlmError>> {
        Box::pin(async move {
            self.asked.lock().unwrap().push(req.purpose.clone());
            let mut answers = self.answers.lock().unwrap();
            match answers.iter().position(|(p, _)| *p == req.purpose) {
                Some(i) => Ok(ChatResponse {
                    model: self.info.model.clone(),
                    text: answers.remove(i).1,
                    usage: Usage {
                        input_tokens: 100,
                        output_tokens: 20,
                    },
                    ..Default::default()
                }),
                None => Err(LlmError::Http {
                    status: 503,
                    message: "no scripted answer".into(),
                }),
            }
        })
    }
}

/// Commerce as the tools of scenes B1 and F2 behave, recording every call.
#[derive(Debug)]
struct Commerce {
    manifest: Vec<ToolSpec>,
    calls: Mutex<Vec<String>>,
    previews: Mutex<Vec<(String, Value, Option<String>)>>,
    counter: Mutex<u32>,
    version: Mutex<i32>,
}

impl Commerce {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            manifest: fixtures::manifest(),
            calls: Mutex::new(Vec::new()),
            previews: Mutex::new(Vec::new()),
            counter: Mutex::new(0),
            version: Mutex::new(1),
        })
    }

    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }

    fn previews(&self) -> Vec<(String, Value, Option<String>)> {
        self.previews.lock().unwrap().clone()
    }

    fn log(&self, s: String) {
        self.calls.lock().unwrap().push(s);
    }

    fn next_preview_id(&self) -> String {
        let mut n = self.counter.lock().unwrap();
        *n += 1;
        format!("pv_{n}")
    }
}

fn check(rule: &str, passed: bool, detail: &str) -> Value {
    json!({"rule": rule, "passed": passed, "detail": detail,
           "fix_tool": if passed { Value::Null } else { json!("set_sale_prices") }})
}

/// `set_sale_prices` as the B1 data behaves: the overshirt breaks the price floor at 20% and
/// two products are under the stock minimum, unless the input fixes both.
fn sale_preview(input: &Value) -> (Value, Value) {
    let excluded = input["exclude_low_stock"] == true;
    let overshirt = input["overrides"]
        .as_array()
        .and_then(|o| {
            o.iter()
                .find(|x| x["product_id"] == "prd_seersucker_overshirt")
        })
        .map_or(input["percent_off"].clone(), |x| x["percent_off"].clone());
    let mut changes = vec![
        json!({"op": "create", "entity": "promotion", "id": "", "to": {
            "name": input["name"], "valid_for_member_tier": input["valid_for_member_tier"],
            "valid_from": input["valid_from"], "valid_through": input["valid_through"]}}),
        json!({"op": "update", "entity": "unit_price_specification", "id": "",
            "from": {"product_id": "prd_windbreaker", "price": 89.00, "price_currency": "USD"},
            "to": {"product_id": "prd_windbreaker", "price": 71.20, "price_currency": "USD", "percent_off": input["percent_off"]}}),
        json!({"op": "update", "entity": "unit_price_specification", "id": "",
            "from": {"product_id": "prd_seersucker_overshirt", "price": 79.00, "price_currency": "USD"},
            "to": {"product_id": "prd_seersucker_overshirt", "percent_off": overshirt}}),
    ];
    if excluded {
        changes.push(
            json!({"op": "exclude", "entity": "product", "id": "prd_chore_coat",
                            "to": {"why": "promo_min_stock", "inventory_level": 7}}),
        );
    }
    let checks = json!([
        check(
            "max_discount",
            true,
            "Highest is 20%; policy allows 30% for vip"
        ),
        check(
            "price_floor",
            overshirt == json!(15),
            "Seersucker overshirt at 20% would be under the floor"
        ),
        check(
            "promo_min_stock",
            excluded,
            "Cotton chore coat (7) and Light rain parka (4) are under 10 units"
        ),
        check("promotion_overlap", true, "Checked 4 running promotions"),
    ]);
    (Value::Array(changes), checks)
}

impl ToolClient for Commerce {
    fn manifest(&self) -> BoxFuture<'_, Result<Vec<ToolSpec>, ToolError>> {
        self.log("manifest".into());
        Box::pin(async move { Ok(self.manifest.clone()) })
    }

    fn read<'a>(
        &'a self,
        tool: &'a str,
        input: &'a Value,
    ) -> BoxFuture<'a, Result<Value, ToolError>> {
        self.log(format!("read {tool}"));
        let _ = input;
        Box::pin(async move {
            Ok(match tool {
                "search_products" => json!({"items": [
                    {"product_id": "prd_windbreaker", "name": "Packable windbreaker", "price": 89.00, "price_currency": "USD", "weight_g": 190},
                    {"product_id": "prd_field_jacket", "name": "Linen blend field jacket", "price": 128.00, "price_currency": "USD", "weight_g": 340},
                    {"product_id": "prd_seersucker_overshirt", "name": "Seersucker overshirt", "price": 79.00, "price_currency": "USD", "weight_g": 360}
                ], "total": 42}),
                "list_tiers" => {
                    json!({"items": [{"id": "member"}, {"id": "gold"}, {"id": "vip", "member_count": 1284}], "total": 3})
                }
                "list_display_slots" => {
                    json!({"items": [{"id": "slot_home_hero", "name": "Main page hero"}], "total": 1})
                }
                "get_cart" => {
                    json!({"id": "cart_1", "lines": [{"cart_line_id": "cln_1", "name": "Packable windbreaker", "size": "M", "quantity": 1}],
                    "subtotal": {"value": 89.00, "currency": "USD"}, "total": {"value": 71.20, "currency": "USD"}})
                }
                _ => json!({"items": [], "total": 0}),
            })
        })
    }

    fn preview<'a>(
        &'a self,
        tool: &'a str,
        input: &'a Value,
        change_set_id: Option<&'a str>,
    ) -> BoxFuture<'a, Result<Preview, ToolError>> {
        self.log(format!(
            "preview {tool}{}",
            change_set_id
                .map(|c| format!(" into {c}"))
                .unwrap_or_default()
        ));
        self.previews.lock().unwrap().push((
            tool.to_owned(),
            input.clone(),
            change_set_id.map(str::to_owned),
        ));
        let id = self.next_preview_id();
        Box::pin(async move {
            let (changes, checks, needs_approval) = match tool {
                "set_sale_prices" => {
                    let (c, k) = sale_preview(input);
                    (c, k, true)
                }
                "create_coupon" => (
                    json!([{"op": "create", "entity": "coupon", "id": "", "to": {"name": input["name"], "promotion_id": input["promotion_id"]}}]),
                    json!([check(
                        "member_benefits_per_order",
                        true,
                        "One member benefit per order"
                    )]),
                    true,
                ),
                "schedule_banner" => (
                    json!([{"op": "schedule", "entity": "display_booking", "id": "", "to": {"slot_id": input["slot_id"], "audience": input["audience"]}}]),
                    json!([check("slot_free", true, "Hero slot open Nov 7–8 for VIP")]),
                    true,
                ),
                "prepare_order" => (
                    json!([{"op": "create", "entity": "order", "id": "", "to": {
                        "lines": [{"name": "Packable windbreaker", "size": "M", "quantity": 1}],
                        "subtotal": {"value": 89.00, "currency": "USD"},
                        "total": {"value": 71.20, "currency": "USD"}}}]),
                    json!([{"rule": "checkout_validation.stock_available", "passed": true, "detail": "In stock", "fix_tool": null}]),
                    false,
                ),
                _ => (
                    json!([{"op": "create", "entity": "thing", "id": "", "to": input}]),
                    json!([]),
                    false,
                ),
            };
            Ok(serde_json::from_value(json!({
                "preview_id": id, "expires_at": null, "changes": changes, "checks": checks,
                "needs_approval": needs_approval, "approval_reason": null, "change_set_id": change_set_id,
            }))
            .unwrap())
        })
    }

    fn commit<'a>(
        &'a self,
        tool: &'a str,
        preview_id: &'a str,
    ) -> BoxFuture<'a, Result<Value, ToolError>> {
        self.log(format!("commit {tool} {preview_id}"));
        Box::pin(async move {
            Ok(match tool {
                "add_to_cart" => json!({"ok": true, "id": "cart_1", "undo": null}),
                "update_my_preferences" => json!({"ok": true, "id": "mem_12",
                    "memory": {"memory_id": "mem_12", "text": "Remembered: weight and packed size matter"}}),
                _ => json!({"ok": true, "id": "x_1"}),
            })
        })
    }

    fn create_change_set<'a>(
        &'a self,
        new: &'a NewChangeSet,
    ) -> BoxFuture<'a, Result<ChangeSetHead, ToolError>> {
        self.log(format!("create_change_set {}", new.title));
        Box::pin(async move {
            Ok(ChangeSetHead {
                id: "cs_1042".into(),
                number: Some(1042),
                version: 1,
                state: "drafting".into(),
            })
        })
    }

    fn replace_items<'a>(
        &'a self,
        change_set_id: &'a str,
        items: &'a [ItemRef],
    ) -> BoxFuture<'a, Result<ChangeSetHead, ToolError>> {
        let ids: Vec<&str> = items.iter().map(|i| i.preview_id.as_str()).collect();
        self.log(format!("replace_items {change_set_id} {}", ids.join(",")));
        let version = {
            let mut v = self.version.lock().unwrap();
            *v += 1;
            *v
        };
        Box::pin(async move {
            Ok(ChangeSetHead {
                id: change_set_id.into(),
                number: None,
                version,
                state: "drafting".into(),
            })
        })
    }

    fn submit<'a>(
        &'a self,
        change_set_id: &'a str,
        version: i32,
    ) -> BoxFuture<'a, Result<ChangeSetHead, ToolError>> {
        self.log(format!("submit {change_set_id} v{version}"));
        Box::pin(async move {
            Ok(ChangeSetHead {
                id: change_set_id.into(),
                number: None,
                version,
                state: "waiting_for_approval".into(),
            })
        })
    }
}

fn settings() -> StoreSettings {
    let time = StoreTime::new(chrono_tz::America::New_York);
    StoreSettings {
        now: time.parse("2026-11-06T11:02").unwrap(),
        time,
        currency: "USD".into(),
    }
}

fn principal(subject: &str, surface: Surface) -> Principal {
    Principal {
        subject: subject.into(),
        kind: ActorKind::AgentOnBehalf,
        tenant: TenantId::parse("store_northwind").unwrap(),
        surface,
        scopes: [
            "changeset:draft",
            "tools:read:promotion",
            "tools:write:promotion",
        ]
        .iter()
        .map(|s| (*s).to_owned())
        .collect(),
        name: None,
        session_id: Some("ses_1".into()),
    }
}

fn runtime(models: &[Arc<Scripted>], log: Arc<MemoryLog>) -> Runtime {
    let set = ModelSet {
        models: models
            .iter()
            .map(|m| (m.info.alias.clone(), m.clone() as Arc<dyn ChatModel>))
            .collect(),
        missing: vec!["glimmer: set GLIMMER_BASE_URL, GLIMMER_MODEL".into()],
    };
    Runtime::new(Arc::new(repo()), set, log, Arc::new(FixedStore(settings())))
}

async fn turn(
    rt: &Runtime,
    who: Principal,
    surface: ChatSurface,
    request: TurnRequest,
    commerce: Arc<Commerce>,
) -> Vec<Event> {
    let p = rt
        .prepare(who, "req_test".into(), surface, request)
        .await
        .unwrap();
    let (emit, mut rx) = Emitter::channel();
    rt.run(p, commerce, emit).await;
    let mut events = Vec::new();
    while let Ok(e) = rx.try_recv() {
        events.push(e);
    }
    events
}

fn say(text: &str) -> TurnRequest {
    TurnRequest {
        message: Some(text.into()),
        ..TurnRequest::default()
    }
}

fn names(events: &[Event]) -> Vec<&'static str> {
    events.iter().map(Event::name).collect()
}

fn blocks(events: &[Event]) -> Vec<&Block> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::Block(b) => Some(b.as_ref()),
            _ => None,
        })
        .collect()
}

fn done(events: &[Event]) -> (TurnUsage, Vec<String>) {
    match events.last() {
        Some(Event::TurnDone { usage, models, .. }) => (usage.clone(), models.clone()),
        other => panic!("the last event is not turn.done: {other:?}"),
    }
}

fn message(events: &[Event]) -> String {
    events
        .iter()
        .find_map(|e| match e {
            Event::MessageDone { text, .. } => Some(text.clone()),
            _ => None,
        })
        .unwrap_or_default()
}

const B1: &str =
    "Run a 20% weekend sale on summer outerwear for VIP members and put it on the main page.";

fn b1_plan() -> String {
    json!({
        "title": "VIP weekend",
        "interpretation": [
            {"aspect": "who", "text": "VIP members"}, {"aspect": "products", "text": "Summer outerwear"},
            {"aspect": "benefit", "text": "20% off"}, {"aspect": "when", "text": "This weekend, Saturday and Sunday"},
            {"aspect": "where", "text": "Main page hero, for VIP members"}],
        "about": "self",
        "steps": [
            {"id": "s1", "tool": "search_products", "args": {"category": "summer_outerwear", "limit": 100}, "uses": []},
            {"id": "s2", "tool": "list_tiers", "args": {}, "uses": []},
            {"id": "s3", "tool": "set_sale_prices", "args": {"name": "VIP weekend", "valid_for_member_tier": "VIP members",
                "valid_from": "Saturday", "valid_through": "Sunday", "scope_mode": "products", "percent_off": "20%"},
             "uses": ["s1", "s2"]},
            {"id": "s4", "tool": "create_coupon", "args": {"name": "VIP weekend"}, "uses": ["s3"]},
            {"id": "s5", "tool": "list_display_slots", "args": {"page": "home"}, "uses": []},
            {"id": "s6", "tool": "schedule_banner", "args": {"audience": "vip"}, "uses": ["s5", "s3"]}
        ],
        "reply_plan": "change_set"
    })
    .to_string()
}

#[tokio::test]
async fn without_a_model_a_turn_says_what_to_configure_and_ends() {
    let log = Arc::new(MemoryLog::default());
    let rt = runtime(&[], log.clone());
    let commerce = Commerce::new();
    let events = turn(
        &rt,
        principal("adm_sj", Surface::Bo),
        ChatSurface::Bo,
        say(B1),
        commerce.clone(),
    )
    .await;
    assert_eq!(
        names(&events),
        ["turn.started", "agent.step", "error", "turn.done"]
    );
    match &events[2] {
        Event::Error { code, message } => {
            assert_eq!(code, "upstream_unavailable");
            assert!(message.starts_with("No model is configured: "), "{message}");
            assert!(message.contains("GLIMMER_BASE_URL"), "{message}");
        }
        other => panic!("{other:?}"),
    }
    assert!(
        commerce.calls().is_empty(),
        "no commerce call without a model"
    );
    let ended = log.ended();
    assert_eq!(ended.len(), 1);
    assert_eq!(
        ended[0].error.as_ref().unwrap()["code"],
        "upstream_unavailable"
    );
}

#[tokio::test]
async fn b1_drafts_a_change_set_through_tools_and_never_approves() {
    let glimmer = Scripted::new(
        "glimmer",
        "Muse Glimmer 30B",
        vec![
            ("route", json!({"agent": "promotion", "confidence": 0.93}).to_string()),
            ("plan", b1_plan()),
            ("reply", "I drafted change set #1042 for the 20% VIP weekend sale. Two checks need your decision before it can go for approval.".into()),
        ],
    );
    let log = Arc::new(MemoryLog::default());
    let rt = runtime(std::slice::from_ref(&glimmer), log.clone());
    let commerce = Commerce::new();
    let events = turn(
        &rt,
        principal("adm_sj", Surface::Bo),
        ChatSurface::Bo,
        say(B1),
        commerce.clone(),
    )
    .await;

    assert_eq!(events[0].name(), "turn.started");
    assert_eq!(events[0].data()["agent"], "promotion");
    assert_eq!(
        commerce.calls(),
        [
            "manifest",
            "read search_products",
            "read list_tiers",
            "create_change_set VIP weekend",
            "preview set_sale_prices into cs_1042",
            "preview create_coupon into cs_1042",
            "read list_display_slots",
            "preview schedule_banner into cs_1042",
        ],
        "reads, then every write previewed into one change set; no submit with failed checks, no approval"
    );

    let previews = commerce.previews();
    let sale = &previews[0].1;
    assert_eq!(sale["valid_for_member_tier"], "vip");
    assert_eq!(sale["valid_from"], "2026-11-07T00:00:00-05:00");
    assert_eq!(sale["valid_through"], "2026-11-08T23:59:00-05:00");
    assert_eq!(sale["percent_off"], 20);
    assert_eq!(
        sale["product_ids"],
        json!([
            "prd_windbreaker",
            "prd_field_jacket",
            "prd_seersucker_overshirt"
        ]),
        "ids come from the search result, not from the model"
    );
    assert_eq!(previews[1].1["promotion_id"], "$item[1].promotion_id");
    assert_eq!(previews[2].1["slot_id"], "slot_home_hero");
    assert_eq!(previews[2].1["promotion_id"], "$item[1].promotion_id");

    let b = blocks(&events);
    assert_eq!(b.len(), 1);
    assert_eq!(b[0].ui, "change_set");
    assert_eq!(b[0].data["change_set_id"], "cs_1042");
    assert_eq!(b[0].data["state"], "drafting");
    assert_eq!(b[0].data["understood"]["who"], "VIP members");
    assert_eq!(
        b[0].person_actions().count(),
        0,
        "a draft with failed checks cannot be approved"
    );
    assert!(b[0].suggested_intents.iter().any(|i| i.element == "check_row:promo_min_stock" && i.say == "Drop the low-stock ones"));

    let highlight = events.iter().find_map(|e| match e {
        Event::UiCommand {
            command: Command::Highlight,
            target,
            ..
        } => Some(target.clone()),
        _ => None,
    });
    assert_eq!(
        highlight,
        Some(format!("{}#check_row:price_floor", b[0].block_id))
    );
    assert!(events.iter().any(
        |e| matches!(e, Event::ChangeSetUpdated { version: 1, state, .. } if state == "drafting")
    ));
    assert!(message(&events).starts_with("I drafted change set #1042"));

    let (usage, models) = done(&events);
    assert_eq!(usage.tool_calls, 6);
    assert_eq!(usage.model_calls, 3);
    assert_eq!(usage.input_tokens, 300);
    assert_eq!(models, ["Muse Glimmer 30B"]);
    assert_eq!(glimmer.asked(), ["route", "plan", "reply"]);

    let calls = log.model_calls();
    assert_eq!(
        calls.iter().map(|c| c.purpose.as_str()).collect::<Vec<_>>(),
        ["route", "plan", "reply"]
    );
    let steps = log.steps();
    assert!(steps.iter().any(|s| s.kind == "call_tool"
        && s.preview_id.is_some()
        && s.change_set_id.as_deref() == Some("cs_1042")));
    assert!(
        steps.iter().all(|s| s.status != "active"),
        "every step ended"
    );
}

#[tokio::test]
async fn b1_correction_replaces_the_item_and_hands_version_2_to_people() {
    let glimmer = Scripted::new(
        "glimmer",
        "Muse Glimmer 30B",
        vec![
            ("route", json!({"agent": "promotion", "confidence": 0.9}).to_string()),
            ("plan", b1_plan()),
            ("reply", "I drafted change set #1042 for the 20% sale.".into()),
            ("route", json!({"agent": "promotion", "confidence": 0.9}).to_string()),
            ("plan", json!({
                "interpretation": [{"aspect": "products", "text": "Without the low-stock ones"},
                                   {"aspect": "benefit", "text": "Overshirt at 15%"}],
                "about": "self",
                "steps": [{"id": "s1", "tool": "set_sale_prices", "uses": [],
                           "args": {"exclude_low_stock": true,
                                    "overrides": [{"product_id": "prd_seersucker_overshirt", "percent_off": 15}]}}],
                "reply_plan": "change_set"}).to_string()),
            ("reply", "Version 2 of change set #1042 passes every check and is waiting for your approval.".into()),
        ],
    );
    let log = Arc::new(MemoryLog::default());
    let rt = runtime(&[glimmer], log);
    let commerce = Commerce::new();
    let jamie_sj = principal("adm_sj", Surface::Bo);
    let first = turn(
        &rt,
        jamie_sj.clone(),
        ChatSurface::Bo,
        say(B1),
        commerce.clone(),
    )
    .await;
    let conversation = first[0].data()["conversation_id"]
        .as_str()
        .unwrap()
        .to_owned();

    let mut fix = say("Drop the low-stock ones. Give the overshirt 15% instead.");
    fix.conversation_id = Some(conversation.clone());
    let events = turn(&rt, jamie_sj, ChatSurface::Bo, fix, commerce.clone()).await;

    let calls = commerce.calls();
    let second: Vec<&str> = calls[8..].iter().map(String::as_str).collect();
    assert_eq!(
        second,
        [
            "manifest",
            "preview set_sale_prices",
            "replace_items cs_1042 pv_4,pv_2,pv_3",
            "submit cs_1042 v2"
        ],
        "the correction is previewed again and replaces item 1; the set goes to people, never approved"
    );
    let corrected = &commerce.previews()[3].1;
    assert_eq!(corrected["exclude_low_stock"], true);
    assert_eq!(corrected["percent_off"], 20, "the rest of the item is kept");
    assert_eq!(corrected["valid_for_member_tier"], "vip");
    assert_eq!(corrected["product_ids"].as_array().unwrap().len(), 3);

    let b = blocks(&events);
    assert_eq!(b[0].data["version"], 2);
    assert_eq!(b[0].data["state"], "waiting_for_approval");
    let approve = b[0]
        .actions
        .iter()
        .find_map(|a| match a {
            Action::Person {
                action_id,
                path,
                body,
                ..
            } if action_id == "approve" => Some((path.clone(), body.clone())),
            _ => None,
        })
        .expect("an approve button for the person");
    assert_eq!(
        approve,
        (
            "/v1/platform/change-sets/cs_1042/approve".to_owned(),
            json!({"version": 2})
        )
    );
    assert!(events.iter().any(
        |e| matches!(e, Event::ChangeSetUpdated { state, .. } if state == "waiting_for_approval")
    ));
    assert!(message(&events).contains("waiting for your approval"));
}

#[tokio::test]
async fn shoppers_cart_writes_run_and_orders_are_only_prepared() {
    let glimmer = Scripted::new(
        "glimmer",
        "Muse Glimmer 30B",
        vec![(
            "plan",
            json!({"interpretation": [], "about": "self", "reply_plan": "checkout_card", "steps": [
                {"id": "s1", "tool": "search_products", "args": {"query": "windbreaker"}, "uses": []},
                {"id": "s2", "tool": "add_to_cart", "args": {"size": "M"}, "uses": ["s1"]},
                {"id": "s3", "tool": "get_cart", "args": {}, "uses": []},
                {"id": "s4", "tool": "prepare_order", "args": {"delivery_mode": "saved"}, "uses": ["s3"]}]})
            .to_string(),
        )],
    );
    let log = Arc::new(MemoryLog::default());
    let rt = runtime(std::slice::from_ref(&glimmer), log);
    let commerce = Commerce::new();
    let events = turn(
        &rt,
        principal("cus_jamie", Surface::Fo),
        ChatSurface::Fo,
        say("Add the windbreaker in M to my cart and get my order ready."),
        commerce.clone(),
    )
    .await;
    assert_eq!(
        glimmer.asked()[0],
        "plan",
        "one storefront agent: no routing call"
    );
    assert_eq!(
        commerce.calls(),
        [
            "manifest",
            "read search_products",
            "preview add_to_cart",
            "commit add_to_cart pv_1",
            "read get_cart",
            "preview prepare_order"
        ],
        "the cart write runs with its preview id; the order is only prepared"
    );
    let prepared = &commerce.previews()[1].1;
    assert_eq!(
        prepared["client_total"]["value"].to_string(),
        "71.2",
        "the total comes from the cart"
    );
    let card = blocks(&events)
        .into_iter()
        .find(|b| b.ui == "checkout_card")
        .expect("checkout card");
    match &card.actions[..] {
        [
            Action::Person {
                path, body, label, ..
            },
        ] => {
            assert_eq!(path, "/v1/sales/orders");
            assert_eq!(body, &json!({"preview_id": "pv_2"}));
            assert_eq!(label, "Place order · $71.20");
        }
        other => panic!("{other:?}"),
    }
    // No scripted reply: the plain reply stands in.
    assert!(message(&events).contains("only you can place it"));
    assert_eq!(done(&events).0.tool_calls, 5);
}

#[tokio::test]
async fn nothing_is_remembered_about_a_gift_recipient_and_made_up_numbers_are_dropped() {
    let glimmer = Scripted::new(
        "glimmer",
        "Muse Glimmer 30B",
        vec![
            ("plan", json!({"interpretation": [{"aspect": "who", "text": "Dad, who hikes"}], "about": "other",
                "reply_plan": "product_list", "steps": [
                {"id": "s1", "tool": "search_products", "args": {"query": "light hiking jacket"}, "uses": []},
                {"id": "s2", "tool": "update_my_preferences", "args": {"add": ["likes hiking"]}, "uses": []}]})
            .to_string()),
            ("reply", "These three are great, and $19.99 cheaper than last week.".into()),
        ],
    );
    let rt = runtime(&[glimmer], Arc::new(MemoryLog::default()));
    let commerce = Commerce::new();
    let events = turn(
        &rt,
        principal("cus_jamie", Surface::Fo),
        ChatSurface::Fo,
        say("Remember that my dad likes hiking and find him a light jacket."),
        commerce.clone(),
    )
    .await;
    assert!(
        !commerce
            .calls()
            .iter()
            .any(|c| c.contains("update_my_preferences"))
    );
    assert!(!events.iter().any(|e| e.name() == "memory.saved"));
    let b = blocks(&events);
    assert_eq!(
        b[0].ui, "look_gallery",
        "a first visit is shown as photos (shopper.yml)"
    );
    let text = message(&events);
    assert!(!text.contains("19.99"), "{text}");
    assert!(text.contains("did not remember anything"), "{text}");
}

#[tokio::test]
async fn a_remembered_preference_is_shown_with_the_shoppers_undo() {
    let glimmer = Scripted::new(
        "glimmer",
        "Muse Glimmer 30B",
        vec![(
            "plan",
            json!({"about": "self", "interpretation": [], "reply_plan": "memory_note", "steps": [
                {"id": "s1", "tool": "update_my_preferences", "args": {"add": ["weight and packed size"]}, "uses": []}]})
            .to_string(),
        )],
    );
    let rt = runtime(&[glimmer], Arc::new(MemoryLog::default()));
    let commerce = Commerce::new();
    let events = turn(
        &rt,
        principal("cus_jamie", Surface::Fo),
        ChatSurface::Fo,
        say("Remember that weight and packed size matter to me when I travel."),
        commerce.clone(),
    )
    .await;
    assert_eq!(
        commerce.calls(),
        [
            "manifest",
            "preview update_my_preferences",
            "commit update_my_preferences pv_1"
        ]
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::MemorySaved { memory_id, .. } if memory_id == "mem_12"))
    );
    let note = blocks(&events)
        .into_iter()
        .find(|b| b.ui == "memory_note")
        .expect("memory note");
    assert!(
        matches!(&note.actions[..], [Action::Person { path, .. }] if path == "/v1/customer/me/memories/mem_12/undo")
    );
}

#[tokio::test]
async fn a_conversation_is_only_continued_by_its_owner() {
    let glimmer = Scripted::new(
        "glimmer",
        "Muse Glimmer 30B",
        vec![
            (
                "route",
                json!({"agent": "promotion", "confidence": 0.9}).to_string(),
            ),
            ("plan", b1_plan()),
        ],
    );
    let rt = runtime(&[glimmer], Arc::new(MemoryLog::default()));
    let first = turn(
        &rt,
        principal("adm_sj", Surface::Bo),
        ChatSurface::Bo,
        say(B1),
        Commerce::new(),
    )
    .await;
    let conversation = first[0].data()["conversation_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let mut req = say("Approve it.");
    req.conversation_id = Some(conversation);
    let e = rt
        .prepare(
            principal("adm_other", Surface::Bo),
            "req_2".into(),
            ChatSurface::Bo,
            req,
        )
        .await
        .unwrap_err();
    assert_eq!(e.code, "not_found");

    let bad = TurnRequest::default();
    assert_eq!(bad.validate().unwrap_err().code, "invalid_input");
    let mut bad = say("hi");
    bad.conversation_id = Some("cs_1042".into());
    assert_eq!(bad.validate().unwrap_err().code, "invalid_input");
}

#[tokio::test]
async fn a_failing_tool_with_a_next_tool_revises_the_plan_once_more() {
    #[derive(Debug)]
    struct Failing(Arc<Commerce>);
    impl ToolClient for Failing {
        fn manifest(&self) -> BoxFuture<'_, Result<Vec<ToolSpec>, ToolError>> {
            self.0.manifest()
        }
        fn read<'a>(
            &'a self,
            tool: &'a str,
            input: &'a Value,
        ) -> BoxFuture<'a, Result<Value, ToolError>> {
            if tool == "search_products" && input.get("category").is_some() {
                self.0.log("read search_products failed".into());
                return Box::pin(async {
                    Err(ToolError::new(
                        ErrorCode::NOT_FOUND,
                        "There is no category summer_jackets.",
                    )
                    .next("list_tiers"))
                });
            }
            self.0.read(tool, input)
        }
        fn preview<'a>(
            &'a self,
            t: &'a str,
            i: &'a Value,
            c: Option<&'a str>,
        ) -> BoxFuture<'a, Result<Preview, ToolError>> {
            self.0.preview(t, i, c)
        }
        fn commit<'a>(&'a self, t: &'a str, p: &'a str) -> BoxFuture<'a, Result<Value, ToolError>> {
            self.0.commit(t, p)
        }
        fn create_change_set<'a>(
            &'a self,
            n: &'a NewChangeSet,
        ) -> BoxFuture<'a, Result<ChangeSetHead, ToolError>> {
            self.0.create_change_set(n)
        }
        fn replace_items<'a>(
            &'a self,
            c: &'a str,
            i: &'a [ItemRef],
        ) -> BoxFuture<'a, Result<ChangeSetHead, ToolError>> {
            self.0.replace_items(c, i)
        }
        fn submit<'a>(
            &'a self,
            c: &'a str,
            v: i32,
        ) -> BoxFuture<'a, Result<ChangeSetHead, ToolError>> {
            self.0.submit(c, v)
        }
    }
    let glimmer = Scripted::new(
        "glimmer",
        "Muse Glimmer 30B",
        vec![
            ("plan", json!({"about": "self", "interpretation": [], "reply_plan": "product_list",
                "steps": [{"id": "s1", "tool": "search_products", "args": {"category": "summer_jackets"}, "uses": []}]}).to_string()),
            ("plan", json!({"about": "self", "interpretation": [], "reply_plan": "product_list",
                "steps": [{"id": "s1", "tool": "search_products", "args": {"query": "light jacket"}, "uses": []}]}).to_string()),
        ],
    );
    let spark = Scripted::new("spark", "Muse Spark", vec![]);
    let rt = runtime(
        &[glimmer.clone(), spark.clone()],
        Arc::new(MemoryLog::default()),
    );
    let commerce = Commerce::new();
    let failing = Arc::new(Failing(commerce.clone()));
    let p = rt
        .prepare(
            principal("cus_jamie", Surface::Fo),
            "req_3".into(),
            ChatSurface::Fo,
            say("Find me a light jacket"),
        )
        .await
        .unwrap();
    let (emit, mut rx) = Emitter::channel();
    rt.run(p, failing, emit).await;
    let mut events = Vec::new();
    while let Ok(e) = rx.try_recv() {
        events.push(e);
    }
    assert_eq!(
        commerce.calls(),
        [
            "manifest",
            "read search_products failed",
            "read search_products"
        ]
    );
    // The revision asks the planner model first; with no scripted answer there, the worker plans.
    assert_eq!(spark.asked(), ["plan", "reply"]);
    assert_eq!(glimmer.asked(), ["plan", "plan", "reply"]);
    assert!(events.iter().any(|e| matches!(e, Event::Step { kind: StepKind::Plan, summary, .. } if summary.starts_with("Changing the plan"))));
    assert_eq!(blocks(&events)[0].ui, "look_gallery");
}
