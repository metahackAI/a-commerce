//! `get_policy`, `list_policies`: the rules in effect for this store (defaults merged with the
//! store's overrides).

use acom_contract::{
    ActionType, Audience, ErrorCode, Group, Kind, Method, OpMeta, Operation, ReadOp, ReadResult,
    Slot, ToolError,
};
use acom_store::{Cx, Page};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

/// A policy file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PolicyFile {
    /// When a person must approve.
    Approval,
    /// Products, variants, stock, prices, reviews.
    Catalog,
    /// Boards, posts, replies.
    Community,
    /// Members, tiers, consent, privacy, inquiries, remedies.
    Customer,
    /// Cancellations, returns, exchanges, refunds, delivery.
    Fulfillment,
    /// Outside channels, bulk imports, incoming events.
    Integration,
    /// Display, categories, banners, popups, curations.
    Merchandising,
    /// Customer messages.
    Messaging,
    /// Cart, checkout, orders, payments.
    Order,
    /// Sellers, contracts, grades.
    Partner,
    /// Watchers, roles, admins, API keys.
    Platform,
    /// Promotions, sale prices, coupons, loyalty, events.
    Promotion,
    /// Search, comparison, indexing.
    Search,
}

impl PolicyFile {
    const ALL: [PolicyFile; 13] = [
        Self::Approval,
        Self::Catalog,
        Self::Community,
        Self::Customer,
        Self::Fulfillment,
        Self::Integration,
        Self::Merchandising,
        Self::Messaging,
        Self::Order,
        Self::Partner,
        Self::Platform,
        Self::Promotion,
        Self::Search,
    ];

    fn name(self) -> &'static str {
        match self {
            Self::Approval => "approval",
            Self::Catalog => "catalog",
            Self::Community => "community",
            Self::Customer => "customer",
            Self::Fulfillment => "fulfillment",
            Self::Integration => "integration",
            Self::Merchandising => "merchandising",
            Self::Messaging => "messaging",
            Self::Order => "order",
            Self::Partner => "partner",
            Self::Platform => "platform",
            Self::Promotion => "promotion",
            Self::Search => "search",
        }
    }

    /// The API domain the file's rules belong to (docs/api/README.md §1).
    fn domain(self) -> &'static str {
        match self {
            Self::Approval => "platform",
            Self::Order => "sales",
            other => other.name(),
        }
    }

    fn summary(self) -> &'static str {
        match self {
            Self::Approval => "When a person must approve, undo windows, irreversible changes",
            Self::Catalog => "Price floors, stock thresholds, product approval, reviews",
            Self::Community => "Boards, posts, replies and moderation",
            Self::Customer => "Members, tiers, consent, privacy, inquiries, remedies",
            Self::Fulfillment => "Cancellation, return and exchange windows, refunds, delivery",
            Self::Integration => "Outside channels, bulk imports, incoming events",
            Self::Merchandising => "Display slots, categories, banners, popups, curations",
            Self::Messaging => "Customer message channels, quiet hours, send caps",
            Self::Order => "Cart, checkout, order and payment rules",
            Self::Partner => "Sellers, contracts, grades",
            Self::Platform => "Task watchers, roles, admin accounts, API keys",
            Self::Promotion => "Discount caps, minimum stock, overlap and coupon rules",
            Self::Search => "Search, comparison and indexing",
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetPolicyInput {
    /// A policy key or the start of one, dot-separated, without the file name. Returns every
    /// leaf value under it.
    #[schemars(regex(pattern = r"^[a-z][a-z0-9_]*(\.[a-z0-9_]+)*$"), example = &"max_discount")]
    pub key: String,
    /// Narrows to one file when the same key exists in several.
    #[schemars(example = &"promotion")]
    pub file: Option<PolicyFile>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct PolicyValue {
    /// Full key path inside the file.
    pub key: String,
    /// The value as stored: ratios 0–1, money `{value, currency}`, conditions `{field, op, value}`.
    pub value: Value,
    /// The value as people read it (`30%`, `$200.00`).
    pub display: String,
    /// The policy file the value is in.
    pub file: String,
    /// `true` when the store's own override sets it.
    pub overridden_by_store: bool,
}

pub struct GetPolicy;

impl Operation for GetPolicy {
    const META: OpMeta = OpMeta {
        address: "platform.policy.get",
        tool: "get_policy",
        group: Group::Operate,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/platform/policies",
        summary: "The store's policy values under a key (defaults merged with the store's overrides), with a readable display for each. Use before planning a change; preview checks cite the same keys.",
        consumes: &[Slot::one("policy_key")],
        produces: &[Slot::many("policy_key")],
        checks: &[],
        errors: &[ErrorCode::NOT_FOUND],
        audience: Audience::Operator,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = GetPolicyInput;
    type Output = ReadResult<PolicyValue>;
}

/// Every leaf under `prefix`, as (dotted key, value). Lists are leaves.
fn leaves(v: &Value, prefix: &str, out: &mut Vec<(String, Value)>) {
    match v {
        Value::Object(o) if !is_money(v) && !is_condition(v) => {
            for (k, x) in o {
                let key = if prefix.is_empty() {
                    k.clone()
                } else {
                    format!("{prefix}.{k}")
                };
                leaves(x, &key, out);
            }
        }
        other => out.push((prefix.to_owned(), other.clone())),
    }
}

fn is_money(v: &Value) -> bool {
    v.as_object()
        .is_some_and(|o| o.len() == 2 && o.contains_key("value") && o.contains_key("currency"))
}

fn is_condition(v: &Value) -> bool {
    v.as_object()
        .is_some_and(|o| o.contains_key("field") && o.contains_key("op"))
}

fn money(v: &Value) -> Option<String> {
    let amount = v.get("value")?.to_string();
    let currency = v.get("currency")?.as_str()?;
    let amount = amount
        .parse::<f64>()
        .map(|a| format!("{a:.2}"))
        .unwrap_or(amount);
    Some(match currency {
        "USD" => format!("${amount}"),
        "EUR" => format!("€{amount}"),
        other => format!("{amount} {other}"),
    })
}

/// How a value reads to people. Ratios become percentages when the key names a share.
pub(crate) fn display(key: &str, v: &Value) -> String {
    let ratio_key = ["discount", "rate", "ratio", "percent", "share", "fee"]
        .iter()
        .any(|w| key.contains(w));
    match v {
        Value::Bool(true) => "yes".into(),
        Value::Bool(false) => "no".into(),
        Value::Number(n) => match n.as_f64() {
            Some(f) if ratio_key && (0.0..=1.0).contains(&f) && !n.is_i64() => {
                format!("{}%", (f * 100.0 * 100.0).round() / 100.0)
            }
            _ => n.to_string(),
        },
        Value::String(s) => s.replace('_', " "),
        Value::Array(a) => a
            .iter()
            .map(|x| display(key, x))
            .collect::<Vec<_>>()
            .join(", "),
        Value::Object(_) if is_money(v) => money(v).unwrap_or_else(|| v.to_string()),
        Value::Object(o) if is_condition(v) => format!(
            "{} {} {}",
            o.get("field").and_then(Value::as_str).unwrap_or(""),
            o.get("op").and_then(Value::as_str).unwrap_or(""),
            o.get("value").map(|x| display(key, x)).unwrap_or_default()
        ),
        other => other.to_string(),
    }
}

/// The store's override partials, by file.
async fn overrides(cx: &Cx) -> Result<Map<String, Value>, ToolError> {
    let mut tx = cx.read().await.map_err(ToolError::from)?;
    let rows: Vec<(String, Value)> =
        sqlx::query_as("SELECT file, partial FROM platform.policy_override WHERE tenant_id = $1")
            .bind(cx.tenant.as_str())
            .fetch_all(&mut *tx.tx)
            .await
            .map_err(|e| ToolError::from(acom_store::StoreError::from(e)))?;
    tx.commit().await.map_err(ToolError::from)?;
    Ok(rows.into_iter().collect())
}

fn has_path(v: &Value, key: &str) -> bool {
    let mut cur = v;
    for part in key.split('.') {
        match cur.get(part) {
            Some(next) => cur = next,
            None => return false,
        }
    }
    true
}

impl ReadOp<Cx> for GetPolicy {
    async fn read(
        &self,
        cx: &Cx,
        input: GetPolicyInput,
    ) -> Result<ReadResult<PolicyValue>, ToolError> {
        let ov = overrides(cx).await?;
        let files: Vec<PolicyFile> = match input.file {
            Some(f) => vec![f],
            None => PolicyFile::ALL.to_vec(),
        };
        let mut items = Vec::new();
        for f in files {
            let Some(doc) = cx.policy.raw(f.name()) else {
                continue;
            };
            let mut all = Vec::new();
            leaves(doc, "", &mut all);
            for (key, value) in all {
                let under = key == input.key || key.starts_with(&format!("{}.", input.key));
                if !under || key == "version" {
                    continue;
                }
                let overridden = ov.get(f.name()).is_some_and(|p| has_path(p, &key));
                items.push(PolicyValue {
                    display: display(&key, &value),
                    key,
                    value,
                    file: f.name().to_owned(),
                    overridden_by_store: overridden,
                });
            }
        }
        if items.is_empty() {
            return Err(ToolError::new(
                ErrorCode::NOT_FOUND,
                format!("No policy key starts with `{}`.", input.key),
            )
            .detail(json!({"entity": "policy", "id": input.key}))
            .next("list_policies"));
        }
        Ok(ReadResult::all(items))
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListPoliciesInput {
    /// Only the files of this API domain, for example `promotion` or `sales`.
    #[schemars(example = &"promotion")]
    pub domain: Option<String>,
    /// Page size, 1–100. Default 20.
    #[schemars(range(min = 1, max = 100), example = 20)]
    pub limit: Option<u32>,
    /// `next_cursor` from the previous page.
    #[schemars(example = &"o20")]
    pub cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct PolicyFileInfo {
    /// The file name, without `.yml`.
    pub file: String,
    /// The file's own version number.
    pub version: Option<i64>,
    /// The API domain its rules belong to.
    pub domain: String,
    /// What the file decides.
    pub summary: String,
    /// How many leaf values the store overrides.
    pub overridden_keys: usize,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct PolicyIndex {
    pub items: Vec<PolicyFileInfo>,
    pub total: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
    /// Hash of the rules in effect; every preview records it.
    pub policy_version: String,
}

pub struct ListPolicies;

impl Operation for ListPolicies {
    const META: OpMeta = OpMeta {
        address: "platform.policy.list",
        tool: "list_policies",
        group: Group::Operate,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/platform/policies/index",
        summary: "The loaded policy files (name, version, domain, summary, how many keys the store overrides) and the policy snapshot version. Values come from get_policy.",
        consumes: &[],
        produces: &[Slot::many("policy_key")],
        checks: &[],
        errors: &[],
        audience: Audience::Operator,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = ListPoliciesInput;
    type Output = PolicyIndex;
}

impl ReadOp<Cx> for ListPolicies {
    async fn read(&self, cx: &Cx, input: ListPoliciesInput) -> Result<PolicyIndex, ToolError> {
        let page = Page::new(input.limit, input.cursor.as_deref())?;
        let ov = overrides(cx).await?;
        let all: Vec<PolicyFileInfo> = PolicyFile::ALL
            .iter()
            .filter(|f| input.domain.as_deref().is_none_or(|d| f.domain() == d))
            .map(|f| {
                let overridden_keys = ov.get(f.name()).map_or(0, |p| {
                    let mut l = Vec::new();
                    leaves(p, "", &mut l);
                    l.len()
                });
                PolicyFileInfo {
                    file: f.name().to_owned(),
                    version: cx
                        .policy
                        .raw(f.name())
                        .and_then(|d| d.get("version"))
                        .and_then(Value::as_i64),
                    domain: f.domain().to_owned(),
                    summary: f.summary().to_owned(),
                    overridden_keys,
                }
            })
            .collect();
        let total = all.len();
        let items: Vec<PolicyFileInfo> = all
            .into_iter()
            .skip(usize::try_from(page.offset).unwrap_or(0))
            .take(usize::try_from(page.limit).unwrap_or(20))
            .collect();
        Ok(PolicyIndex {
            next_cursor: page.next_cursor(items.len(), total as i64),
            total: total as u64,
            items,
            policy_version: cx.policy.version().to_owned(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn displays_values_people_can_read() {
        assert_eq!(display("max_discount.by_tier.vip", &json!(0.30)), "30%");
        assert_eq!(
            display(
                "thresholds.refund.when.value",
                &json!({"value": 200.00, "currency": "USD"})
            ),
            "$200.00"
        );
        assert_eq!(display("preview_ttl_minutes", &json!(30)), "30");
        assert_eq!(
            display("on_violation", &json!("use_everyone")),
            "use everyone"
        );
    }

    #[test]
    fn walks_leaves_but_keeps_money_whole() {
        let mut out = Vec::new();
        leaves(
            &json!({"a": {"b": 1, "m": {"value": 2, "currency": "USD"}}}),
            "",
            &mut out,
        );
        let keys: Vec<&str> = out.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(keys, ["a.b", "a.m"]);
    }
}
