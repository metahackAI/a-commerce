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
    /// The last message of each reply call: what the reply was written from.
    reply_inputs: Mutex<Vec<String>>,
    streams_nothing: std::sync::atomic::AtomicBool,
    /// Streams the background check's request but nothing in a turn (the demo's endpoint did).
    streams_check_only: std::sync::atomic::AtomicBool,
    /// Answers a reply held to a schema with a server error, as the demo's endpoint once did.
    fails_on_reply_schema: std::sync::atomic::AtomicBool,
}

impl Scripted {
    fn new(alias: &str, display: &str, answers: Vec<(&str, String)>) -> Arc<Self> {
        Arc::new(Self {
            info: ModelInfo {
                alias: alias.into(),
                model: format!("{alias}-test"),
                display_name: display.into(),
                price: None,
                stream_variants: acom_llm::StreamVariant::ALL.to_vec(),
            },
            answers: Mutex::new(
                answers
                    .into_iter()
                    .map(|(p, a)| (p.to_owned(), a))
                    .collect(),
            ),
            asked: Mutex::new(Vec::new()),
            reply_inputs: Mutex::new(Vec::new()),
            streams_nothing: std::sync::atomic::AtomicBool::new(false),
            streams_check_only: std::sync::atomic::AtomicBool::new(false),
            fails_on_reply_schema: std::sync::atomic::AtomicBool::new(false),
        })
    }

    fn failing_on_reply_schema(self: Arc<Self>) -> Arc<Self> {
        self.fails_on_reply_schema
            .store(true, std::sync::atomic::Ordering::SeqCst);
        self
    }

    fn streaming_the_check_only(self: Arc<Self>) -> Arc<Self> {
        self.streams_check_only
            .store(true, std::sync::atomic::Ordering::SeqCst);
        self
    }

    /// Answers a streamed call with no text, as a provider that streams structured output
    /// apart does; a call without streaming gets the scripted answer.
    fn streaming_nothing(self: Arc<Self>) -> Arc<Self> {
        self.streams_nothing
            .store(true, std::sync::atomic::Ordering::SeqCst);
        self
    }

    fn asked(&self) -> Vec<String> {
        self.asked.lock().unwrap().clone()
    }

    /// One more answer, written once an earlier turn has run (a block id it showed).
    fn will_answer(&self, purpose: &str, answer: String) {
        self.answers
            .lock()
            .unwrap()
            .push((purpose.to_owned(), answer));
    }

    fn reply_inputs(&self) -> Vec<String> {
        self.reply_inputs.lock().unwrap().clone()
    }
}

impl ChatModel for Scripted {
    fn info(&self) -> &ModelInfo {
        &self.info
    }

    fn chat<'a>(
        &'a self,
        req: &'a ChatRequest,
        on_delta: Option<DeltaSink<'a>>,
    ) -> BoxFuture<'a, Result<ChatResponse, LlmError>> {
        Box::pin(async move {
            self.asked.lock().unwrap().push(req.purpose.clone());
            if req.purpose == "reply"
                && let Some(m) = req.messages.last()
            {
                self.reply_inputs.lock().unwrap().push(m.content.clone());
            }
            // The background streaming check: any model can answer it.
            if req.purpose == "stream_check" {
                let streams = !self
                    .streams_nothing
                    .load(std::sync::atomic::Ordering::SeqCst);
                let text = if streams && on_delta.is_some() {
                    "{\"text\": \"Ready.\"}".to_owned()
                } else {
                    String::new()
                };
                if let Some(sink) = on_delta {
                    for piece in [&text[..text.len().min(10)], &text[text.len().min(10)..]] {
                        if !piece.is_empty() {
                            sink(piece);
                        }
                    }
                }
                return Ok(ChatResponse {
                    model: self.info.model.clone(),
                    text,
                    ..Default::default()
                });
            }
            if req.purpose == "reply"
                && req.output.is_some()
                && req.stream_variant != acom_llm::StreamVariant::PlainText
                && self
                    .fails_on_reply_schema
                    .load(std::sync::atomic::Ordering::SeqCst)
            {
                return Err(LlmError::Http {
                    status: 500,
                    message: "Internal server error".into(),
                });
            }
            if on_delta.is_some()
                && (self
                    .streams_nothing
                    .load(std::sync::atomic::Ordering::SeqCst)
                    || self
                        .streams_check_only
                        .load(std::sync::atomic::Ordering::SeqCst))
            {
                return Ok(ChatResponse {
                    model: self.info.model.clone(),
                    ..Default::default()
                });
            }
            let mut answers = self.answers.lock().unwrap();
            match answers.iter().position(|(p, _)| *p == req.purpose) {
                Some(i) => Ok(ChatResponse {
                    model: self.info.model.clone(),
                    text: {
                        let text = answers.remove(i).1;
                        // Streamed as a provider would: in small pieces, cut anywhere.
                        if let Some(sink) = on_delta {
                            let chars: Vec<char> = text.chars().collect();
                            for piece in chars.chunks(7) {
                                sink(&piece.iter().collect::<String>());
                            }
                        }
                        text
                    },
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

/// Commerce as the tools of scenes B1, B3 and F2 behave, recording every call.
#[derive(Debug)]
struct Commerce {
    manifest: Vec<ToolSpec>,
    /// The screen-shaped facade's manifest (`GET /v1/tools?set=facade`).
    facade: Vec<ToolSpec>,
    calls: Mutex<Vec<String>>,
    reads: Mutex<Vec<(String, Value)>>,
    previews: Mutex<Vec<(String, Value, Option<String>)>>,
    counter: Mutex<u32>,
    version: Mutex<i32>,
}

impl Commerce {
    fn new() -> Arc<Self> {
        Self::with_manifest(fixtures::manifest())
    }

    fn with_manifest(manifest: Vec<ToolSpec>) -> Arc<Self> {
        Arc::new(Self {
            manifest,
            facade: facade_manifest(),
            calls: Mutex::new(Vec::new()),
            reads: Mutex::new(Vec::new()),
            previews: Mutex::new(Vec::new()),
            counter: Mutex::new(0),
            version: Mutex::new(1),
        })
    }

    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }

    fn reads(&self) -> Vec<(String, Value)> {
        self.reads.lock().unwrap().clone()
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
        self.reads
            .lock()
            .unwrap()
            .push((tool.to_owned(), input.clone()));
        Box::pin(async move {
            Ok(match tool {
                "explain_sales_change" => crate::blocks::insights::tests::b3_explain(),
                "get_sales_trend" => crate::blocks::insights::tests::b3_trend(),
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
                "get_bot_status" => json!({"settings": {"mode": "propose", "model": "glimmer",
                    "max_proposals_per_cycle": 2, "directives": ["Never discount new arrivals."],
                    "areas": {"pricing": true, "inventory": false, "promotions": false, "merchandising": false,
                              "catalog_content": false, "returns": true, "questions_reviews": false, "community": false}},
                    "acts_as": "adm_sj"}),
                "search_return_requests" => {
                    json!({"items": [{"claim_id": "clm_20402", "amount": {"value": 240.0, "currency": "USD"}}], "total": 12})
                }
                "get_cart" => {
                    json!({"id": "cart_1", "lines": [{"cart_line_id": "cln_1", "name": "Packable windbreaker", "size": "M", "quantity": 1}],
                    "subtotal": {"value": 89.00, "currency": "USD"}, "total": {"value": 71.20, "currency": "USD"}})
                }
                "get_my_orders" => json!({"items": [
                    {"id": "ord_20268", "order_number": "20268", "order_status": "delivered",
                     "order_date": "2026-09-08T09:14:00-04:00",
                     "ordered_item": [{"id": "oit_1", "product_id": "prd_windbreaker", "name": "Packable windbreaker",
                                       "size": "M", "color": "Black", "order_quantity": 1}],
                     "total": {"value": 89.00, "currency": "USD"}}], "total": 1}),
                "get_order" if input["order_id"] == "ord_20105" => {
                    json!({"id": "ord_20105", "order_number": "20105",
                    "order_status": "delivered", "delivery": {"id": "dlv_20105"},
                    "ordered_item": [{"id": "oit_9", "product_id": "prd_chore_coat", "name": "Cotton chore coat", "order_quantity": 1}]})
                }
                "get_order" => {
                    json!({"id": "ord_20268", "order_number": "20268", "order_status": "delivered",
                    "delivery": {"id": "dlv_20268"},
                    "ordered_item": [{"id": "oit_1", "product_id": "prd_windbreaker", "name": "Packable windbreaker",
                                      "size": "M", "color": "Black", "order_quantity": 1}],
                    "totals": {"total": {"value": 89.00, "currency": "USD"}}})
                }
                "track_shipment" => {
                    json!({"items": [{"at": "2026-09-11T15:05:00-04:00", "delivery_state": "delivered"}],
                    "total": 1, "delivery_id": "dlv_20268", "carrier": "UPS", "tracking_number": "1Z1"})
                }
                "get_payment" => {
                    json!({"payment_status": "payment_complete", "label": "Card ending 4821",
                    "amount": {"value": 89.00, "currency": "USD"}})
                }
                "list_faqs" => json!({"items": [
                    {"id": "faq_return_days", "name": "How long do I have to return something?", "category": "returns"}], "total": 1}),
                "list_reviews" => json!({"items": [
                    {"id": "rev_1", "product_id": "prd_windbreaker", "review_rating": 5,
                     "author": {"name": "Jordan P."}, "review_body": "Packs into its pocket."}], "total": 1}),
                "list_questions" => json!({"items": [
                    {"id": "qst_1", "product_id": "prd_windbreaker", "text": "Does the hood roll in?",
                     "accepted_answer": {"text": "Yes, into the collar."}}], "total": 1}),
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
                "batch_import" => (
                    serde_json::to_value(&crate::blocks::drafts::fixtures::b4_preview().changes)
                        .unwrap(),
                    serde_json::to_value(&crate::blocks::drafts::fixtures::b4_preview().checks)
                        .unwrap(),
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
        match &new.task_id {
            Some(task) => self.log(format!("create_change_set {} for {task}", new.title)),
            None => self.log(format!("create_change_set {}", new.title)),
        }
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

    fn store(&self) -> BoxFuture<'_, Result<crate::store::StoreSettings, ToolError>> {
        Box::pin(async {
            Ok(crate::store::StoreSettings {
                time: settings().time,
                currency: "USD".into(),
                now: settings().now,
            })
        })
    }

    fn facade_manifest(&self) -> BoxFuture<'_, Result<Vec<ToolSpec>, ToolError>> {
        self.log("facade_manifest".into());
        Box::pin(async move { Ok(self.facade.clone()) })
    }

    fn facade_call<'a>(
        &'a self,
        tool: &'a str,
        input: &'a Value,
    ) -> BoxFuture<'a, Result<Value, ToolError>> {
        self.log(format!("facade {tool}"));
        self.reads
            .lock()
            .unwrap()
            .push((tool.to_owned(), input.clone()));
        Box::pin(async move {
            Ok(match tool {
                "getPrmtGridList" => json!({"list": [
                    {"prmtNo": "promo_autumn_knit", "prmtNm": "Autumn knitwear 15%", "prmtTpCd": "T01", "prmtStatCd": "20"},
                    {"prmtNo": "promo_gold_10", "prmtNm": "Gold tier 10%", "prmtTpCd": "T02", "prmtStatCd": "20"}],
                    "pageNo": 1, "pageSize": 20}),
                "getMbrGrdList" => json!({"list": [
                    {"mbrGrdCd": "G03", "mbrGrdNm": "VIP", "mbrCnt": 1284},
                    {"mbrGrdCd": "G02", "mbrGrdNm": "Gold", "mbrCnt": 2900}]}),
                "savePrmtList" => json!({"resultCd": "0000", "resultMsg": "Saved.", "procCnt": 1,
                    "list": [{"rowNo": 1, "rowStat": "I", "prmtNo": "promo_901"}]}),
                _ => json!({"list": [], "pageNo": 1, "pageSize": 20}),
            })
        })
    }
}

/// A few screens of the facade, in the shape commerce serves at `GET /v1/tools?set=facade`:
/// screen names, codes without meaning, no links, no checks.
fn facade_manifest() -> Vec<ToolSpec> {
    fn screen(tool: &str, address: &str, kind: &str, summary: &str, properties: Value) -> Value {
        json!({"tool": tool, "address": address, "kind": kind, "summary": summary,
               "consumes": [], "produces": [], "checks": [], "group": "engage", "method": "POST",
               "path": format!("/v1/tools/{tool}"), "errors": [], "audience": "operator",
               "action_type": "SearchAction", "person_scope": "",
               "input_schema": {"type": "object", "additionalProperties": false,
                                "properties": properties, "required": []}})
    }
    let text = |d: &str| json!({"type": "string", "description": d});
    let row = json!({"type": "object", "additionalProperties": false, "required": ["rowStat"],
        "properties": {
            "rowStat": {"type": "string", "enum": ["I", "U", "D"], "description": "Row status."},
            "prmtNo": text("Promotion no."), "prmtNm": text("Promotion name."),
            "prmtTpCd": {"type": "string", "enum": ["T01", "T02", "T03", "T04", "T05"], "description": "Promotion type code."},
            "tgtMbrGrdCd": text("Target member grade code."),
            "strtDtm": text("Start date-time."), "endDtm": text("End date-time."),
            "aplyTgtCd": {"type": "string", "enum": ["C", "P"], "description": "Apply target code."},
            "ctgNo": text("Category no."),
            "dcRt": {"type": "number", "description": "Discount rate."}}});
    let items = json!([
        screen(
            "getPrmtGridList",
            "promotion.prmt.getPrmtGridList",
            "read",
            "Promotion grid list.",
            json!({"prmtStatCd": {"type": "string", "enum": ["00", "10", "20", "30", "90"], "description": "Promotion status code."},
                      "pageNo": {"type": "integer", "description": "Page no."}})
        ),
        screen(
            "getPrmtGridListCnt",
            "promotion.prmt.getPrmtGridListCnt",
            "read",
            "Promotion grid list count.",
            json!({"prmtStatCd": {"type": "string", "enum": ["00", "10", "20", "30", "90"], "description": "Promotion status code."}})
        ),
        screen(
            "getMbrGrdList",
            "customer.mbr.getMbrGrdList",
            "read",
            "Member grade list.",
            json!({})
        ),
        screen(
            "getPrdGridList",
            "search.prd.getPrdGridList",
            "read",
            "Product grid list.",
            json!({"srchKwd": text("Search keyword."), "ctgNo": text("Category no.")})
        ),
        screen(
            "savePrmtList",
            "promotion.prmt.savePrmtList",
            "write",
            "Promotion list save.",
            json!({"rows": {"type": "array", "items": row, "description": "Grid rows."}})
        ),
    ]);
    serde_json::from_value(items).unwrap()
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
    let rt = Runtime::new(Arc::new(repo()), set, log, Arc::new(FixedStore(settings())));
    // The scripted models stream, as the background check would have found.
    for m in models {
        rt.assume_streaming(&m.info.alias, StreamState::Works(StreamVariant::Configured));
    }
    rt
}

#[tokio::test]
async fn the_conversation_list_is_the_callers_own_with_times_in_store_time() {
    let log = Arc::new(MemoryLog::default());
    let rt = runtime(&[], log.clone());
    let sj = principal("adm_sj", Surface::Bo);
    let started = |conversation: &str, owner: &Principal| TurnStart {
        tenant: "store_northwind".into(),
        conversation_id: conversation.into(),
        surface: "bo".into(),
        owner: Owner::of(owner),
        name: "Run a VIP weekend sale".into(),
        turn_id: format!("turn_{}", acom_core::id::new_suffix()),
        text: "Run a VIP weekend sale".into(),
        context: json!([]),
        agent: "promotion".into(),
        request_id: "req_test".into(),
        started_at: settings().now,
    };
    log.start_turn(&started("conv_sj", &sj)).await.unwrap();
    log.start_turn(&started("conv_mina", &principal("adm_mina", Surface::Bo)))
        .await
        .unwrap();

    // The test commerce gives no store settings, so the store's own fallback (New York) applies.
    let list = rt
        .conversations(&sj, ChatSurface::Bo, 20, Commerce::new().as_ref())
        .await
        .unwrap();
    assert_eq!(list["total"], 1, "{list:#}");
    let item = &list["items"][0];
    assert_eq!(item["conversation_id"], "conv_sj");
    assert_eq!(item["name"], "Run a VIP weekend sale");
    assert_eq!(item["started_at"], "2026-11-06T11:02:00-05:00");
    assert_eq!(item["last_turn_at"], "2026-11-06T11:02:00-05:00");
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

/// The blocks on screen when the turn ended: a block sent again with its id replaces itself in
/// place, and a removed one is gone (the turn sends blocks while it works).
fn blocks(events: &[Event]) -> Vec<&Block> {
    let mut shown: Vec<&Block> = Vec::new();
    for e in events {
        match e {
            Event::Block(b) => match shown.iter_mut().find(|s| s.block_id == b.block_id) {
                Some(s) => *s = b.as_ref(),
                None => shown.push(b.as_ref()),
            },
            Event::BlockRemoved { block_id } => shown.retain(|s| &s.block_id != block_id),
            _ => {}
        }
    }
    shown
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

/// A model that is down: the endpoint answers 503 to everything.
#[tokio::test]
async fn when_the_model_is_down_the_turn_says_so_plainly() {
    let log = Arc::new(MemoryLog::default());
    let down = Scripted::new("glimmer", "Muse Glimmer 30B", vec![]);
    let rt = runtime(&[down], log);
    let commerce = Commerce::new();
    let events = turn(
        &rt,
        principal("adm_sj", Surface::Bo),
        ChatSurface::Bo,
        say("What needs me today?"),
        commerce.clone(),
    )
    .await;
    let said: Vec<String> = events
        .iter()
        .filter_map(|e| match e {
            Event::MessageDone { text, .. } => Some(text.clone()),
            Event::Error { message, .. } => Some(message.clone()),
            _ => None,
        })
        .collect();
    assert!(
        said.iter().any(|t| t == MODEL_DOWN),
        "{said:?} in {:?}",
        names(&events)
    );
    assert!(
        !said.iter().any(|t| t.contains("503")),
        "no endpoint detail for the person: {said:?}"
    );
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

    // The change set is on screen while it is drafted: each write sends it again, with the same
    // id, and the first one comes before the turn checks the finished set and replies.
    let sent: Vec<usize> = events
        .iter()
        .enumerate()
        .filter(|(_, e)| matches!(e, Event::Block(x) if x.ui == "change_set"))
        .map(|(i, _)| i)
        .collect();
    assert!(sent.len() >= 2, "sent {} times", sent.len());
    assert!(events.iter().all(|e| match e {
        Event::Block(x) if x.ui == "change_set" => x.block_id == b[0].block_id,
        _ => true,
    }));
    let checking = events
        .iter()
        .position(|e| {
            e.data()["summary"]
                .as_str()
                .is_some_and(|s| s.starts_with("Checking the change set"))
        })
        .expect("the finished set is checked");
    assert!(
        sent[0] < checking,
        "the first change set block came at {} after the check at {checking}",
        sent[0]
    );

    // The reply streams in pieces as it is written, and `message.done` carries the same text.
    let deltas: Vec<String> = events
        .iter()
        .filter_map(|e| match e {
            Event::MessageDelta { text } => Some(text.clone()),
            _ => None,
        })
        .collect();
    assert!(deltas.len() > 5, "{} pieces", deltas.len());
    let final_text = events
        .iter()
        .find_map(|e| match e {
            Event::MessageDone { text, .. } => Some(text.clone()),
            _ => None,
        })
        .unwrap();
    assert_eq!(deltas.concat(), final_text);
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
    // The request names the promotion agent's work ("weekend sale"), so no router call.
    assert_eq!(usage.model_calls, 2);
    assert_eq!(usage.input_tokens, 200);
    assert_eq!(models, ["Muse Glimmer 30B"]);
    assert_eq!(glimmer.asked(), ["plan", "reply"]);

    let calls = log.model_calls();
    assert_eq!(
        calls.iter().map(|c| c.purpose.as_str()).collect::<Vec<_>>(),
        ["plan", "reply"]
    );
    let steps = log.steps();
    assert!(steps.iter().any(|s| s.kind == "call_tool"
        && s.preview_id.is_some()
        && s.change_set_id.as_deref() == Some("cs_1042")));
    assert!(
        steps.iter().all(|s| s.status != "active"),
        "every step ended"
    );
    assert_eq!(
        log.ended()[0].usage["tool_set"],
        "commerce",
        "the turn log says which tools the turn used"
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
    // The reply is written knowing the cart line was added; the order is only prepared.
    let facts = glimmer.reply_inputs();
    let saved = facts
        .iter()
        .find_map(|f| f.find("\"saved\"").map(|i| f[i..].to_owned()))
        .expect("the reply's facts list what ran");
    assert!(saved.contains("add_to_cart"), "{saved}");
    assert!(!saved.contains("prepare_order"), "{saved}");
}

#[tokio::test]
async fn a_cart_answer_after_adding_to_it_shows_the_cart_read_again() {
    let find =
        json!({"id": "f", "tool": "search_products", "args": {"query": "windbreaker"}, "uses": []});
    let add = json!({"id": "a", "tool": "add_to_cart", "args": {"size": "M"}, "uses": ["f"]});
    let read = |id: &str| json!({"id": id, "tool": "get_cart", "args": {}, "uses": []});
    for (steps, cart_reads) in [
        // Added, never read: the cart is read once more.
        (json!([find, add]), 1),
        // Read before the change: stale, read again.
        (json!([read("r"), find, add]), 2),
        // Read after the change: shown as read.
        (json!([find, add, read("r")]), 1),
    ] {
        let glimmer = Scripted::new(
            "glimmer",
            "Muse Glimmer 30B",
            vec![(
                "plan",
                json!({"interpretation": [], "about": "self", "reply_plan": "cart", "steps": steps})
                    .to_string(),
            )],
        );
        let rt = runtime(
            std::slice::from_ref(&glimmer),
            Arc::new(MemoryLog::default()),
        );
        let commerce = Commerce::new();
        let events = turn(
            &rt,
            principal("cus_jamie", Surface::Fo),
            ChatSurface::Fo,
            say("Add the black one in M to my cart."),
            commerce.clone(),
        )
        .await;
        let calls = commerce.calls();
        let reads = calls
            .iter()
            .filter(|c| c.as_str() == "read get_cart")
            .count();
        assert_eq!(reads, cart_reads, "{steps}: {calls:?}");
        assert_eq!(
            calls.last().map(String::as_str),
            Some("read get_cart"),
            "{steps}: {calls:?}"
        );
        let b = blocks(&events);
        assert_eq!(b.len(), 1, "{steps}: {b:?}");
        assert_eq!(b[0].ui, "cart", "{steps}");
        assert_eq!(b[0].data["lines"][0]["name"], "Packable windbreaker");
    }
}

#[test]
fn of_two_names_the_request_says_the_longer_one_is_meant() {
    let m = fixtures::manifest();
    let tool = |name: &str| m.iter().find(|t| t.tool == name).unwrap();
    let out = json!({"items": [
        {"category_id": "cat_outerwear", "name": "Outerwear"},
        {"category_id": "cat_summer_outerwear", "name": "Summer outerwear"},
        {"category_id": "cat_knitwear", "name": "Knitwear"}]});
    let read = StepRecord {
        step: "c".into(),
        tool: "search_products".into(),
        kind: ToolKind::Read,
        input: json!({}),
        slots: executor::slots_from_read(tool("search_products"), &out),
        output: Some(out),
        preview: None,
        item: None,
    };
    let view = BTreeMap::from([("c".to_owned(), read)]);
    let step = PlanStep {
        id: "s".into(),
        tool: "set_sale_prices".into(),
        args: json!({}),
        uses: vec!["c".into()],
        each: None,
        when: None,
    };
    let named =
        |said: &str| named_in_request("category_id", &step, tool("set_sale_prices"), &view, said);
    assert_eq!(
        named("Run a 20% weekend sale on summer outerwear for VIP members."),
        Some(json!("cat_summer_outerwear"))
    );
    assert_eq!(
        named("Mark all outerwear down 10%."),
        Some(json!("cat_outerwear"))
    );
    assert_eq!(named("Outerwear and knitwear at 10% off."), None);
    assert_eq!(named("A sale on shoes."), None);
}

#[tokio::test]
async fn an_earlier_block_written_where_an_id_goes_is_used_for_the_row_the_plan_names() {
    let glimmer = Scripted::new(
        "glimmer",
        "Muse Glimmer 30B",
        vec![(
            "plan",
            json!({"interpretation": [], "about": "self", "reply_plan": "product_list", "steps": [
                {"id": "f", "tool": "search_products", "args": {"query": "light jacket"}, "uses": []}]})
            .to_string(),
        )],
    );
    let rt = runtime(
        std::slice::from_ref(&glimmer),
        Arc::new(MemoryLog::default()),
    );
    let commerce = Commerce::new();
    let first = turn(
        &rt,
        principal("cus_jamie", Surface::Fo),
        ChatSurface::Fo,
        say("I need a light jacket for spring trips."),
        commerce.clone(),
    )
    .await;
    let shown = blocks(&first);
    let block_id = shown[0].block_id.clone();
    let conversation = first[0].data()["conversation_id"]
        .as_str()
        .unwrap()
        .to_owned();
    // The model writes the block's id under the product prefix ("prd_01…" for "blk_01…"), and
    // names the product only in what it understood.
    let (_, body) = block_id.split_once('_').unwrap();
    glimmer.will_answer(
        "plan",
        json!({"about": "self", "reply_plan": "cart",
            "interpretation": [{"aspect": "products", "text": "The windbreaker is the Packable windbreaker."}],
            "steps": [{"id": "a", "tool": "add_to_cart", "uses": [],
                "args": {"mode": "product_options", "product_id": format!("prd_{body}"), "size": "M", "color": "Black"}}]})
        .to_string(),
    );
    let mut req = say("Add the black one in M to my cart.");
    req.conversation_id = Some(conversation);
    turn(
        &rt,
        principal("cus_jamie", Surface::Fo),
        ChatSurface::Fo,
        req,
        commerce.clone(),
    )
    .await;
    let added: Vec<Value> = commerce
        .previews()
        .into_iter()
        .filter(|(tool, _, _)| tool == "add_to_cart")
        .map(|(_, input, _)| input)
        .collect();
    assert_eq!(added.len(), 1, "{:?}", commerce.calls());
    assert_eq!(added[0]["product_id"], "prd_windbreaker", "{added:?}");
    assert_eq!(added[0]["size"], "M");
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
        b[0].ui, "product_list",
        "a signed-in shopper is not on a first visit (shopper.yml)"
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
    // Replies fail as a server error, so each model is asked once more without the schema.
    assert_eq!(spark.asked(), ["plan", "reply", "reply"]);
    assert_eq!(glimmer.asked(), ["plan", "plan", "reply", "reply"]);
    assert!(events.iter().any(|e| matches!(e, Event::Step { kind: StepKind::Plan, summary, .. } if summary.starts_with("Changing the plan"))));
    // Jamie is signed in, so not on a first visit: the plain list.
    assert_eq!(blocks(&events)[0].ui, "product_list");
}

/// A plan that fails on its last write comes back repeating the writes before it: those items
/// change in place, and the change set does not end up with the sale twice.
#[tokio::test]
async fn a_revised_plan_changes_the_items_it_repeats() {
    #[derive(Debug)]
    struct BannerFailsOnce(Arc<Commerce>, Mutex<bool>);
    impl ToolClient for BannerFailsOnce {
        fn manifest(&self) -> BoxFuture<'_, Result<Vec<ToolSpec>, ToolError>> {
            self.0.manifest()
        }
        fn read<'a>(
            &'a self,
            tool: &'a str,
            input: &'a Value,
        ) -> BoxFuture<'a, Result<Value, ToolError>> {
            self.0.read(tool, input)
        }
        fn preview<'a>(
            &'a self,
            t: &'a str,
            i: &'a Value,
            c: Option<&'a str>,
        ) -> BoxFuture<'a, Result<Preview, ToolError>> {
            if t == "schedule_banner" && !std::mem::replace(&mut *self.1.lock().unwrap(), true) {
                self.0.log("preview schedule_banner failed".into());
                return Box::pin(async {
                    Err(ToolError::invalid_input(
                        "`image_source: product` needs a product.",
                    ))
                });
            }
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
            (
                "route",
                json!({"agent": "promotion", "confidence": 0.9}).to_string(),
            ),
            ("plan", b1_plan()),
            ("plan", b1_plan()),
            (
                "reply",
                "I drafted change set #1042 for the 20% sale.".into(),
            ),
        ],
    );
    let rt = runtime(&[glimmer], Arc::new(MemoryLog::default()));
    let commerce = Commerce::new();
    let failing = Arc::new(BannerFailsOnce(commerce.clone(), Mutex::new(false)));
    let p = rt
        .prepare(
            principal("adm_sj", Surface::Bo),
            "req_9".into(),
            ChatSurface::Bo,
            say(B1),
        )
        .await
        .unwrap();
    let (emit, mut rx) = Emitter::channel();
    rt.run(p, failing, emit).await;
    while rx.try_recv().is_ok() {}

    let calls = commerce.calls();
    let revised: Vec<&str> = calls[8..].iter().map(String::as_str).collect();
    assert_eq!(
        revised,
        [
            "preview set_sale_prices",
            "replace_items cs_1042 pv_3,pv_2",
            "preview create_coupon",
            "replace_items cs_1042 pv_3,pv_4",
            "preview schedule_banner into cs_1042",
        ],
        "sale and coupon replaced in place, the banner added once: {calls:#?}"
    );
}

// ---- B3: a sales question, read only ------------------------------------------------------

const B3: &str = "Why did sales drop yesterday?";

/// The fixture tools plus the two analytics reads, as commerce lists them.
fn manifest_with_analytics() -> Vec<ToolSpec> {
    let date = |what: &str| json!({"type": "string", "format": "date", "description": what});
    let analytics: crate::tools::Manifest = serde_json::from_value(json!({"items": [
        {"tool": "explain_sales_change", "address": "analytics.sales.explain_sales_change", "kind": "read",
         "summary": "Explain why sales went up or down against a basis, split into causes the data shows.",
         "consumes": [], "produces": [], "checks": [],
         "input_schema": {"type": "object", "additionalProperties": false,
            "required": ["valid_from", "valid_through", "compare_mode"],
            "properties": {
                "valid_from": date("First day."), "valid_through": date("Last day."),
                "compare_mode": {"type": "string", "description": "Basis.",
                                 "enum": ["same_weekday_last_4_weeks", "previous_period", "custom"]}}}},
        {"tool": "get_sales_trend", "address": "analytics.sales.get_sales_trend", "kind": "read",
         "summary": "Sales over time per day, week or month.",
         "consumes": [], "produces": [], "checks": [],
         "input_schema": {"type": "object", "additionalProperties": false,
            "required": ["valid_from", "valid_through", "acom:granularity"],
            "properties": {
                "valid_from": date("First day."), "valid_through": date("Last day."),
                "acom:granularity": {"type": "string", "enum": ["day", "week", "month"], "description": "Point length."}}}}
    ]}))
    .unwrap();
    let mut m = fixtures::manifest();
    m.extend(analytics.items);
    m
}

/// The shopper's records: orders, a payment, reviews and questions.
fn manifest_with_records() -> Vec<ToolSpec> {
    let read = |tool: &str, address: &str, summary: &str| {
        json!({"tool": tool, "address": address, "kind": "read", "summary": summary,
               "consumes": [], "produces": [], "checks": [],
               "input_schema": {"type": "object", "additionalProperties": false, "properties": {
                   "order_id": {"type": "string", "description": "Order."},
                   "product_id": {"type": "string", "description": "Product."},
                   "answered": {"type": "boolean", "description": "Only answered (true) or waiting (false)."},
                   "limit": {"type": "integer", "description": "Page size."}}, "required": []}})
    };
    let records: crate::tools::Manifest = serde_json::from_value(json!({"items": [
        read("get_my_orders", "sales.order.get_my_orders", "The shopper's own orders, newest first."),
        read("get_order", "sales.order.get_order", "One order with its lines, delivery and payment."),
        read("get_payment", "sales.payment.get_payment", "How an order was paid."),
        read("track_shipment", "fulfillment.parcel_delivery.track_shipment", "Where an order's parcel is."),
        read("list_reviews", "catalog.review.list_reviews", "Reviews of a product."),
        read("list_faqs", "customer.faq_notice.list_faqs", "Questions shoppers often ask, with the store's answers."),
        read("list_questions", "catalog.question.list_questions", "Questions about a product and their answers."),
    ]}))
    .unwrap();
    let mut m = fixtures::manifest();
    m.extend(records.items);
    m
}

async fn shopper_turn(plan: Value, said: &str) -> Vec<Event> {
    let glimmer = Scripted::new(
        "glimmer",
        "Muse Glimmer 30B",
        vec![("plan", plan.to_string())],
    );
    let rt = runtime(
        std::slice::from_ref(&glimmer),
        Arc::new(MemoryLog::default()),
    );
    turn(
        &rt,
        principal("cus_jamie", Surface::Fo),
        ChatSurface::Fo,
        say(said),
        Commerce::with_manifest(manifest_with_records()),
    )
    .await
}

#[tokio::test]
async fn a_shoppers_orders_are_shown_as_records_whatever_block_the_plan_named() {
    for reply_plan in ["text", "product_list", "record_list"] {
        let events = shopper_turn(
            json!({"interpretation": [], "about": "self", "reply_plan": reply_plan, "steps": [
                {"id": "s1", "tool": "get_my_orders", "args": {}, "uses": []}]}),
            "Show me my orders",
        )
        .await;
        let b = blocks(&events);
        assert_eq!(b.len(), 1, "{reply_plan}: {b:?}");
        assert_eq!(b[0].ui, "record_list", "{reply_plan}");
        assert_eq!(b[0].data["kind"], "orders");
        assert_eq!(b[0].data["rows"][0]["title"], "Order #20268");
    }
}

#[tokio::test]
async fn an_order_is_shown_from_the_order_read_not_its_payment() {
    for reply_plan in ["order_detail", "text"] {
        let events = shopper_turn(
            json!({"interpretation": [], "about": "self", "reply_plan": reply_plan, "steps": [
                {"id": "s1", "tool": "get_order", "args": {"order_id": "ord_20268"}, "uses": []},
                {"id": "s2", "tool": "get_payment", "args": {"order_id": "ord_20268"}, "uses": []}]}),
            "Show me order 20268",
        )
        .await;
        let b = blocks(&events);
        assert_eq!(b.len(), 1, "{reply_plan}: {b:?}");
        assert_eq!(b[0].ui, "order_detail", "{reply_plan}");
        assert_eq!(b[0].data["order_number"], "20268");
        assert_eq!(b[0].data["lines"][0]["name"], "Packable windbreaker");
    }
}

#[tokio::test]
async fn of_several_orders_read_the_one_whose_parcel_was_tracked_is_shown() {
    let events = shopper_turn(
        json!({"interpretation": [], "about": "self", "reply_plan": "order_detail", "steps": [
            {"id": "s1", "tool": "get_order", "args": {"order_id": "ord_20268"}, "uses": []},
            {"id": "s2", "tool": "track_shipment", "args": {"order_id": "ord_20268"}, "uses": ["s1"]},
            {"id": "s3", "tool": "get_order", "args": {"order_id": "ord_20105"}, "uses": []}]}),
        "Track my windbreaker order",
    )
    .await;
    let b = blocks(&events);
    assert_eq!(b.len(), 1, "{b:?}");
    assert_eq!(b[0].data["order_number"], "20268");
}

#[test]
fn a_list_read_one_row_at_a_time_counts_from_its_first_row() {
    let rec = |step: &str, order: &str| StepRecord {
        step: step.into(),
        tool: "get_order".into(),
        kind: ToolKind::Read,
        input: json!({}),
        output: Some(json!({"order_number": order})),
        preview: None,
        item: None,
        slots: BTreeMap::new(),
    };
    let numbers = |h: &[StepRecord]| -> Vec<String> {
        newest_first(h, |_| true)
            .iter()
            .map(|r| {
                r.output.as_ref().unwrap()["order_number"]
                    .as_str()
                    .unwrap()
                    .to_owned()
            })
            .collect()
    };
    // `s2` ran once per row of a list sorted newest first: its first row is the newest order.
    let each = [
        rec("s1", "1"),
        rec("s2", "29202"),
        rec("s2", "20381"),
        rec("s2", "20105"),
    ];
    assert_eq!(numbers(&each), ["29202", "20381", "20105", "1"]);
    // Separate steps: the last read comes first, as before.
    let steps = [rec("s1", "20268"), rec("s2", "20105")];
    assert_eq!(numbers(&steps), ["20105", "20268"]);
    assert!(numbers(&[]).is_empty());
}

#[tokio::test]
async fn what_the_agent_always_has_at_hand_does_not_crowd_out_the_answer() {
    let events = shopper_turn(
        json!({"interpretation": [], "about": "self", "reply_plan": "text", "steps": [
            {"id": "s1", "tool": "list_faqs", "args": {}, "uses": []},
            {"id": "s2", "tool": "get_my_orders", "args": {}, "uses": []}]}),
        "How long do I have to return something?",
    )
    .await;
    let b = blocks(&events);
    assert_eq!(b.len(), 1, "{b:?}");
    assert_eq!(b[0].data["kind"], "faqs");
}

#[tokio::test]
async fn of_several_orders_read_the_one_with_what_was_named_is_shown() {
    let events = shopper_turn(
        json!({"interpretation": [], "about": "self", "reply_plan": "order_detail", "steps": [
            {"id": "s1", "tool": "get_order", "args": {"order_id": "ord_20268"}, "uses": []},
            {"id": "s2", "tool": "get_order", "args": {"order_id": "ord_20105"}, "uses": []}]}),
        "Track my windbreaker order",
    )
    .await;
    let b = blocks(&events);
    assert_eq!(b[0].data["order_number"], "20268", "{b:?}");
}

#[tokio::test]
async fn products_come_with_the_reviews_and_questions_read_about_them() {
    let events = shopper_turn(
        json!({"interpretation": [], "about": "self", "reply_plan": "product_list", "steps": [
            {"id": "s1", "tool": "search_products", "args": {"query": "windbreaker"}, "uses": []},
            {"id": "s2", "tool": "list_reviews", "args": {"product_id": "prd_windbreaker"}, "uses": ["s1"]},
            {"id": "s3", "tool": "list_questions", "args": {"product_id": "prd_windbreaker"}, "uses": ["s1"]}]}),
        "What do people say about the windbreaker?",
    )
    .await;
    let b = blocks(&events);
    let uis: Vec<&str> = b.iter().map(|b| b.ui.as_str()).collect();
    assert_eq!(uis, ["product_list", "record_list", "record_list"]);
    assert_eq!(b[1].data["kind"], "reviews");
    assert_eq!(b[2].data["kind"], "questions");
    assert_eq!(
        b[2].data["rows"][0]["body"],
        "The store: Yes, into the collar."
    );
}

/// What Muse Glimmer answered for scene B3 (recorded): two reads and the explainer block.
fn b3_plan() -> String {
    json!({
        "title": "Sales on Thursday",
        "interpretation": [{"aspect": "when", "text": "Yesterday, Thu Nov 5"},
                           {"aspect": "products", "text": "All sales, against the last 4 Thursdays"}],
        "about": "self",
        "steps": [
            {"id": "s1", "tool": "explain_sales_change",
             "args": {"valid_from": "yesterday", "valid_through": "yesterday",
                      "compare_mode": "same_weekday_last_4_weeks"}, "uses": []},
            {"id": "s2", "tool": "get_sales_trend",
             "args": {"valid_from": "2026-10-23", "valid_through": "yesterday",
                      "acom:granularity": "day"}, "uses": []}
        ],
        "reply_plan": "sales_explainer"
    })
    .to_string()
}

#[tokio::test]
async fn b3_routes_to_insights_reads_twice_and_answers_with_the_explainer() {
    let glimmer = Scripted::new(
        "glimmer",
        "Muse Glimmer 30B",
        vec![
            ("plan", b3_plan()),
            ("reply", "Revenue was $18,240 yesterday, $4,660 (20%) under a normal Thursday. \
                       Sold-out M and L sizes of three bestsellers cost about $2,700 and the unsent \
                       weekly email about $1,300; $660 stays unexplained."
                .into()),
        ],
    );
    let log = Arc::new(MemoryLog::default());
    let rt = runtime(std::slice::from_ref(&glimmer), log.clone());
    let commerce = Commerce::with_manifest(manifest_with_analytics());
    let events = turn(
        &rt,
        principal("adm_sj", Surface::Bo),
        ChatSurface::Bo,
        say(B3),
        commerce.clone(),
    )
    .await;

    assert_eq!(events[0].data()["agent"], "insights");
    assert_eq!(
        glimmer.asked(),
        ["plan", "reply"],
        "the sales question is routed by its words, without a model call"
    );
    assert_eq!(
        commerce.calls(),
        [
            "manifest",
            "read explain_sales_change",
            "read get_sales_trend"
        ],
        "reads only: no preview, no change set"
    );
    let reads = commerce.reads();
    assert_eq!(
        reads[0].1,
        json!({"valid_from": "2026-11-05", "valid_through": "2026-11-05",
               "compare_mode": "same_weekday_last_4_weeks"}),
        "yesterday is Thu Nov 5 on the store clock"
    );
    assert_eq!(reads[1].1["valid_through"], "2026-11-05");

    let b = blocks(&events);
    assert_eq!(b.len(), 1);
    assert_eq!(b[0].ui, "sales_explainer");
    assert_eq!(
        b[0].data["summary"]["difference"]["value"].to_string(),
        "-4660.00"
    );
    assert_eq!(b[0].data["causes"].as_array().unwrap().len(), 3);
    assert_eq!(b[0].person_actions().count(), 0);
    assert_eq!(b[0].actions.len(), 3, "three next steps, each a say button");
    assert!(message(&events).starts_with("Revenue was $18,240 yesterday"));
    let (usage, _) = done(&events);
    assert_eq!((usage.tool_calls, usage.model_calls), (2, 2));
}

#[tokio::test]
async fn a_next_step_button_speaks_its_request() {
    let req = TurnRequest {
        ui_event: Some(UiEvent {
            block_id: "blk_1".into(),
            action_id: "next_3".into(),
            args: json!({"request": "Send the weekly email today, to the same list as last time."}),
        }),
        ..TurnRequest::default()
    };
    assert_eq!(
        req.text(),
        "Send the weekly email today, to the same list as last time."
    );
    let plain = TurnRequest {
        ui_event: Some(UiEvent {
            block_id: "blk_1".into(),
            action_id: "more".into(),
            args: json!({}),
        }),
        ..TurnRequest::default()
    };
    assert!(plain.button_request().is_none());
    assert!(
        plain
            .text()
            .starts_with("(The person pressed the button `more`")
    );
}

const B4: &str = "List the 30 products in this supplier sheet. Use our usual pricing and write the descriptions in our tone.";

/// What the worker model answered for scene B4 (recorded): one import, the drafts block.
fn b4_plan() -> String {
    json!({
        "title": "FW26 supplier sheet",
        "interpretation": [
            {"aspect": "products", "text": "The 30 rows of the attached supplier sheet"},
            {"aspect": "benefit", "text": "Prices from the store's usual pricing rule"},
            {"aspect": "other", "text": "Descriptions in the store's tone, from the supplier's text"}],
        "about": "self",
        "steps": [{"id": "s1", "tool": "batch_import", "args": {"acom:mode": "create"}, "uses": []}],
        "reply_plan": "product_drafts"
    })
    .to_string()
}

#[tokio::test]
async fn b4_turns_the_supplier_sheet_into_drafts_and_leaves_saving_to_a_person() {
    let glimmer = Scripted::new(
        "glimmer",
        "Muse Glimmer 30B",
        vec![
            ("plan", b4_plan()),
            ("reply", "I turned the 30 rows into drafts: 26 are ready and 4 need you. Shoppers do not see drafts; saving them waits for you.".into()),
        ],
    );
    let log = Arc::new(MemoryLog::default());
    let rt = runtime(std::slice::from_ref(&glimmer), log.clone());
    let commerce = Commerce::new();
    let mut request = say(B4);
    request.attachments = vec![json!({"kind": "sheet", "upload_id": "upl_fw26_supplier"})];
    let events = turn(
        &rt,
        principal("adm_sj", Surface::Bo),
        ChatSurface::Bo,
        request,
        commerce.clone(),
    )
    .await;

    // The request names a supplier sheet: the catalog agent takes it without a routing call.
    assert_eq!(glimmer.asked(), ["plan", "reply"]);
    assert_eq!(events[0].data()["agent"], "catalog");
    assert!(events.iter().any(
        |e| matches!(e, Event::Step { kind: StepKind::Route, summary, .. }
        if summary.contains("matched a routing hint"))
    ));

    // One import previewed into a new change set, handed to people, never approved.
    assert_eq!(
        commerce.calls(),
        [
            "manifest",
            "create_change_set FW26 supplier sheet",
            "preview batch_import into cs_1042",
            "submit cs_1042 v1"
        ]
    );
    let (tool, input, cs) = &commerce.previews()[0];
    assert_eq!(tool, "batch_import");
    assert_eq!(cs.as_deref(), Some("cs_1042"));
    assert_eq!(
        input["acom:file"], "upl_fw26_supplier",
        "the attached sheet fills the upload slot"
    );
    assert_eq!(input["acom:mode"], "create");

    // The answer is the drafts block, which carries the change set; no second change_set block.
    let b = blocks(&events);
    assert_eq!(b.len(), 1);
    let drafts = b[0];
    assert_eq!(drafts.ui, "product_drafts");
    assert_eq!(
        drafts.data["progress"],
        json!({"total": 30, "ready": 26, "needs_you": 4, "working": 0})
    );
    assert_eq!(drafts.data["change_set_id"], "cs_1042");
    assert_eq!(drafts.data["state"], "waiting_for_approval");
    let checks: Vec<(&str, &str)> = drafts.data["checks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| (c["rule"].as_str().unwrap(), c["status"].as_str().unwrap()))
        .collect();
    assert_eq!(
        checks,
        [
            ("product.required_on_create", "passed"),
            ("product.sku_unique", "passed"),
            ("price_floor", "passed"),
            ("import.required_fields", "open")
        ]
    );
    let save = drafts
        .actions
        .iter()
        .find_map(|a| match a {
            Action::Person { path, body, .. } => Some((path.clone(), body.clone())),
            _ => None,
        })
        .expect("saving the drafts is the person's button");
    assert_eq!(
        save,
        (
            "/v1/platform/change-sets/cs_1042/approve".to_owned(),
            json!({"version": 1})
        )
    );
    // The open check is pointed at, never the person's button.
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::UiCommand { target, .. }
        if target.ends_with("#check_row:import.required_fields")))
    );
    assert!(message(&events).contains("26 are ready"));
    let steps = log.steps();
    assert!(
        steps
            .iter()
            .any(|s| s.kind == "call_tool" && s.change_set_id.as_deref() == Some("cs_1042"))
    );
}

#[tokio::test]
async fn an_empty_plan_is_planned_again_and_the_answer_comes_from_the_store() {
    let glimmer = Scripted::new(
        "glimmer",
        "Muse Glimmer 30B",
        vec![
            ("plan", json!({"interpretation": [], "about": "self", "reply_plan": "text", "steps": []}).to_string()),
            ("plan", json!({"interpretation": [{"aspect": "products", "text": "light jackets"}], "about": "self",
                "reply_plan": "product_list", "steps": [
                {"id": "s1", "tool": "search_products", "args": {"query": "light jacket"}, "uses": []}]})
            .to_string()),
            ("reply", "Here are three light jackets.".into()),
        ],
    );
    let rt = runtime(
        std::slice::from_ref(&glimmer),
        Arc::new(MemoryLog::default()),
    );
    let commerce = Commerce::new();
    let events = turn(
        &rt,
        principal("cus_jamie", Surface::Fo),
        ChatSurface::Fo,
        say("I need a light jacket for spring trips"),
        commerce.clone(),
    )
    .await;
    assert_eq!(
        glimmer.asked().iter().filter(|p| *p == "plan").count(),
        2,
        "{:?}",
        glimmer.asked()
    );
    assert!(
        commerce
            .calls()
            .iter()
            .any(|c| c.contains("search_products"))
    );
    assert_eq!(blocks(&events)[0].ui, "product_list");
    assert_eq!(message(&events), "Here are three light jackets.");
}

#[tokio::test]
async fn a_reply_with_made_up_numbers_is_written_again_and_what_was_read_is_shown() {
    let glimmer = Scripted::new(
        "glimmer",
        "Muse Glimmer 30B",
        vec![
            ("plan", json!({"interpretation": [{"aspect": "who", "text": "VIP members"}], "about": "self",
                "reply_plan": "text", "steps": [
                {"id": "s1", "tool": "list_tiers", "args": {}, "uses": []}]})
            .to_string()),
            ("reply", "You have 9,999 VIP members.".into()),
            ("reply", "You have 1,284 VIP members.".into()),
        ],
    );
    let rt = runtime(
        std::slice::from_ref(&glimmer),
        Arc::new(MemoryLog::default()),
    );
    let commerce = Commerce::new();
    let events = turn(
        &rt,
        principal("adm_sj", Surface::Bo),
        ChatSurface::Bo,
        say("Show me our member tiers."),
        commerce,
    )
    .await;
    assert_eq!(message(&events), "You have 1,284 VIP members.");
    // The person saw the first reply being written; `message.reset` took it off the screen
    // before the second one streamed.
    let mut screen = String::new();
    let mut resets = 0;
    for e in &events {
        match e {
            Event::MessageDelta { text } => screen.push_str(text),
            Event::MessageReset { .. } => {
                assert_eq!(screen, "You have 9,999 VIP members.");
                screen.clear();
                resets += 1;
            }
            _ => {}
        }
    }
    assert_eq!(resets, 1);
    assert_eq!(screen, "You have 1,284 VIP members.");
    let b = blocks(&events);
    assert_eq!(b.len(), 1, "the tiers read, shown as a table");
    assert_eq!(b[0].ui, "product_table");
    assert_eq!(glimmer.asked().iter().filter(|p| *p == "reply").count(), 2);
}

#[tokio::test]
async fn a_planned_yes_no_the_shopper_never_said_is_not_sent() {
    // The plan does not say which product; the arguments model is asked and answers with every
    // field, the yes/no included.
    let glimmer = Scripted::new(
        "glimmer",
        "Muse Glimmer 30B",
        vec![
            (
                "plan",
                json!({"interpretation": [], "about": "self", "reply_plan": "record_list", "steps": [
                    {"id": "q", "tool": "list_questions", "uses": [], "args": {"answered": true, "limit": 20}}]})
                .to_string(),
            ),
            (
                "arguments",
                json!({"product_id": "prd_windbreaker", "answered": true, "limit": 20}).to_string(),
            ),
        ],
    );
    let mut manifest = manifest_with_records();
    for t in &mut manifest {
        if t.tool == "list_questions" {
            t.input_schema["required"] = json!(["product_id"]);
        }
    }
    let rt = runtime(
        std::slice::from_ref(&glimmer),
        Arc::new(MemoryLog::default()),
    );
    let commerce = Commerce::with_manifest(manifest);
    turn(
        &rt,
        principal("cus_jamie", Surface::Fo),
        ChatSurface::Fo,
        say("What questions have people asked about the windbreaker?"),
        commerce.clone(),
    )
    .await;
    let asked: Vec<Value> = commerce
        .reads()
        .into_iter()
        .filter(|(t, _)| t == "list_questions")
        .map(|(_, input)| input)
        .collect();
    assert!(!asked.is_empty());
    for input in &asked {
        assert!(input.get("answered").is_none(), "{input}");
    }
}

#[test]
fn a_shoppers_read_drops_a_yes_no_they_never_mentioned() {
    let tool: ToolSpec = serde_json::from_value(fixtures::tool(
        "list_questions",
        "catalog.question.list_questions",
        "read",
        "Questions about a product.",
        json!([]),
        json!([]),
        json!({"product_id": {"type": "string", "description": "Product."},
               "answered": {"type": "boolean", "description": "Answered or not."}}),
        json!([]),
    ))
    .unwrap();
    let mut input = json!({"product_id": "prd_windbreaker", "answered": true, "limit": 20});
    let notes = unasked_yes_no(
        &mut input,
        &tool,
        "What questions have people asked about the windbreaker?",
    );
    assert_eq!(input, json!({"product_id": "prd_windbreaker", "limit": 20}));
    assert_eq!(notes, ["answered: true was not asked for, left out"]);
    for said in [
        "Show me the answered questions",
        "Which ones are still unanswered?",
    ] {
        let mut input = json!({"answered": false});
        assert!(unasked_yes_no(&mut input, &tool, said).is_empty(), "{said}");
        assert_eq!(input["answered"], false);
    }
}

#[tokio::test]
async fn the_one_the_request_names_is_taken_from_a_list_read_before() {
    let glimmer = Scripted::new(
        "glimmer",
        "Muse Glimmer 30B",
        vec![(
            "plan",
            json!({"interpretation": [], "about": "self", "reply_plan": "product_table", "steps": [
                {"id": "tiers", "tool": "list_tiers", "args": {}, "uses": []},
                {"id": "who", "tool": "search_members", "args": {"tier_id": "", "limit": 100}, "uses": ["tiers"]}]})
            .to_string(),
        )],
    );
    let rt = runtime(
        std::slice::from_ref(&glimmer),
        Arc::new(MemoryLog::default()),
    );
    let mut manifest = fixtures::manifest();
    manifest.push(
        serde_json::from_value(fixtures::tool(
            "search_members",
            "customer.person.search_members",
            "read",
            "Find members by words, tier or state.",
            json!([]),
            json!([{"name": "customer_id", "many": true}]),
            json!({"tier_id": {"type": "string", "description": "Tier."},
                   "limit": {"type": "integer", "description": "Page size."}}),
            json!([]),
        ))
        .unwrap(),
    );
    let commerce = Commerce::with_manifest(manifest);
    turn(
        &rt,
        principal("adm_sj", Surface::Bo),
        ChatSurface::Bo,
        say("Show me the VIP members"),
        commerce.clone(),
    )
    .await;
    let searched: Vec<Value> = commerce
        .reads()
        .into_iter()
        .filter(|(t, _)| t == "search_members")
        .map(|(_, input)| input)
        .collect();
    assert_eq!(searched, [json!({"tier_id": "vip", "limit": 100})]);
}

#[tokio::test]
async fn the_bot_reads_the_areas_on_finds_signs_by_rule_and_keeps_only_figures_it_read() {
    let report = json!({"summary": "12 return requests wait.", "findings": [],
        "proposals": [{"area": "returns", "title": "Review the 12 return requests", "request": "Review the return requests over $200.",
                       "reason": "12 wait.", "evidence": ["12 requests wait", "99 customers are angry"],
                       "expected_effect": {"metric": "waiting_returns", "direction": "down", "horizon_days": 2},
                       "risk": "low", "about": []}]});
    let glimmer = Scripted::new(
        "glimmer",
        "Muse Glimmer 30B",
        vec![("bot_analysis", report.to_string())],
    );
    let rt = runtime(
        std::slice::from_ref(&glimmer),
        Arc::new(MemoryLog::default()),
    );
    let commerce = Commerce::new();
    let out = crate::bot::analyze(&rt, commerce.as_ref()).await;
    assert_eq!(out["state"], "done", "{out:#}");
    assert_eq!(out["snapshot"]["areas"], json!(["pricing", "returns"]));
    let read: Vec<String> = commerce.reads().into_iter().map(|(t, _)| t).collect();
    assert!(
        read.contains(&"search_return_requests".to_owned()),
        "{read:?}"
    );
    assert!(
        !read.iter().any(|t| t == "list_boards"),
        "community is off: {read:?}"
    );
    assert!(
        commerce
            .calls()
            .iter()
            .all(|c| c.starts_with("read ") || c == "manifest"),
        "the bot only reads: {:?}",
        commerce.calls()
    );
    assert_eq!(out["anomalies"][0]["code"], "returns_waiting");
    assert_eq!(
        out["report"]["proposals"][0]["evidence"],
        json!(["12 requests wait"])
    );
    assert_eq!(out["usage"]["model_calls"], 1);
    assert_eq!(glimmer.asked(), ["bot_analysis"]);
}

#[tokio::test]
async fn a_list_that_found_nothing_is_shown_as_an_empty_table() {
    let glimmer = Scripted::new(
        "glimmer",
        "Muse Glimmer 30B",
        vec![(
            "plan",
            json!({"interpretation": [], "about": "self", "reply_plan": "text", "steps": [
                {"id": "s1", "tool": "get_policy", "args": {"key": "campaign_hours"}, "uses": []}]})
            .to_string(),
        )],
    );
    let rt = runtime(
        std::slice::from_ref(&glimmer),
        Arc::new(MemoryLog::default()),
    );
    let events = turn(
        &rt,
        principal("adm_sj", Surface::Bo),
        ChatSurface::Bo,
        say("What campaigns are scheduled?"),
        Commerce::new(),
    )
    .await;
    let b = blocks(&events);
    assert_eq!(b.len(), 1, "{b:?}");
    assert_eq!(
        (b[0].ui.as_str(), b[0].title.as_str()),
        ("product_table", "Nothing found")
    );
    assert_eq!(b[0].data["total"], 0);
}

#[tokio::test]
async fn a_reply_that_copies_what_a_check_told_it_is_not_the_answer() {
    let asked = "Show me the change sets waiting for approval";
    let glimmer = Scripted::new(
        "glimmer",
        "Muse Glimmer 30B",
        vec![
            (
                "plan",
                json!({"interpretation": [], "about": "self", "reply_plan": "text", "steps": [
                {"id": "s1", "tool": "list_tiers", "args": {}, "uses": []}]})
                .to_string(),
            ),
            ("reply", format!("{asked}. The results show the tiers.")),
            ("reply", format!("{REPEATED_REQUEST} Here it is.")),
        ],
    );
    let rt = runtime(
        std::slice::from_ref(&glimmer),
        Arc::new(MemoryLog::default()),
    );
    let events = turn(
        &rt,
        principal("adm_sj", Surface::Bo),
        ChatSurface::Bo,
        say(asked),
        Commerce::new(),
    )
    .await;
    let reply = message(&events);
    assert!(!reply.contains("repeats the request"), "{reply}");
    assert!(!reply.is_empty());
}

#[tokio::test]
async fn replies_stream_only_where_the_background_check_saw_streaming_work() {
    let plan = || {
        json!({"interpretation": [{"aspect": "who", "text": "VIP members"}], "about": "self",
            "reply_plan": "text", "steps": [
            {"id": "s1", "tool": "list_tiers", "args": {}, "uses": []}]})
        .to_string()
    };
    let script = || {
        let mut a: Vec<(&str, String)> = Vec::new();
        for _ in 0..3 {
            a.push(("plan", plan()));
            a.push(("reply", "You have 1,284 VIP members.".into()));
        }
        a
    };
    async fn ask(rt: &Runtime) -> Vec<Event> {
        turn(
            rt,
            principal("adm_sj", Surface::Bo),
            ChatSurface::Bo,
            say("Show me our member tiers."),
            Commerce::new(),
        )
        .await
    }
    let deltas = |events: &[Event]| {
        events
            .iter()
            .filter(|e| matches!(e, Event::MessageDelta { .. }))
            .count()
    };
    let replies = |g: &Scripted| g.asked().iter().filter(|p| *p == "reply").count();

    // Not checked yet: the reply is asked for once, whole, and comes in sentences.
    let glimmer = Scripted::new("glimmer", "Muse Glimmer 30B", script());
    let rt = runtime(
        std::slice::from_ref(&glimmer),
        Arc::new(MemoryLog::default()),
    );
    rt.assume_streaming("glimmer", StreamState::Unknown);
    let events = ask(&rt).await;
    assert_eq!(message(&events), "You have 1,284 VIP members.");
    assert_eq!((replies(&glimmer), deltas(&events)), (1, 1));

    // The check sees the model stream: the next reply streams in pieces.
    rt.probe_streaming().await;
    assert_eq!(
        rt.stream_state("glimmer"),
        StreamState::Works(StreamVariant::Configured)
    );
    let events = ask(&rt).await;
    assert_eq!(message(&events), "You have 1,284 VIP members.");
    assert_eq!(replies(&glimmer), 2);
    assert!(deltas(&events) > 1);

    // A model whose streams are empty is off after the check, and replies come whole.
    let dumb = Scripted::new("glimmer", "Muse Glimmer 30B", script()).streaming_nothing();
    let rt = runtime(std::slice::from_ref(&dumb), Arc::new(MemoryLog::default()));
    rt.assume_streaming("glimmer", StreamState::Unknown);
    rt.probe_streaming().await;
    assert_eq!(rt.stream_state("glimmer"), StreamState::Off);
    let events = ask(&rt).await;
    assert_eq!(message(&events), "You have 1,284 VIP members.");
    assert_eq!(replies(&dumb), 1);

    // A way that worked stops working mid-turn: the turn asks again unstreamed, so the person
    // still gets the model's reply, and streaming waits for the next check.
    rt.assume_streaming("glimmer", StreamState::Works(StreamVariant::Configured));
    let events = ask(&rt).await;
    assert_eq!(
        message(&events),
        "You have 1,284 VIP members.",
        "the model's reply, not the plain one"
    );
    assert_eq!(replies(&dumb), 3);
    assert_eq!(rt.stream_state("glimmer"), StreamState::Unknown);
}

#[tokio::test]
async fn a_reply_the_server_failed_on_is_asked_again_without_its_schema() {
    let plan = json!({"interpretation": [{"aspect": "who", "text": "VIP members"}], "about": "self",
        "reply_plan": "text", "steps": [{"id": "s1", "tool": "list_tiers", "args": {}, "uses": []}]})
    .to_string();
    let model = Scripted::new(
        "glimmer",
        "Muse Glimmer 30B",
        vec![
            ("plan", plan),
            ("reply", "You have 1,284 VIP members.".into()),
        ],
    )
    .failing_on_reply_schema();
    let rt = runtime(std::slice::from_ref(&model), Arc::new(MemoryLog::default()));
    rt.assume_streaming("glimmer", StreamState::Off);
    let events = turn(
        &rt,
        principal("adm_sj", Surface::Bo),
        ChatSurface::Bo,
        say("Show me our member tiers."),
        Commerce::new(),
    )
    .await;
    assert_eq!(
        message(&events),
        "You have 1,284 VIP members.",
        "the model's reply, not the plain one"
    );
    assert_eq!(model.asked().iter().filter(|p| *p == "reply").count(), 2);
}

#[tokio::test]
async fn a_way_that_fails_in_turns_is_not_trusted_again_by_the_check() {
    let plan = json!({"interpretation": [{"aspect": "who", "text": "VIP members"}], "about": "self",
        "reply_plan": "text", "steps": [{"id": "s1", "tool": "list_tiers", "args": {}, "uses": []}]})
    .to_string();
    let mut answers: Vec<(&str, String)> = Vec::new();
    for _ in 0..=StreamVariant::ALL.len() {
        answers.push(("plan", plan.clone()));
        answers.push(("reply", "You have 1,284 VIP members.".into()));
    }
    let model = Scripted::new("glimmer", "Muse Glimmer 30B", answers).streaming_the_check_only();
    let rt = runtime(std::slice::from_ref(&model), Arc::new(MemoryLog::default()));
    rt.assume_streaming("glimmer", StreamState::Unknown);
    // The check passes every way; each turn proves one wrong, and the check moves past it.
    for (n, way) in StreamVariant::ALL.iter().enumerate() {
        rt.probe_streaming().await;
        assert_eq!(
            rt.stream_state("glimmer"),
            StreamState::Works(*way),
            "check {n}"
        );
        let events = turn(
            &rt,
            principal("adm_sj", Surface::Bo),
            ChatSurface::Bo,
            say("Show me our member tiers."),
            Commerce::new(),
        )
        .await;
        assert_eq!(message(&events), "You have 1,284 VIP members.", "turn {n}");
        assert_eq!(rt.stream_state("glimmer"), StreamState::Unknown);
    }
    // Every way failed in a turn: the check turns streaming off, and replies come whole.
    rt.probe_streaming().await;
    assert_eq!(rt.stream_state("glimmer"), StreamState::Off);
    let before = model.asked().iter().filter(|p| *p == "reply").count();
    turn(
        &rt,
        principal("adm_sj", Surface::Bo),
        ChatSurface::Bo,
        say("Show me our member tiers."),
        Commerce::new(),
    )
    .await;
    assert_eq!(
        model.asked().iter().filter(|p| *p == "reply").count(),
        before + 1
    );
}

fn on_facade(text: &str) -> TurnRequest {
    TurnRequest {
        message: Some(text.into()),
        tool_set: ToolSet::Facade,
        ..TurnRequest::default()
    }
}

#[test]
fn a_turn_request_names_its_tool_set_only_for_the_facade() {
    let r: TurnRequest =
        serde_json::from_value(json!({"message": "hi", "tool_set": "facade"})).unwrap();
    assert_eq!(r.tool_set, ToolSet::Facade);
    let normal: TurnRequest = serde_json::from_value(json!({"message": "hi"})).unwrap();
    assert_eq!(normal.tool_set, ToolSet::Commerce);
    assert!(
        serde_json::to_value(&normal)
            .unwrap()
            .get("tool_set")
            .is_none()
    );
    assert!(
        serde_json::from_value::<TurnRequest>(json!({"message": "hi", "tool_set": "x"})).is_err()
    );
}

#[tokio::test]
async fn a_facade_turn_plans_with_facade_tools_only_and_saves_at_once() {
    let glimmer = Scripted::new(
        "glimmer",
        "Muse Glimmer 30B",
        vec![
            ("plan", json!({
                "interpretation": [{"aspect": "who", "text": "VIP members"}, {"aspect": "benefit", "text": "20% off"}],
                "about": "self", "reply_plan": "text",
                "steps": [
                    {"id": "s1", "tool": "getMbrGrdList", "args": {}, "uses": []},
                    {"id": "s2", "tool": "savePrmtList", "uses": ["s1"], "args": {"rows": [
                        {"rowStat": "I", "prmtNm": "VIP weekend", "prmtTpCd": "T03", "tgtMbrGrdCd": "G03",
                         "strtDtm": "2026-11-07T00:00", "endDtm": "2026-11-08T23:59",
                         "aplyTgtCd": "C", "ctgNo": "cat_summer_outerwear", "dcRt": 20}]}}]})
            .to_string()),
            ("reply", "The VIP weekend promotion is saved.".into()),
        ],
    );
    let log = Arc::new(MemoryLog::default());
    let rt = runtime(std::slice::from_ref(&glimmer), log.clone());
    let commerce = Commerce::new();
    let events = turn(
        &rt,
        principal("adm_sj", Surface::Bo),
        ChatSurface::Bo,
        // Tool search matches words: the screens' terse names and descriptions only surface
        // for a request in their own words ("save", "promotion").
        on_facade("Save a promotion for VIP members: 20% off summer outerwear this weekend."),
        commerce.clone(),
    )
    .await;

    assert_eq!(
        commerce.calls(),
        [
            "facade_manifest",
            "facade getMbrGrdList",
            "facade savePrmtList"
        ],
        "the facade's manifest only; the write runs at once: no preview, no change set, no submit"
    );
    let screens: Vec<String> = facade_manifest().into_iter().map(|t| t.tool).collect();
    let steps = log.steps();
    let search = steps
        .iter()
        .find(|s| s.kind == "search_tools")
        .expect("tool search step");
    let candidates: Vec<&str> = search
        .output
        .as_ref()
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|c| c["tool"].as_str())
        .collect();
    assert!(!candidates.is_empty());
    assert!(
        candidates.iter().all(|c| screens.iter().any(|s| s == c)),
        "only facade tools are planned with: {candidates:?}"
    );
    let saved = &commerce.reads()[1];
    assert_eq!(saved.0, "savePrmtList");
    assert_eq!(saved.1["rows"][0]["prmtTpCd"], "T03");
    assert!(
        steps
            .iter()
            .all(|s| s.preview_id.is_none() && s.change_set_id.is_none())
    );
    let route = steps.iter().find(|s| s.kind == "route").unwrap();
    assert_eq!(route.output.as_ref().unwrap()["tool_set"], "facade");

    assert!(blocks(&events).iter().all(|b| b.ui != "change_set"));
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, Event::ChangeSetUpdated { .. }))
    );
    assert_eq!(message(&events), "The VIP weekend promotion is saved.");
    let (usage, _) = done(&events);
    assert_eq!(usage.tool_calls, 2);
    let ended = log.ended();
    assert_eq!(ended[0].usage["tool_set"], "facade");
    assert_eq!(ended[0].usage["tool_calls"], 2);
}

#[tokio::test]
async fn a_facade_grid_is_shown_as_a_table_and_a_plan_naming_a_store_tool_is_refused() {
    let glimmer = Scripted::new(
        "glimmer",
        "Muse Glimmer 30B",
        vec![
            // The worker reaches for a tool of the other set: not among the candidates.
            ("plan", json!({"interpretation": [], "about": "self", "reply_plan": "product_table",
                "steps": [{"id": "s1", "tool": "list_active_promotions", "args": {}, "uses": []}]})
            .to_string()),
            ("plan", json!({"interpretation": [], "about": "self", "reply_plan": "product_table",
                "steps": [{"id": "s1", "tool": "getPrmtGridList", "args": {"prmtStatCd": "20"}, "uses": []}]})
            .to_string()),
            ("reply", "Here are the running promotions.".into()),
        ],
    );
    let log = Arc::new(MemoryLog::default());
    let rt = runtime(std::slice::from_ref(&glimmer), log.clone());
    let commerce = Commerce::new();
    let events = turn(
        &rt,
        principal("adm_sj", Surface::Bo),
        ChatSurface::Bo,
        on_facade("Which promotions are running?"),
        commerce.clone(),
    )
    .await;
    assert_eq!(
        commerce.calls(),
        ["facade_manifest", "facade getPrmtGridList"]
    );
    assert_eq!(
        glimmer.asked(),
        ["plan", "plan", "reply"],
        "the plan naming a tool outside the facade was refused and made again"
    );
    let b = blocks(&events);
    assert_eq!(b.len(), 1);
    assert_eq!(b[0].ui, "product_table");
    assert_eq!(b[0].data["rows"][0]["prmtNo"], "promo_autumn_knit");
    assert_eq!(b[0].data["rows"].as_array().unwrap().len(), 2);
    assert_eq!(message(&events), "Here are the running promotions.");
    assert_eq!(log.ended()[0].usage["tool_set"], "facade");
}

#[tokio::test]
async fn a_turn_for_a_task_drafts_its_change_set_for_that_task() {
    let glimmer = Scripted::new(
        "glimmer",
        "Muse Glimmer 30B",
        vec![
            (
                "route",
                json!({"agent": "promotion", "confidence": 0.93}).to_string(),
            ),
            ("plan", b1_plan()),
            ("reply", "I drafted change set #1042.".into()),
        ],
    );
    let log = Arc::new(MemoryLog::default());
    let rt = runtime(std::slice::from_ref(&glimmer), log.clone());
    let commerce = Commerce::new();
    let request = TurnRequest {
        task_id: Some("task_vip_weekend".into()),
        ..say(B1)
    };
    turn(
        &rt,
        principal("adm_sj", Surface::Bo),
        ChatSurface::Bo,
        request,
        commerce.clone(),
    )
    .await;
    assert!(
        commerce
            .calls()
            .contains(&"create_change_set VIP weekend for task_vip_weekend".to_owned()),
        "{:?}",
        commerce.calls()
    );
}

#[test]
fn a_task_id_must_look_like_one() {
    for bad in ["cs_1042", "task_", "task_A1", "task_1; drop"] {
        let req = TurnRequest {
            task_id: Some(bad.into()),
            ..say("Draft a reorder.")
        };
        let e = req.validate().unwrap_err();
        assert_eq!(e.detail.as_ref().unwrap()["field"], "task_id", "{bad}");
    }
    let ok = TurnRequest {
        task_id: Some("task_01m680dqrxx6rt0rz5tcxs1n2d".into()),
        ..say("Draft a reorder.")
    };
    assert!(ok.validate().is_ok());
}

#[test]
fn a_result_that_fits_the_budget_is_shown_whole() {
    // A rash guard's inventory: 3 colors x 5 sizes. The fixed 5-item cut showed one color only.
    let variants: Vec<Value> = ["Black", "Navy", "Coral"]
        .iter()
        .flat_map(|c| {
            ["XS", "S", "M", "L", "XL"].iter().map(move |s| {
                json!({"variant_id": format!("var_{c}_{s}"), "color": c, "size": s,
                       "inventory_level": 1, "acom:safety_stock": 2, "on_order": 0})
            })
        })
        .collect();
    let inventory = json!({"product_id": "prd_swimwear_08", "variants": variants});
    assert_eq!(compact(&inventory, 6000), inventory);
}

#[test]
fn a_larger_result_loses_its_biggest_list_first() {
    // A 20-product search where each product lists its 15 variants: products go before sizes.
    let products: Vec<Value> = (0..20)
        .map(|p| {
            let variants: Vec<Value> = (0..15)
                .map(|v| json!({"variant_id": format!("var_{p}_{v}"), "inventory_level": v}))
                .collect();
            json!({"id": format!("prd_{p}"), "name": "Fleece hoodie", "has_variant": variants})
        })
        .collect();
    let search = json!({"items": products, "total": 20});
    let out = compact(&search, 6000);
    assert!(out.to_string().chars().count() <= 6000);
    let kept = out["items"].as_array().unwrap_or_else(|| panic!("items"));
    assert_eq!(kept.last(), Some(&json!("… 15 more")));
    assert_eq!(kept.len(), 6, "5 products and the count of the rest");
    assert_eq!(kept[0]["has_variant"].as_array().map(Vec::len), Some(15));
    // Long text is halved the same way once it is the biggest part.
    let note = json!({"id": "chk_1", "detail": "x".repeat(9000)});
    let cut = compact(&note, 6000);
    let detail = cut["detail"].as_str().unwrap_or_default();
    assert!(
        detail.ends_with('…') && detail.chars().count() == 4501,
        "{}",
        detail.len()
    );
    // Nothing more to halve: what is left, still saying what it left out.
    let tight = compact(&search, 10);
    assert_eq!(tight["items"].as_array().map(Vec::len), Some(2));
    assert_eq!(tight["items"][1], json!("… 19 more"));
}
