//! Replays of scenes B2, F3, F4 and F6 (docs/scenarios.md §6) with recorded model outputs
//! (route, plan, arguments, reply) against a fake commerce that answers with the scenes' tool
//! outputs. No network, no database, no real model. The blocks must carry the tools' numbers
//! unchanged, people's buttons must be the only way to approve or order, and nothing may be
//! remembered about a gift's recipient.

use std::collections::BTreeMap;
use std::sync::Mutex;

use acom_auth::Surface;
use acom_core::enums::ActorKind;
use acom_core::{StoreTime, TenantId};
use acom_llm::{ChatResponse, DeltaSink, ModelInfo, Usage};
use futures::future::BoxFuture;

use super::*;
use crate::blocks::Action;
use crate::blocks::review::fixtures::b2;
use crate::client::ChangeSetHead;
use crate::profile::fixtures::repo;
use crate::store::{FixedStore, MemoryLog};
use crate::tools::Manifest;

// ---------------------------------------------------------------- a scripted model

/// Answers each purpose (`route`, `plan`, `arguments`, `reply`) with its next recorded output.
#[derive(Debug)]
struct Recorded {
    info: ModelInfo,
    answers: Mutex<Vec<(String, String)>>,
    asked: Mutex<Vec<String>>,
}

impl Recorded {
    fn new(answers: Vec<(&str, Value)>) -> Arc<Self> {
        let r = Arc::new(Self {
            info: ModelInfo {
                alias: "glimmer".into(),
                model: "glimmer-test".into(),
                display_name: "Muse Glimmer 30B".into(),
                price: None,
            },
            answers: Mutex::new(Vec::new()),
            asked: Mutex::new(Vec::new()),
        });
        for (p, a) in answers {
            r.push(p, a);
        }
        r
    }

    /// Records one more answer (a later turn's plan names blocks of an earlier turn).
    fn push(&self, purpose: &str, answer: Value) {
        let text = match answer {
            Value::String(s) => s,
            other => other.to_string(),
        };
        self.answers
            .lock()
            .unwrap()
            .push((purpose.to_owned(), text));
    }

    fn asked(&self) -> Vec<String> {
        self.asked.lock().unwrap().clone()
    }
}

impl ChatModel for Recorded {
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
                    message: "no recorded answer".into(),
                }),
            }
        })
    }
}

// ---------------------------------------------------------------- the scenes' tools

fn id(prefix: &str, what: &str) -> Value {
    json!({"type": "string", "pattern": format!("^{prefix}_[a-z0-9_]+$"), "description": what})
}

fn money_schema() -> Value {
    json!({"type": "object", "additionalProperties": false, "required": ["value", "currency"],
        "description": "An amount.",
        "properties": {"value": {"type": "number", "description": "Amount."},
                       "currency": {"type": "string", "pattern": "^[A-Z]{3}$", "description": "Code."}}})
}

fn spec(
    tool: &str,
    address: &str,
    kind: &str,
    summary: &str,
    links: (Value, Value),
    properties: Value,
    required: Value,
) -> Value {
    json!({"tool": tool, "address": address, "kind": kind, "summary": summary,
           "consumes": links.0, "produces": links.1, "checks": [],
           "input_schema": {"type": "object", "additionalProperties": false,
                            "properties": properties, "required": required}})
}

fn one(slot: &str) -> Value {
    json!([{"name": slot, "many": false}])
}

fn many(slot: &str) -> Value {
    json!([{"name": slot, "many": true}])
}

/// The tools of the four scenes as commerce lists them (real names, addresses and summaries;
/// input schemas cut down to the fields the scenes use).
fn manifest() -> Vec<ToolSpec> {
    let none = json!([]);
    let address = json!({"type": "object", "additionalProperties": false,
        "required": ["street_address", "address_locality", "address_country"],
        "description": "Address for this order only.",
        "properties": {"name": {"type": "string", "description": "Name on the parcel."},
                       "street_address": {"type": "string", "description": "Street."},
                       "address_locality": {"type": "string", "description": "Town."},
                       "address_country": {"type": "string", "pattern": "^[A-Z]{2}$", "description": "Country."}}});
    let items = json!({"items": [
        spec("search_products", "search.product.search_products", "read",
             "Find products by words, category, tags, stock and price. Each product comes with its list price, the member price that applies to the caller, total stock and its variants.",
             (none.clone(), json!([{"name": "product_id", "many": true}])),
             json!({"query": {"type": "string", "description": "Words to look for."},
                    "max_price": money_schema(),
                    "limit": {"type": "integer", "minimum": 1, "maximum": 100, "description": "Max results."}}),
             json!([])),
        spec("get_product_measurements", "catalog.product.get_measurements", "read",
             "Garment measurements per size in cm (chest, shoulder, sleeve, length, …), with the same names on every product so sizes can be compared with what the shopper already owns.",
             (one("product_id"), none.clone()),
             json!({"product_id": id("prd", "Product."),
                    "sizes": {"type": "array", "items": {"type": "string", "description": "Size."}, "description": "Only these sizes."}}),
             json!(["product_id"])),
        spec("get_review_summary", "catalog.review.summarize", "read",
             "Reviews of a product summed up: rating, fit votes (true to size, runs small, runs large), phrases reviewers repeat, and a few reviews as evidence.",
             (one("product_id"), none.clone()),
             json!({"product_id": id("prd", "Product.")}),
             json!(["product_id"])),
        spec("get_my_orders", "sales.order.list_mine", "read",
             "The caller's own orders, newest first, with their lines and when they were delivered. Where returns and exchanges start.",
             (none.clone(), json!([{"name": "order_id", "many": true}, {"name": "order_item_id", "many": true}])),
             json!({"order_status": {"type": "array", "items": {"type": "string", "enum": ["delivered", "returned"], "description": "State."}, "description": "Only these states."}}),
             json!([])),
        spec("get_my_profile", "customer.person.get_my_profile", "read",
             "The signed-in shopper's own profile: sizes, remembered preferences, saved addresses, tier, and the saved card (last four digits only).",
             (none.clone(), one("customer_id")),
             json!({}), json!([])),
        spec("update_my_preferences", "customer.preference.update_my_preferences", "write",
             "Remember or forget something the shopper said about themselves (size, fit, fabrics, how they like results shown, travel priorities). The shopper sees the memory and can undo it.",
             (none.clone(), one("memory_id")),
             json!({"add": {"type": "array", "description": "Memories to add.", "items": {
                 "type": "object", "additionalProperties": false, "required": ["key", "value", "source"],
                 "properties": {"key": {"type": "string", "enum": ["size", "fit_note", "layout_preference", "other"], "description": "Kind."},
                                "value": {"type": "string", "description": "In the shopper's words."},
                                "source": {"type": "string", "enum": ["conversation"], "description": "Where from."}}}}}),
             json!([])),
        spec("get_cart", "sales.cart.get", "read",
             "The caller's own cart: lines, subtotal, one discount line per promotion, shipping, total, delivery estimate, payment method and the coupons the order will record.",
             (none.clone(), one("cart_id")), json!({}), json!([])),
        spec("add_to_cart", "sales.cart.add_to_cart", "write",
             "Put a product (with size and color) or a variant in the caller's own cart, creating the cart if needed. Runs right after its preview; no approval.",
             (one("product_id"), one("cart_id")),
             json!({"product_id": id("prd", "Product."), "size": {"type": "string", "description": "Size."},
                    "quantity": {"type": "integer", "minimum": 1, "description": "How many."}}),
             json!(["product_id"])),
        spec("prepare_gift_order", "sales.gift_order.prepare", "write",
             "Prepare a gift from the caller's own cart: the recipient, the card, the wrap, a gift receipt without prices, and the address for this order only (or a link the recipient accepts). Preview only; the sender places it with the Place order button. Nothing about the recipient is remembered.",
             (one("cart_id"), one("preview_id")),
             json!({"recipient_mode": {"type": "string", "enum": ["address_known", "recipient_accepts"], "description": "How it reaches them."},
                    "recipient_name": {"type": "string", "description": "Who receives it."},
                    "delivery_address": address,
                    "card_message": {"type": "string", "description": "Card."},
                    "gift_wrap": {"type": "boolean", "description": "Wrap."},
                    "gift_receipt": {"type": "boolean", "description": "Gift receipt."},
                    "client_total": money_schema()}),
             json!(["recipient_mode", "recipient_name", "client_total"])),
        spec("search_return_requests", "fulfillment.claim.search_return_requests", "read",
             "Return requests waiting for review, one queue row each (claim, order, customer, item, amount, reason, photos, days since delivery). The first call when reviewing returns in a batch.",
             (none.clone(), many("claim_id")),
             json!({"min_amount": money_schema(),
                    "limit": {"type": "integer", "minimum": 1, "maximum": 100, "description": "Page size."}}),
             json!([])),
        spec("evaluate_return_request", "fulfillment.claim.evaluate_return_request", "read",
             "Apply the return policy to one return request and suggest refund, ask_for_photos or decline, with the policy rule behind it, the refund amount, whether a person must approve, and flags such as frequent returns. Changes nothing.",
             (one("claim_id"), one("claim_id")),
             json!({"claim_id": id("clm", "Return request.")}),
             json!(["claim_id"])),
        spec("refund_return", "fulfillment.refund.create", "write",
             "Refund one return claim to the original payment. The server computes the amount from the claim (coupon share taken off); over the approval limit a person approves it in a change set. A refund cannot be undone.",
             (one("claim_id"), one("refund_id")),
             json!({"claim_id": id("clm", "Return claim.")}),
             json!(["claim_id"])),
        spec("send_customer_message", "messaging.message.send", "write_approval",
             "Send one shopper a message about their order or return (photo request, decline, refund notice, other). The preview shows the message exactly as they will read it; a person always approves, and a sent message cannot be taken back.",
             (json!([{"name": "customer_id", "many": false}, {"name": "order_id", "many": false}]), one("message_id")),
             json!({"customer_id": id("cus", "Shopper."), "order_id": id("ord", "Order."),
                    "kind": {"type": "string", "enum": ["ask_for_photos", "decline", "refund_done", "custom"], "description": "ask_for_photos, decline, refund_done or custom."},
                    "text": {"type": "string", "description": "The message as the shopper reads it."}}),
             json!(["customer_id", "kind", "text"])),
    ], "total": 13});
    serde_json::from_value::<Manifest>(items).unwrap().items
}

// ---------------------------------------------------------------- the scenes' commerce

type Reads = Box<dyn Fn(&str, &Value) -> Result<Value, ToolError> + Send + Sync>;

/// Commerce as the scenes' tools answer, recording every call.
struct Scene {
    manifest: Vec<ToolSpec>,
    reads: Reads,
    /// A guest has no memories (`not_signed_in`).
    guest: bool,
    calls: Mutex<Vec<String>>,
    previews: Mutex<Vec<(String, Value)>>,
    counter: Mutex<u32>,
}

impl Scene {
    fn new(
        guest: bool,
        reads: impl Fn(&str, &Value) -> Result<Value, ToolError> + Send + Sync + 'static,
    ) -> Arc<Self> {
        Arc::new(Self {
            manifest: manifest(),
            reads: Box::new(reads),
            guest,
            calls: Mutex::new(Vec::new()),
            previews: Mutex::new(Vec::new()),
            counter: Mutex::new(0),
        })
    }

    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }

    fn previews_of(&self, tool: &str) -> Vec<Value> {
        self.previews
            .lock()
            .unwrap()
            .iter()
            .filter(|(t, _)| t == tool)
            .map(|(_, i)| i.clone())
            .collect()
    }

    fn log(&self, s: String) {
        self.calls.lock().unwrap().push(s);
    }
}

fn passed(rule: &str, detail: &str) -> Value {
    json!({"rule": rule, "passed": true, "detail": detail, "fix_tool": null})
}

/// The refund each B2 claim gets, from the evaluations.
fn b2_refunds() -> BTreeMap<String, Value> {
    b2().1
        .iter()
        .flat_map(crate::executor::rows)
        .filter(|e| e["refund_amount"].is_object())
        .map(|e| {
            (
                e["claim_id"].as_str().unwrap().to_owned(),
                e["refund_amount"].clone(),
            )
        })
        .collect()
}

/// What `prepare_gift_order` previews for Jamie's gift (docs/api/sales.md §gift_order).
fn gift_preview(input: &Value) -> (Value, Value) {
    let changes = json!([{"op": "create", "entity": "order", "id": "", "to": {
        "order_type": "gift",
        "lines": [{"cart_line_id": "cln_1", "variant_id": "var_windbreaker_l_slate", "name": "Packable windbreaker",
                   "color": "Slate", "size": "L", "quantity": 1, "price": {"value": 89.00, "currency": "USD"}}],
        "subtotal": {"value": 89.00, "currency": "USD"},
        "discounts": [{"label": "VIP weekend", "amount": {"value": 17.80, "currency": "USD"}, "promotion_id": "promo_1042"}],
        "gift_wrap": {"value": 5.00, "currency": "USD"},
        "shipping": {"value": 0.00, "currency": "USD"},
        "goods_total": {"value": 71.20, "currency": "USD"},
        "total": {"value": 76.20, "currency": "USD"},
        "delivery": {"to": input["recipient_name"], "address_label": "Dad · 48 Pine Rd, Lakeview",
                     "arrives": "2026-11-10", "address_saved": false},
        "gift_options": {"wrap": input["gift_wrap"], "wrap_fee": {"value": 5.00, "currency": "USD"},
                         "card_message": input["card_message"], "gift_receipt": true,
                         "receipt_hides_prices": true, "exchange_until": "2026-12-31"},
        "recipient": {"name": input["recipient_name"], "mode": input["recipient_mode"]},
        "payment": {"payment_method": "credit_card", "label": "Card ending 4821"}}}]);
    let checks = json!([
        passed("gift.recipient_per_order", "One recipient: Dad"),
        passed(
            "checkout_validation.stock_available",
            "Packable windbreaker L Slate is in stock"
        ),
        passed(
            "checkout_validation.server_total_matches_client",
            "Recalculated with calculate_benefits: 89.00 − 17.80 + 0.00 = 71.20, same as shown; with the gift wrap (5.00) the gift comes to 76.20"
        ),
    ]);
    (changes, checks)
}

impl ToolClient for Scene {
    fn manifest(&self) -> BoxFuture<'_, Result<Vec<ToolSpec>, ToolError>> {
        self.log("manifest".into());
        Box::pin(async move { Ok(self.manifest.clone()) })
    }

    fn read<'a>(
        &'a self,
        tool: &'a str,
        input: &'a Value,
    ) -> BoxFuture<'a, Result<Value, ToolError>> {
        let about = ["claim_id", "product_id"]
            .iter()
            .find_map(|k| input.get(*k).and_then(Value::as_str))
            .map(|v| format!(" {v}"))
            .unwrap_or_default();
        self.log(format!("read {tool}{about}"));
        Box::pin(async move { (self.reads)(tool, input) })
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
        self.previews
            .lock()
            .unwrap()
            .push((tool.to_owned(), input.clone()));
        let n = {
            let mut n = self.counter.lock().unwrap();
            *n += 1;
            *n
        };
        Box::pin(async move {
            let (changes, checks, needs) = match tool {
                "refund_return" => {
                    let claim = input["claim_id"].as_str().unwrap_or_default();
                    let amount = b2_refunds().get(claim).cloned().unwrap_or(Value::Null);
                    (
                        json!([{"op": "create", "entity": "refund", "id": "", "to": {
                                   "claim_id": claim, "amount": amount, "refund_method": "original_payment",
                                   "to": "card on file"}},
                               {"op": "update", "entity": "claim", "id": claim, "from": "requested", "to": "approved"}]),
                        json!([
                            passed("refund.to", "Back to the card the order was paid with"),
                            passed(
                                "refund.coupon_deducted",
                                "Coupon share taken off where a coupon was used"
                            ),
                            passed(
                                "change_set.irreversible",
                                "A refund can't be undone once it is sent"
                            )
                        ]),
                        true,
                    )
                }
                "send_customer_message" => (
                    json!([{"op": "send", "entity": "message", "id": "", "to": {
                        "recipient": "C*** S***", "customer_id": input["customer_id"], "about": input["order_id"],
                        "kind": input["kind"], "channel": "email", "text": input["text"]}}]),
                    json!([
                        passed(
                            "customer_messages",
                            "Every customer message needs a person's approval"
                        ),
                        passed(
                            "change_set.irreversible",
                            "A sent message can't be taken back"
                        )
                    ]),
                    true,
                ),
                "update_my_preferences" if self.guest => {
                    return Err(ToolError::new(
                        ErrorCode::new("not_signed_in", acom_contract::ErrorClass::Unauthenticated),
                        "Sign in first; guests have no memories.",
                    ));
                }
                "update_my_preferences" => (
                    json!([{"op": "create", "entity": "preference", "id": "", "to": input["add"][0]}]),
                    json!([passed(
                        "preference.about_shopper_only",
                        "About the shopper, not a gift recipient"
                    )]),
                    false,
                ),
                "add_to_cart" => (
                    json!([{"op": "create", "entity": "cart_line", "id": "", "to": input}]),
                    json!([]),
                    false,
                ),
                "prepare_gift_order" => {
                    let (c, k) = gift_preview(input);
                    (c, k, false)
                }
                other => {
                    return Err(ToolError::invalid_input(format!(
                        "{other} has no preview in this scene"
                    )));
                }
            };
            Ok(serde_json::from_value(json!({
                "preview_id": format!("pv_{n}"), "expires_at": null, "changes": changes,
                "checks": checks, "needs_approval": needs, "approval_reason": null,
                "change_set_id": change_set_id,
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
        let value = self
            .previews_of("update_my_preferences")
            .last()
            .and_then(|i| i["add"][0]["value"].as_str().map(str::to_owned))
            .unwrap_or_default();
        Box::pin(async move {
            Ok(match tool {
                "update_my_preferences" => json!({"ok": true, "id": "mem_21",
                    "memory": {"memory_id": "mem_21", "text": format!("Remembered: {value}")},
                    "undo": {"by": "person", "target": "mem_21", "until": null}}),
                "add_to_cart" => json!({"ok": true, "id": "cart_1", "undo": null}),
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
                id: "cs_1043".into(),
                number: Some(1043),
                version: 1,
                state: "drafting".into(),
            })
        })
    }

    fn replace_items<'a>(
        &'a self,
        change_set_id: &'a str,
        _: &'a [ItemRef],
    ) -> BoxFuture<'a, Result<ChangeSetHead, ToolError>> {
        self.log(format!("replace_items {change_set_id}"));
        Box::pin(async move {
            Ok(ChangeSetHead {
                id: change_set_id.into(),
                number: None,
                version: 2,
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

// ---------------------------------------------------------------- running turns

fn at(local: &str) -> StoreSettings {
    let time = StoreTime::new(chrono_tz::America::New_York);
    StoreSettings {
        now: time.parse(local).unwrap(),
        time,
        currency: "USD".into(),
    }
}

fn person(subject: &str, surface: Surface) -> Principal {
    Principal {
        subject: subject.into(),
        kind: ActorKind::AgentOnBehalf,
        tenant: TenantId::parse("store_northwind").unwrap(),
        surface,
        scopes: ["changeset:draft"]
            .iter()
            .map(|s| (*s).to_owned())
            .collect(),
        name: None,
        session_id: Some(format!("ses_{subject}")),
    }
}

fn runtime(model: &Arc<Recorded>, clock: &str) -> Runtime {
    let set = ModelSet {
        models: [("glimmer".to_owned(), model.clone() as Arc<dyn ChatModel>)]
            .into_iter()
            .collect(),
        missing: vec![],
    };
    Runtime::new(
        Arc::new(repo()),
        set,
        Arc::new(MemoryLog::default()),
        Arc::new(FixedStore(at(clock))),
    )
}

async fn turn(
    rt: &Runtime,
    who: &Principal,
    surface: ChatSurface,
    request: TurnRequest,
    commerce: &Arc<Scene>,
) -> Vec<Event> {
    let p = rt
        .prepare(who.clone(), "req_scene".into(), surface, request)
        .await
        .unwrap();
    let (emit, mut rx) = Emitter::channel();
    rt.run(p, commerce.clone(), emit).await;
    let mut events = Vec::new();
    while let Ok(e) = rx.try_recv() {
        events.push(e);
    }
    events
}

fn say(text: &str, conversation: Option<&str>) -> TurnRequest {
    TurnRequest {
        message: Some(text.into()),
        conversation_id: conversation.map(str::to_owned),
        ..TurnRequest::default()
    }
}

fn conversation(events: &[Event]) -> String {
    events[0].data()["conversation_id"]
        .as_str()
        .unwrap()
        .to_owned()
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

fn block<'a>(events: &'a [Event], ui: &str) -> &'a Block {
    blocks(events)
        .into_iter()
        .find(|b| b.ui == ui)
        .unwrap_or_else(|| {
            let steps: Vec<String> = events
                .iter()
                .filter_map(|e| match e {
                    Event::Step { summary, .. } => Some(summary.clone()),
                    Event::MessageDone { text, .. } => Some(format!("reply: {text}")),
                    _ => None,
                })
                .collect();
            panic!("no {ui} block; steps: {steps:#?}")
        })
}

fn reply(events: &[Event]) -> String {
    events
        .iter()
        .find_map(|e| match e {
            Event::MessageDone { text, .. } => Some(text.clone()),
            _ => None,
        })
        .unwrap_or_default()
}

fn tool_calls(events: &[Event]) -> u32 {
    match events.last() {
        Some(Event::TurnDone { usage, .. }) => usage.tool_calls,
        other => panic!("the last event is not turn.done: {other:?}"),
    }
}

fn person_labels(b: &Block) -> Vec<(String, String)> {
    b.person_actions()
        .filter_map(|a| match a {
            Action::Person { label, path, .. } => Some((label.clone(), path.clone())),
            _ => None,
        })
        .collect()
}

fn not_found(tool: &str) -> ToolError {
    ToolError::new(
        ErrorCode::NOT_FOUND,
        format!("{tool}: nothing in this scene"),
    )
}

// ---------------------------------------------------------------- B2

fn b2_reads(tool: &str, input: &Value) -> Result<Value, ToolError> {
    let (queue, evaluations) = b2();
    match tool {
        "search_return_requests" => Ok(queue),
        "evaluate_return_request" => evaluations
            .into_iter()
            .find(|e| e["items"][0]["claim_id"] == input["claim_id"])
            .ok_or_else(|| not_found(tool)),
        _ => Err(not_found(tool)),
    }
}

const B2_ASK: &str = "Review the return requests over $200 that are waiting.";
const B2_KEEP: &str = "Keep the decline, but be kind in the message. Go ahead with the refunds and the photo requests.";

#[tokio::test]
async fn b2_twelve_requests_become_one_change_set_of_nine_refunds_and_three_messages() {
    let glimmer = Recorded::new(vec![
        ("route", json!({"agent": "orders", "confidence": 0.92})),
        (
            "plan",
            json!({
            "title": "Returns over $200",
            "interpretation": [{"aspect": "other", "text": "Return requests over $200 waiting for review"}],
            "about": "self",
            "steps": [
                {"id": "s1", "tool": "search_return_requests",
                 "args": {"min_amount": {"value": 200, "currency": "USD"}, "limit": 100}, "uses": []},
                {"id": "s2", "tool": "evaluate_return_request", "args": {}, "uses": [], "each": "s1"}],
            "reply_plan": "refund_review"}),
        ),
        (
            "reply",
            json!(
                "Of the 12 requests, 9 can be refunded for $2,463.00, 2 need photos first, and 1 is outside the return window."
            ),
        ),
    ]);
    let rt = runtime(&glimmer, "2026-11-06T10:40");
    let commerce = Scene::new(false, b2_reads);
    let sj = person("adm_sj", Surface::Bo);

    // 10:40, read only: the queue and one evaluation per request.
    let first = turn(&rt, &sj, ChatSurface::Bo, say(B2_ASK, None), &commerce).await;
    assert_eq!(first[0].data()["agent"], "orders");
    let calls = commerce.calls();
    assert_eq!(calls[..2], ["manifest", "read search_return_requests"]);
    assert_eq!(
        calls
            .iter()
            .filter(|c| c.starts_with("read evaluate_return_request clm_"))
            .count(),
        12,
        "{calls:#?}"
    );
    assert!(
        !calls
            .iter()
            .any(|c| c.starts_with("preview") || c.starts_with("create")),
        "reading the queue writes nothing"
    );
    assert_eq!(tool_calls(&first), 13);
    let review = block(&first, "refund_review");
    let s = &review.data["summary"];
    assert_eq!(s["waiting"], 12);
    assert_eq!(s["refund"]["count"], 9);
    assert_eq!(s["refund"]["amount"]["value"].to_string(), "2463.00");
    assert_eq!(
        (s["ask_photos"].clone(), s["decline"].clone()),
        (json!(2), json!(1))
    );
    let cashmere = review.data["requests"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["claim_id"] == "clm_20290")
        .unwrap();
    assert_eq!(cashmere["suggested"], "decline");
    assert_eq!(cashmere["day"], 39);
    let flag = review.data["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["rule"] == "return_suggestions.flag_return_history")
        .unwrap();
    assert_eq!(flag["detail"], "#20431: 4 returns in the last 90 days");
    assert!(
        review
            .suggested_intents
            .iter()
            .any(|i| i.say == "Show me this customer's return history")
    );
    assert_eq!(review.person_actions().count(), 0, "nothing to approve yet");
    assert!(blocks(&first).iter().all(|b| b.ui != "change_set"));
    assert!(reply(&first).contains("$2,463.00"));

    // The operator keeps the decline and asks for a kind message: the refunds and messages are
    // drafted from the rows the review showed, into one change set, without reading again.
    let review_id = review.block_id.clone();
    let kind_decline = "Thank you for writing to us. Your sweater arrived more than a month ago, past our return window, so we can't take it back this time. We're sorry, and we hope to see you again.";
    let photos = "Sorry your order arrived damaged. Could you reply with a photo or two of the damage? We'll take it from there.";
    glimmer.push("route", json!({"agent": "orders", "confidence": 0.9}));
    glimmer.push("plan", json!({
        "title": "Returns over $200",
        "interpretation": [{"aspect": "other", "text": "Refund the 9, ask 2 for photos, decline 1 kindly"}],
        "about": "self",
        "steps": [
            {"id": "s1", "tool": "refund_return", "args": {}, "uses": [],
             "each": review_id, "when": {"suggested": "refund"}},
            {"id": "s2", "tool": "send_customer_message", "args": {"kind": "ask_for_photos", "text": photos},
             "uses": [], "each": review_id, "when": {"suggested": "ask_for_photos"}},
            {"id": "s3", "tool": "send_customer_message", "args": {"kind": "decline", "text": kind_decline},
             "uses": [], "each": review_id, "when": {"suggested": "decline"}}],
        "reply_plan": "refund_review"}));
    glimmer.push("reply", json!("Change set #1043 holds the 9 refunds and the messages, and it is waiting for your approval. Messages cannot be taken back once sent."));
    let before = commerce.calls().len();
    let second = turn(
        &rt,
        &sj,
        ChatSurface::Bo,
        say(B2_KEEP, Some(&conversation(&first))),
        &commerce,
    )
    .await;
    let calls: Vec<String> = commerce.calls()[before..].to_vec();
    assert!(
        !calls.iter().any(|c| c.starts_with("read")),
        "nothing is read again: {calls:#?}"
    );
    assert_eq!(calls[1], "create_change_set Returns over $200");
    assert_eq!(
        calls
            .iter()
            .filter(|c| *c == "preview refund_return into cs_1043")
            .count(),
        9
    );
    assert_eq!(
        calls
            .iter()
            .filter(|c| *c == "preview send_customer_message into cs_1043")
            .count(),
        3
    );
    assert_eq!(
        calls.last().unwrap(),
        "submit cs_1043 v1",
        "handed to people, never approved"
    );
    assert_eq!(tool_calls(&second), 12);
    let refunded: Vec<Value> = commerce
        .previews_of("refund_return")
        .iter()
        .map(|i| i["claim_id"].clone())
        .collect();
    assert!(refunded.contains(&json!("clm_20402")) && !refunded.contains(&json!("clm_20290")));
    let messages = commerce.previews_of("send_customer_message");
    let decline = messages.iter().find(|m| m["kind"] == "decline").unwrap();
    assert_eq!(
        (decline["customer_id"].clone(), decline["order_id"].clone()),
        (json!("cus_priya"), json!("ord_20290")),
        "ids come from the review's rows, not from the model"
    );
    assert_eq!(decline["text"], kind_decline);

    let review = block(&second, "refund_review");
    assert!(
        blocks(&second).iter().all(|b| b.ui != "change_set"),
        "the review carries its change set"
    );
    assert_eq!(review.data["change_set_id"], "cs_1043");
    assert_eq!(review.data["summary"]["refund"]["count"], 9);
    assert_eq!(
        review.data["summary"]["refund"]["amount"]["value"].to_string(),
        "2463.00"
    );
    assert_eq!(review.data["messages"].as_array().unwrap().len(), 3);
    let approve = person_labels(review);
    assert_eq!(
        approve[0],
        (
            "Approve 9 refunds and send 3 messages".to_owned(),
            "/v1/platform/change-sets/cs_1043/approve".to_owned()
        )
    );
    assert!(second.iter().any(|e| matches!(e,
        Event::ChangeSetUpdated { state, .. } if state == "waiting_for_approval")));
    assert!(reply(&second).contains("waiting for your approval"));
}

// ---------------------------------------------------------------- F3

fn coat() -> Value {
    json!({"product_id": "prd_wool_overcoat", "name": "Wool overcoat", "price": 289.00, "price_currency": "USD",
           "member_price": null, "color": "Charcoal", "material": "70% wool",
           "summary": "Free size exchange within 30 days", "image": "https://cdn.northwind.example/p/wool_overcoat.webp"})
}

fn f3_reads(tool: &str, input: &Value) -> Result<Value, ToolError> {
    match (tool, input["product_id"].as_str()) {
        ("search_products", _) => Ok(json!({"items": [coat()], "total": 1})),
        ("get_product_measurements", Some("prd_wool_overcoat")) => Ok(json!({
            "product_id": "prd_wool_overcoat", "unit": "cm",
            "measurements": ["chest", "shoulder", "sleeve", "length"],
            "sizes": [{"size": "M", "chest": 56, "shoulder": 46, "sleeve": 64, "length": 98},
                      {"size": "L", "chest": 59, "shoulder": 48, "sleeve": 65.5, "length": 100}],
            "how_measured": "Garment laid flat."})),
        ("get_product_measurements", Some("prd_windbreaker")) => Ok(json!({
            "product_id": "prd_windbreaker", "unit": "cm",
            "measurements": ["chest", "shoulder", "sleeve", "length"],
            "sizes": [{"size": "M", "chest": 57, "shoulder": 47, "sleeve": 64, "length": 71}]})),
        ("get_review_summary", _) => Ok(json!({
            "product_id": "prd_wool_overcoat", "review_count": 41, "rating_value": 4.6,
            "filter": {"height_min_cm": 175, "height_max_cm": 182, "size": null},
            "fit_votes": [{"fit": "true_to_size", "label": "True to size", "count": 29},
                          {"fit": "runs_small", "label": "A bit snug", "count": 12},
                          {"fit": "runs_large", "label": "Runs large", "count": 0}],
            "mentions": [{"text": "size up for a hoodie", "count": 9}], "sample": []})),
        // Jamie's two most recent orders: the merino tee, then the windbreaker M she kept.
        ("get_my_orders", _) => Ok(json!({"items": [
            {"id": "ord_20381", "order_number": "20381", "order_status": "delivered", "ordered_item": [
                {"id": "oln_1", "product_id": "prd_merino_tee", "name": "Merino trail tee", "size": "M",
                 "color": "Charcoal", "order_quantity": 1, "order_item_status": "delivered"}]},
            {"id": "ord_20268", "order_number": "20268", "order_status": "delivered", "ordered_item": [
                {"id": "oln_1", "product_id": "prd_windbreaker", "name": "Packable windbreaker", "size": "M",
                 "color": "Black", "order_quantity": 1, "order_item_status": "delivered"}]}],
            "total": 6})),
        _ => Err(not_found(tool)),
    }
}

fn f3_plan(first: &str, then: &str, remember: bool) -> Value {
    let mut steps = vec![
        json!({"id": "s1", "tool": "search_products", "args": {"query": "wool overcoat"}, "uses": []}),
        json!({"id": "s2", "tool": "get_product_measurements", "args": {"sizes": ["M", "L"]}, "uses": ["s1"]}),
        json!({"id": "s3", "tool": "get_review_summary", "args": {}, "uses": ["s1"]}),
        json!({"id": "s4", "tool": "get_my_orders", "args": {"order_status": ["delivered"]}, "uses": []}),
        json!({"id": "s5", "tool": "get_product_measurements", "args": {"sizes": ["M"]}, "uses": ["s4"]}),
    ];
    if remember {
        steps.push(json!({"id": "s6", "tool": "update_my_preferences", "uses": [], "args": {
            "add": [{"key": "fit_note", "value": "you layer hoodies under coats", "source": "conversation"}]}}));
    }
    json!({"about": "self",
           "interpretation": [{"aspect": "products", "text": "Wool overcoat"}],
           "steps": steps,
           "notes": [{"element": format!("size:{first}"), "text": if first == "L" { "Over a hoodie" } else { "Over a shirt or sweater" }},
                     {"element": format!("size:{then}"), "text": if then == "L" { "Over a hoodie" } else { "Over a shirt or sweater" }}],
           "reply_plan": "fit_check"})
}

#[tokio::test]
async fn f3_fit_check_compares_the_coat_with_the_windbreaker_she_kept() {
    let glimmer = Recorded::new(vec![
        ("plan", f3_plan("L", "M", true)),
        // The windbreaker's id comes from her orders, after they were read.
        (
            "arguments",
            json!({"product_id": "prd_windbreaker", "sizes": ["M"]}),
        ),
        (
            "reply",
            json!(
                "Probably not M over a hoodie. M is about the size of your windbreaker, which you kept, but over a hoodie you'll want more room in the chest. I'd go with L."
            ),
        ),
    ]);
    let rt = runtime(&glimmer, "2026-11-07T10:00");
    let commerce = Scene::new(false, f3_reads);
    let jamie = person("cus_jamie", Surface::Fo);
    let events = turn(
        &rt,
        &jamie,
        ChatSurface::Fo,
        say(
            "Will M fit in the wool overcoat? I want to wear it over a hoodie.",
            None,
        ),
        &commerce,
    )
    .await;
    assert_eq!(
        commerce.calls(),
        [
            "manifest",
            "read search_products",
            "read get_product_measurements prd_wool_overcoat",
            "read get_review_summary prd_wool_overcoat",
            "read get_my_orders",
            "read get_product_measurements prd_windbreaker",
            "preview update_my_preferences",
            "commit update_my_preferences pv_1",
        ]
    );
    assert_eq!(glimmer.asked(), ["plan", "arguments", "reply"]);
    assert!(events.iter().any(|e| matches!(e,
        Event::MemorySaved { text, .. } if text == "Remembered: you layer hoodies under coats")));
    let note = block(&events, "memory_note");
    assert_eq!(
        person_labels(note),
        [(
            "Undo".to_owned(),
            "/v1/customer/me/memories/mem_21/undo".to_owned()
        )]
    );

    let fit = block(&events, "fit_check");
    let d = &fit.data;
    assert_eq!(d["product"]["name"], "Wool overcoat");
    assert_eq!(d["product"]["price"]["value"].to_string(), "289.0");
    assert_eq!(
        (
            d["options"][0]["size"].clone(),
            d["options"][0]["label"].clone(),
            d["options"][0]["recommended"].clone()
        ),
        (json!("L"), json!("Over a hoodie"), json!(true))
    );
    assert_eq!(d["options"][1]["size"], "M");
    assert_eq!(
        d["options"][1]["reason"],
        "Chest 56 cm; your packable windbreaker M, which you kept, is 57 cm"
    );
    assert_eq!(
        d["measurements"]["columns"],
        json!([
            "",
            "Wool overcoat, M",
            "Wool overcoat, L",
            "Your packable windbreaker M"
        ])
    );
    assert_eq!(
        d["measurements"]["rows"][0],
        json!({"label": "Chest", "values": ["56 cm", "59 cm", "57 cm"]})
    );
    assert_eq!(d["reviews"]["sample"], "41 reviews from people 175–182 cm");
    assert_eq!(
        d["reviews"]["fit"],
        json!(["29 say true to size", "12 say a bit snug"])
    );
    assert_eq!(d["reviews"]["mentions"][0]["count"], 9);
    assert_eq!(
        d["used"][0],
        json!({"label": "Your orders", "value": "Packable windbreaker M, kept"})
    );
    assert_eq!(d["used"][1]["value"], "You layer hoodies under coats");
    assert_eq!(d["not_used"][0]["label"], "Height and weight");
    assert!(fit.actions.iter().any(|a| matches!(a,
        Action::Say { label, args, .. } if label == "Add L · $289.00" && args["size"] == "L")));
    assert_eq!(fit.person_actions().count(), 0);

    // "What if I just wear a sweater?" Then M comes first.
    glimmer.push("plan", f3_plan("M", "L", false));
    glimmer.push(
        "arguments",
        json!({"product_id": "prd_windbreaker", "sizes": ["M"]}),
    );
    glimmer.push(
        "reply",
        json!("Then M. Both sizes are on the right so you can see the difference."),
    );
    let again = turn(
        &rt,
        &jamie,
        ChatSurface::Fo,
        say(
            "What if I just wear a sweater?",
            Some(&conversation(&events)),
        ),
        &commerce,
    )
    .await;
    let fit = block(&again, "fit_check");
    assert_eq!(fit.data["options"][0]["size"], "M");
    assert_eq!(fit.data["options"][0]["label"], "Over a shirt or sweater");
    assert!(!again.iter().any(|e| e.name() == "memory.saved"));
}

// ---------------------------------------------------------------- F4

fn f4_reads(tool: &str, _: &Value) -> Result<Value, ToolError> {
    match tool {
        // While change set #1042's VIP weekend sale runs, Jamie (VIP) sees $71.20.
        "search_products" => Ok(json!({"items": [
            {"product_id": "prd_windbreaker", "name": "Packable windbreaker", "price": 89.00, "price_currency": "USD",
             "member_price": {"price": 71.20, "name": "VIP weekend", "valid_for_member_tier": "vip", "promotion_id": "promo_1042"},
             "description": "Packs into its own pocket, handles light rain", "image": "https://cdn.northwind.example/p/windbreaker.webp"},
            {"product_id": "prd_fleece_half_zip", "name": "Fleece half-zip", "price": 84.00, "price_currency": "USD",
             "member_price": null, "description": "Warm layer for cold starts, fits under a shell"},
            {"product_id": "prd_merino_tee", "name": "Merino trail tee", "price": 58.00, "price_currency": "USD",
             "member_price": null, "description": "Doesn't smell after a long day"}], "total": 3})),
        "get_my_profile" => Ok(json!({"id": "cus_jamie", "name": "Jamie", "tier_id": "vip",
            "sizes": {"tops": "M", "bottoms": null, "shoes": null},
            "preferences": [{"memory_id": "mem_03", "key": "size", "value": "M", "text": "Size M", "source": "conversation"}]})),
        "get_cart" => Ok(json!({"id": "cart_1", "lines": [
            {"cart_line_id": "cln_1", "product_id": "prd_windbreaker", "variant_id": "var_windbreaker_l_slate",
             "name": "Packable windbreaker", "color": "Slate", "size": "L", "quantity": 1,
             "price": {"value": 89.00, "currency": "USD"}}],
            "subtotal": {"value": 89.00, "currency": "USD"},
            "discounts": [{"label": "VIP weekend", "amount": {"value": 17.80, "currency": "USD"}}],
            "shipping": {"value": 0.00, "currency": "USD"},
            "total": {"value": 71.20, "currency": "USD"},
            "delivery": {"address_label": "Home · 12 Harbor St, Apt 3", "arrives": "2026-11-10"}})),
        _ => Err(not_found(tool)),
    }
}

#[tokio::test]
async fn f4_a_gift_for_dad_is_prepared_for_76_20_and_nothing_is_remembered() {
    let chips = |extra: bool| {
        let mut c = vec![
            json!({"aspect": "who", "text": "Gift for Dad", "source": "said"}),
            json!({"aspect": "products", "text": "Hiking", "source": "his"}),
            json!({"aspect": "benefit", "text": "Under $100", "source": "said"}),
            json!({"aspect": "when", "text": "Arrives by Nov 14", "source": "said"}),
        ];
        if extra {
            c.push(json!({"aspect": "other", "text": "Size L", "source": "his"}));
        }
        c
    };
    let glimmer = Recorded::new(vec![
        (
            "plan",
            json!({"about": "other", "interpretation": chips(false), "reply_plan": "gift_picker",
            "steps": [{"id": "s1", "tool": "search_products", "uses": [],
                       "args": {"query": "hiking", "max_price": {"value": 100, "currency": "USD"}, "limit": 3}}]}),
        ),
        (
            "reply",
            json!(
                "Here are three that hikers keep using. I won't save any of this to your profile, since it's about your dad, not you."
            ),
        ),
    ]);
    let rt = runtime(&glimmer, "2026-11-07T10:00");
    let commerce = Scene::new(false, f4_reads);
    let jamie = person("cus_jamie", Surface::Fo);
    let first = turn(
        &rt,
        &jamie,
        ChatSurface::Fo,
        say(
            "A gift for my dad. He hikes a lot. Under $100, and it has to arrive by Nov 14.",
            None,
        ),
        &commerce,
    )
    .await;
    assert_eq!(commerce.calls(), ["manifest", "read search_products"]);
    let shown: Vec<&str> = blocks(&first).iter().map(|b| b.ui.as_str()).collect();
    assert_eq!(shown, ["looking_for", "gift_picker"]);
    let looking = block(&first, "looking_for");
    assert_eq!(
        looking.data["chips"][1],
        json!({"label": "Hiking", "source": "his", "removable": true})
    );
    let picker = block(&first, "gift_picker");
    let picks = picker.data["picks"].as_array().unwrap();
    assert_eq!(picks.len(), 3);
    assert_eq!(
        picks[0]["price"]["value"].to_string(),
        "71.2",
        "the VIP weekend price"
    );
    assert_eq!(picks[0]["list_price"]["value"].to_string(), "89.0");
    assert_eq!(picks[1]["price"]["value"].to_string(), "84.0");
    assert_eq!(picks[2]["price"]["value"].to_string(), "58.0");
    assert_eq!(
        picks[1]["why"],
        "Warm layer for cold starts, fits under a shell"
    );
    assert!(picks.iter().all(|p| p["chosen_size"].is_null()));

    // "He's about my size, maybe a bit bigger." Jamie points at the windbreaker.
    glimmer.push("plan", json!({"about": "other", "interpretation": chips(true), "reply_plan": "gift_picker",
        "steps": [
            {"id": "s1", "tool": "get_my_profile", "args": {}, "uses": []},
            {"id": "s2", "tool": "add_to_cart", "args": {"size": "L", "quantity": 1}, "uses": []},
            {"id": "s3", "tool": "get_cart", "args": {}, "uses": []},
            {"id": "s4", "tool": "prepare_gift_order", "uses": ["s3"], "args": {
                "recipient_mode": "address_known", "recipient_name": "Dad",
                "delivery_address": {"name": "Dad", "street_address": "48 Pine Rd", "address_locality": "Lakeview", "address_country": "US"},
                "card_message": "For the next trail. Love, J.", "gift_wrap": true, "gift_receipt": true}}]}));
    glimmer.push("reply", json!("Then L. If it doesn't fit he can exchange it himself with the gift receipt, no need to involve you."));
    let mut ask = say(
        "He's about my size, maybe a bit bigger. Send it to Dad at 48 Pine Rd, Lakeview, with gift wrap and a card: For the next trail. Love, J.",
        Some(&conversation(&first)),
    );
    ask.context = vec![ElementContext {
        element_ref: picker.element_ref("product_card:prd_windbreaker"),
        kind: "product_card".into(),
        label: "Packable windbreaker · $71.20".into(),
        ids: BTreeMap::from([("product_id".to_owned(), "prd_windbreaker".to_owned())]),
    }];
    let before = commerce.calls().len();
    let second = turn(&rt, &jamie, ChatSurface::Fo, ask, &commerce).await;
    assert_eq!(
        commerce.calls()[before..],
        [
            "manifest",
            "read get_my_profile",
            "preview add_to_cart",
            "commit add_to_cart pv_1",
            "read get_cart",
            "preview prepare_gift_order",
        ],
        "the gift is only prepared; nothing is remembered"
    );
    assert!(!second.iter().any(|e| e.name() == "memory.saved"));
    let added = &commerce.previews_of("add_to_cart")[0];
    assert_eq!(
        (added["product_id"].clone(), added["size"].clone()),
        (json!("prd_windbreaker"), json!("L"))
    );
    let prepared = &commerce.previews_of("prepare_gift_order")[0];
    assert_eq!(
        prepared["client_total"]["value"].to_string(),
        "71.2",
        "the total the cart showed"
    );
    assert_eq!(prepared["delivery_address"]["street_address"], "48 Pine Rd");

    let shown: Vec<&str> = blocks(&second).iter().map(|b| b.ui.as_str()).collect();
    assert_eq!(shown, ["looking_for", "gift_picker", "checkout_card"]);
    let chips = &block(&second, "looking_for").data["chips"];
    assert_eq!(
        chips[4],
        json!({"label": "Size L", "source": "his", "removable": true})
    );
    let picker = block(&second, "gift_picker");
    let d = &picker.data;
    assert_eq!(d["picks"][0]["name"], "Packable windbreaker");
    assert_eq!(
        d["picks"][0]["price"]["value"].to_string(),
        "71.2",
        "the same picks, same prices"
    );
    assert_eq!(d["picks"][0]["chosen_size"], "L");
    assert!(d["picks"][1]["chosen_size"].is_null());
    assert_eq!(
        d["gift_options"],
        json!({"wrap": "Gift wrap, +$5.00", "card_message": "For the next trail. Love, J.",
        "receipt": "Gift receipt, no prices shown", "exchange_until": "2026-12-31"})
    );
    assert_eq!(
        d["delivery"],
        json!({"to": "Dad · 48 Pine Rd, Lakeview", "arrives": "2026-11-10", "address_saved": false})
    );
    assert_eq!(d["total"]["value"].to_string(), "76.2");
    let card = block(&second, "checkout_card");
    assert!(picker.actions.iter().any(|a| matches!(a,
        Action::Open { label, target, .. } if label == "Review gift order · $76.20" && *target == format!("block:{}", card.block_id))));
    assert_eq!(picker.person_actions().count(), 0);
    match card.person_actions().collect::<Vec<_>>()[..] {
        [
            Action::Person {
                label, path, body, ..
            },
        ] => {
            assert_eq!(label, "Place order · $76.20");
            assert_eq!(path, "/v1/sales/orders");
            assert_eq!(body, &json!({"preview_id": "pv_2"}));
        }
        ref other => panic!("{other:?}"),
    }
    assert_eq!(
        card.data["gift_options"]["card_message"],
        "For the next trail. Love, J."
    );
}

// ---------------------------------------------------------------- F6

fn jackets() -> Value {
    json!({"items": [
        {"product_id": "prd_windbreaker", "name": "Packable windbreaker", "price": 89.00, "price_currency": "USD",
         "weight_g": 190, "material": "Recycled nylon", "image": "https://cdn.northwind.example/p/windbreaker.webp"},
        {"product_id": "prd_field_jacket", "name": "Linen blend field jacket", "price": 128.00, "price_currency": "USD",
         "weight_g": 340, "material": "Linen, cotton", "image": "https://cdn.northwind.example/p/field_jacket.webp"},
        {"product_id": "prd_seersucker_overshirt", "name": "Seersucker overshirt", "price": 79.00, "price_currency": "USD",
         "weight_g": 360, "material": "Cotton", "image": "https://cdn.northwind.example/p/seersucker_overshirt.webp"}],
        "total": 3})
}

/// A signed-in shopper whose profile says nothing about layout, or one who asked for photos.
fn f6_reads(photos: bool) -> impl Fn(&str, &Value) -> Result<Value, ToolError> + Send + Sync {
    move |tool, _| match tool {
        "search_products" => Ok(jackets()),
        "get_my_profile" => {
            let mut prefs = vec![json!({"memory_id": "mem_a1", "key": "size", "value": "M",
                                        "text": "Size M", "source": "conversation"})];
            if photos {
                prefs.push(json!({"memory_id": "mem_21", "key": "layout_preference",
                                  "value": "photos over details", "text": "Photos over details",
                                  "source": "conversation"}));
            }
            Ok(json!({"id": "cus_mina", "name": "Mina", "preferences": prefs}))
        }
        _ => Err(not_found(tool)),
    }
}

fn find_jackets() -> Value {
    json!({"about": "self", "interpretation": [], "reply_plan": "product_list", "steps": [
        {"id": "s1", "tool": "get_my_profile", "args": {}, "uses": []},
        {"id": "s2", "tool": "search_products", "args": {"query": "light jacket for spring trips"}, "uses": []}]})
}

fn remember_photos() -> Value {
    json!({"about": "self", "interpretation": [], "reply_plan": "look_gallery", "steps": [
        {"id": "s1", "tool": "update_my_preferences", "uses": [], "args": {
            "add": [{"key": "layout_preference", "value": "photos over details", "source": "conversation"}]}}]})
}

#[tokio::test]
async fn f6_less_text_shows_the_same_results_as_photos_and_remembers_it_with_an_undo() {
    let glimmer = Recorded::new(vec![
        ("plan", find_jackets()),
        ("plan", remember_photos()),
        ("plan", find_jackets()),
    ]);
    let rt = runtime(&glimmer, "2026-11-07T10:00");
    let commerce = Scene::new(false, f6_reads(false));
    let mina = person("cus_mina", Surface::Fo);
    let first = turn(
        &rt,
        &mina,
        ChatSurface::Fo,
        say("Light jacket for spring trips", None),
        &commerce,
    )
    .await;
    let list = block(&first, "product_list");
    assert_eq!(list.data["items"].as_array().unwrap().len(), 3);
    assert!(
        list.data["items"][0]["facts"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f == "weight g: 190")
    );

    let before = commerce.calls().len();
    let second = turn(
        &rt,
        &mina,
        ChatSurface::Fo,
        say(
            "Less text. Just show me bigger photos.",
            Some(&conversation(&first)),
        ),
        &commerce,
    )
    .await;
    assert_eq!(
        commerce.calls()[before..],
        [
            "manifest",
            "preview update_my_preferences",
            "commit update_my_preferences pv_1"
        ],
        "the same results, laid out again: nothing is searched again"
    );
    let remembered = &commerce.previews_of("update_my_preferences")[0]["add"][0];
    assert_eq!(remembered["key"], "layout_preference");
    let note = block(&second, "memory_note");
    assert_eq!(note.data["text"], "Remembered: photos over details");
    assert_eq!(
        person_labels(note),
        [(
            "Undo".to_owned(),
            "/v1/customer/me/memories/mem_21/undo".to_owned()
        )],
        "one line Mina can undo, through the person-only memory endpoint"
    );
    let gallery = block(&second, "look_gallery");
    assert_eq!(
        gallery.why.as_deref(),
        Some("Shown as photos. Say \"compare them\" for a table.")
    );
    let items = gallery.data["items"].as_array().unwrap();
    assert_eq!(items.len(), 3);
    for (shown, before) in items.iter().zip(list.data["items"].as_array().unwrap()) {
        assert_eq!(shown["product_id"], before["product_id"]);
        assert_eq!(shown["price"], before["price"], "prices are the tools' own");
    }
    assert!(
        items[0]["image"]
            .as_str()
            .unwrap()
            .ends_with("windbreaker.webp")
    );

    // A later conversation: the remembered layout comes from her profile.
    let commerce = Scene::new(false, f6_reads(true));
    let third = turn(
        &rt,
        &mina,
        ChatSurface::Fo,
        say("Light jacket for spring trips", None),
        &commerce,
    )
    .await;
    assert_eq!(
        block(&third, "look_gallery").data["items"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
}

#[tokio::test]
async fn f6_a_guest_gets_the_layout_but_no_memory() {
    let glimmer = Recorded::new(vec![
        (
            "plan",
            json!({"about": "self", "interpretation": [], "reply_plan": "product_list", "steps": [
            {"id": "s1", "tool": "search_products", "args": {"query": "light jacket for spring trips"}, "uses": []}]}),
        ),
        (
            "plan",
            json!({"about": "self", "interpretation": [], "reply_plan": "product_list", "steps": [
            {"id": "s1", "tool": "update_my_preferences", "uses": [], "args": {
                "add": [{"key": "layout_preference", "value": "lists with details", "source": "conversation"}]}}]}),
        ),
    ]);
    let rt = runtime(&glimmer, "2026-11-07T10:00");
    let commerce = Scene::new(true, f6_reads(false));
    let guest = person("ses_mina", Surface::Fo);
    let first = turn(
        &rt,
        &guest,
        ChatSurface::Fo,
        say("Light jacket for spring trips", None),
        &commerce,
    )
    .await;
    assert_eq!(
        block(&first, "look_gallery").data["items"]
            .as_array()
            .unwrap()
            .len(),
        3,
        "a first visit is shown as photos"
    );
    let second = turn(
        &rt,
        &guest,
        ChatSurface::Fo,
        say(
            "Show them as a list with details, and remember that.",
            Some(&conversation(&first)),
        ),
        &commerce,
    )
    .await;
    let list = block(&second, "product_list");
    assert_eq!(list.data["items"].as_array().unwrap().len(), 3);
    assert!(!second.iter().any(|e| e.name() == "memory.saved"));
    assert!(
        reply(&second).contains("could not remember"),
        "{}",
        reply(&second)
    );
}
