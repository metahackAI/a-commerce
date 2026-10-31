//! `register_channel` (`integration.channel.register_channel`): connect an outside system — a
//! marketplace, a carrier, a payment provider or an outside warehouse. The store keeps where its
//! credential is (a secret-store reference) only as a hash, never returns it, and checks signed
//! events with a signing secret it also keeps only as a hash: given by the operator, or made when
//! the approved set runs and shown that once. A person always approves (W$).
//!
//! No outside system is connected in this store: the connection test checks the address and the
//! credential reference for form only, makes no call, and says so.

use std::collections::BTreeMap;

use acom_contract::{
    ActionType, ApprovalNeed, Audience, Change, ChangeOp, Check, Committed, ErrorCode, Group, Kind,
    Method, OpMeta, Operation, Planned, Slot, ToolError, UndoSupport, Verification, WriteOp,
};
use acom_core::enums::{ChannelType, ClaimStatus, OrderStatus};
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::repo::{self, NewChannel};
use crate::rules::{ChannelRules, MappingRules};
use crate::{
    CONNECTION_FAILED, DUPLICATE_CHANNEL, MAPPING_INCOMPLETE, check_id, new_secret, required_text,
    sha256_hex,
};

/// One status word of a channel's orders and the order status it means.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OrderWord {
    /// The channel's word, exactly as it sends it.
    #[schemars(length(min = 1, max = 80), example = &"shipped")]
    pub external_status: String,
    /// The order status it means (enums.md §1.1).
    #[schemars(example = &"in_transit")]
    pub order_status: OrderStatus,
}

/// One status word of a channel's cancel, return and exchange requests and the claim status it
/// means.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ClaimWord {
    /// The channel's word, exactly as it sends it.
    #[schemars(length(min = 1, max = 80), example = &"return_requested")]
    pub external_status: String,
    /// The claim status it means (enums.md §1.5).
    #[schemars(example = &"requested")]
    pub claim_status: ClaimStatus,
}

/// The channel's status words mapped to ours, one list per part.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StatusMappingInput {
    /// Order status words, for example shipped → in_transit.
    #[schemars(example = json!([{"external_status": "paid_waiting_shipment", "order_status": "processing"}, {"external_status": "shipped", "order_status": "in_transit"}]))]
    pub order: Option<Vec<OrderWord>>,
    /// Claim status words, for example return_requested → requested.
    #[schemars(example = json!([{"external_status": "return_requested", "claim_status": "requested"}]))]
    pub claim: Option<Vec<ClaimWord>>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RegisterChannelInput {
    /// The name people use for the channel.
    #[schemars(length(min = 1, max = 120), example = &"UPS")]
    pub name: String,
    /// What the channel is. marketplace: sells the store's products; carrier: carries parcels;
    /// payment: takes payments and refunds; fulfillment: an outside warehouse.
    #[serde(rename = "acom:channel_type", alias = "channel_type")]
    #[schemars(example = &"carrier")]
    pub channel_type: ChannelType,
    /// The address of the outside system's API (https).
    #[schemars(length(min = 1, max = 500), example = &"https://api.ups.example/v1")]
    pub url: String,
    /// Where the credential the store calls the channel with is kept: a reference into the
    /// secret store, never the secret itself. Kept only as a hash and never shown again.
    #[serde(rename = "acom:credential_ref", alias = "credential_ref")]
    #[schemars(length(min = 1, max = 500), example = &"vault://integration/ups/api-key", extend("writeOnly" = true))]
    pub credential_ref: String,
    /// The channel's status words mapped to ours. A marketplace or an outside warehouse maps
    /// every part `status_mapping.must_cover` names (orders and claims); carriers and payment
    /// providers send fixed event words and need none.
    #[serde(rename = "acom:status_mapping", alias = "status_mapping")]
    pub status_mapping: StatusMappingInput,
    /// The secret the channel signs its events with, when it gives one. Kept only as a hash;
    /// left out, the store makes one when the change set runs and shows it that once.
    #[serde(rename = "acom:signing_secret_ref", alias = "signing_secret_ref")]
    #[schemars(length(min = 16, max = 500), example = &"whsec_9c1e7a…", extend("writeOnly" = true))]
    pub signing_secret_ref: Option<String>,
    /// The marketplace seller the channel belongs to; left out for the store's own channels.
    #[schemars(regex(pattern = r"^slr_[a-z0-9_]+$"), example = &"slr_harbor_goods")]
    pub seller_id: Option<String>,
}

/// What `register_channel` runs: the channel and its secrets as hashes only.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RegisterChannelPlan {
    pub channel_id: String,
    pub name: String,
    pub channel_type: ChannelType,
    pub url: String,
    pub seller_id: Option<String>,
    pub credential_hash: String,
    /// The hash of the signing secret the operator gave; `None` when the run makes one.
    pub signing_secret_hash: Option<String>,
    /// `{order: {word: order_status}, claim: {word: claim_status}}`.
    pub status_mapping: Value,
}

/// Register an outside channel.
#[derive(Debug)]
pub struct RegisterChannel;

impl Operation for RegisterChannel {
    const META: OpMeta = OpMeta {
        address: "integration.channel.register_channel",
        tool: "register_channel",
        group: Group::Operate,
        kind: Kind::WriteApproval,
        method: Method::Post,
        path: "/v1/integration/channels",
        summary: "Connect an outside channel (marketplace, carrier, payment provider, outside warehouse): its API address, where its credential is kept, how its status words map to ours, and the secret its events are signed with. Secrets are kept only as hashes; a person always approves.",
        consumes: &[],
        produces: &[Slot::one("channel_id")],
        checks: &[
            "channel.connection_test_required",
            "status_mapping.must_cover",
        ],
        errors: &[CONNECTION_FAILED, MAPPING_INCOMPLETE, DUPLICATE_CHANNEL],
        audience: Audience::Operator,
        action_type: ActionType::REGISTER,
        person_scope: "",
    };
    type Input = RegisterChannelInput;
    type Output = Value;
}

/// `UPS` → `ups`, `Marketplace A` → `marketplace_a`.
fn slug(text: &str) -> String {
    let mut out = String::new();
    for ch in text.trim().chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if !out.ends_with('_') && !out.is_empty() {
            out.push('_');
        }
    }
    let out = out.trim_end_matches('_').to_owned();
    if out.is_empty() { "x".into() } else { out }
}

/// The mapping as the channel row keeps it, refusing a word given twice.
fn mapping(input: &StatusMappingInput) -> Result<Value, ToolError> {
    fn part<'a>(
        name: &str,
        words: impl Iterator<Item = (&'a str, &'static str)>,
    ) -> Result<Map<String, Value>, ToolError> {
        let mut out = Map::new();
        for (word, internal) in words {
            let word = word.trim();
            if word.is_empty() {
                return Err(ToolError::invalid_input(format!(
                    "A {name} status word is empty."
                ))
                .detail(json!({"field": format!("acom:status_mapping.{name}"), "reason": "missing"})));
            }
            if out.insert(word.to_owned(), json!(internal)).is_some() {
                return Err(ToolError::invalid_input(format!(
                    "The {name} word `{word}` is mapped twice."
                ))
                .detail(
                    json!({"field": format!("acom:status_mapping.{name}"), "reason": "format"}),
                ));
            }
        }
        Ok(out)
    }
    let order = part(
        "order",
        input
            .order
            .iter()
            .flatten()
            .map(|w| (w.external_status.as_str(), w.order_status.as_str())),
    )?;
    let claim = part(
        "claim",
        input
            .claim
            .iter()
            .flatten()
            .map(|w| (w.external_status.as_str(), w.claim_status.as_str())),
    )?;
    Ok(json!({"order": order, "claim": claim}))
}

/// `status_mapping.must_cover` for a channel that carries orders and claims; carriers and
/// payment providers report fixed event words.
fn coverage(rules: &MappingRules, kind: ChannelType, map: &Value) -> Result<Check, ToolError> {
    let carries = matches!(kind, ChannelType::Marketplace | ChannelType::Fulfillment);
    if !carries {
        return Ok(Check {
            rule: "status_mapping.must_cover".into(),
            passed: true,
            detail: Some(format!(
                "A {} sends fixed event words; no status mapping is needed",
                kind.as_str()
            )),
            fix_tool: None,
        });
    }
    let count = |part: &str| map.get(part).and_then(Value::as_object).map_or(0, Map::len);
    let missing: Vec<&str> = rules
        .must_cover
        .iter()
        .map(String::as_str)
        .filter(|p| count(p) == 0)
        .collect();
    if !missing.is_empty() {
        return Err(ToolError::new(
            MAPPING_INCOMPLETE,
            format!(
                "A {} must map its {} status words; none are given.",
                kind.as_str(),
                missing.join(" and ")
            ),
        )
        .detail(
            json!({"rule": "status_mapping.must_cover", "missing": missing,
                       "field": "acom:status_mapping"}),
        )
        .next("register_channel"));
    }
    let parts: Vec<String> = rules
        .must_cover
        .iter()
        .map(|p| format!("{p}: {} words", count(p)))
        .collect();
    Ok(Check {
        rule: "status_mapping.must_cover".into(),
        passed: true,
        detail: Some(format!(
            "Order and claim states mapped ({})",
            parts.join(", ")
        )),
        fix_tool: None,
    })
}

/// The connection test. No outside system is connected in this store, so it checks the form of
/// the address and the credential reference and makes no call.
fn connection_test(
    rules: &ChannelRules,
    url: &str,
    credential_ref: &str,
) -> Result<Check, ToolError> {
    let host = url
        .strip_prefix("https://")
        .map(|r| r.split(['/', '?', '#']).next().unwrap_or(""))
        .unwrap_or("");
    let host_ok = host.contains('.') && !host.contains(char::is_whitespace);
    let credential_ok =
        !credential_ref.trim().is_empty() && !credential_ref.contains(char::is_whitespace);
    if !(host_ok && credential_ok) {
        let why = if host_ok {
            "the credential reference has spaces in it"
        } else {
            "the address is not an https address with a host"
        };
        if rules.connection_test_required {
            return Err(ToolError::new(
                CONNECTION_FAILED,
                format!("The connection test failed: {why}."),
            )
            .detail(
                json!({"rule": "channel.connection_test_required", "upstream": url,
                           "retryable": false}),
            )
            .next("register_channel"));
        }
        return Ok(Check {
            rule: "channel.connection_test_required".into(),
            passed: false,
            detail: Some(format!("The connection test failed: {why}")),
            fix_tool: Some("register_channel".into()),
        });
    }
    Ok(Check {
        rule: "channel.connection_test_required".into(),
        passed: true,
        detail: Some(format!(
            "No outside system is connected in this store: {host} and the credential reference were checked for form only, no call was made"
        )),
        fix_tool: None,
    })
}

impl WriteOp<Cx> for RegisterChannel {
    type Plan = RegisterChannelPlan;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(
        &self,
        cx: &Cx,
        input: RegisterChannelInput,
    ) -> Result<Planned<RegisterChannelPlan>, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let planned = plan(&mut tx, input).await?;
        tx.commit().await.map_err(ToolError::from)?;
        Ok(planned)
    }

    async fn commit(
        &self,
        tx: &mut TxCx,
        plan: &RegisterChannelPlan,
    ) -> Result<Committed, ToolError> {
        let mut produced = Map::new();
        produced.insert("channel_id".into(), json!(plan.channel_id));
        if repo::channel(tx, &plan.channel_id).await?.is_some() {
            // Ran before: a secret made then was shown then and is not shown again.
            return Ok(Committed {
                id: plan.channel_id.clone(),
                produced,
            });
        }
        if let Some(existing) = repo::channel_named(tx, &plan.name, &plan.url).await? {
            return Err(duplicate(&plan.name, &existing));
        }
        let (signing_hash, made) = match &plan.signing_secret_hash {
            Some(h) => (h.clone(), None),
            None => {
                let secret = new_secret("whsec_");
                (sha256_hex(&secret), Some(secret))
            }
        };
        repo::insert_channel(
            tx,
            &NewChannel {
                id: &plan.channel_id,
                name: &plan.name,
                channel_type: plan.channel_type.as_str(),
                url: &plan.url,
                seller_id: plan.seller_id.as_deref(),
                credential_hash: &plan.credential_hash,
                signing_secret_hash: Some(&signing_hash),
                status_mapping: &plan.status_mapping,
            },
        )
        .await?;
        if let Some(secret) = made {
            produced.insert(
                acom_changeset::SHOWN_ONCE.into(),
                json!({"signing_secret": secret,
                       "note": "Give this secret to the channel to sign its events with. It is shown this once and kept only as a hash."}),
            );
        }
        Ok(Committed {
            id: plan.channel_id.clone(),
            produced,
        })
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &RegisterChannelPlan,
        _done: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let c = repo::channel(&mut tx, &plan.channel_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let mismatches = match c {
            None => vec![format!("{} was not created", plan.channel_id)],
            Some(c) => {
                let mut m = Vec::new();
                if c.name != plan.name || c.url != plan.url {
                    m.push(format!("{} is not named or addressed as previewed", c.id));
                }
                if c.channel_type != plan.channel_type.as_str() {
                    m.push(format!("{} is a {}", c.id, c.channel_type));
                }
                if c.status_mapping != plan.status_mapping {
                    m.push(format!("{} maps its status words differently", c.id));
                }
                if c.signing_secret_hash.is_none() {
                    m.push(format!("{} has no signing secret", c.id));
                }
                m
            }
        };
        Ok(Verification {
            matched: mismatches.is_empty(),
            mismatches,
        })
    }

    /// Undoing takes the channel out again while nothing came from it yet.
    async fn undo(
        &self,
        tx: &mut TxCx,
        plan: &RegisterChannelPlan,
        _done: &Committed,
    ) -> Result<(), ToolError> {
        if repo::channel_in_use(tx, &plan.channel_id).await? {
            return Err(ToolError::new(
                ErrorCode::INVALID_STATE,
                format!(
                    "{} already holds orders, claims or events; it stays registered.",
                    plan.channel_id
                ),
            )
            .detail(json!({"entity": "channel", "id": plan.channel_id}))
            .next("list_channels"));
        }
        repo::delete_channel(tx, &plan.channel_id).await
    }
}

fn duplicate(name: &str, existing: &str) -> ToolError {
    ToolError::new(
        DUPLICATE_CHANNEL,
        format!("{existing} is already registered as {name} at the same address."),
    )
    .detail(json!({"entity": "channel", "field": "name", "existing_id": existing}))
    .next("list_channels")
}

/// The preview's work, inside one transaction (a read-only one for a preview).
pub(crate) async fn plan(
    tx: &mut TxCx,
    input: RegisterChannelInput,
) -> Result<Planned<RegisterChannelPlan>, ToolError> {
    let channel_rules = ChannelRules::from_policy(&tx.policy)?;
    let mapping_rules = MappingRules::from_policy(&tx.policy)?;
    let name = required_text("name", &input.name)?;
    let url = required_text("url", &input.url)?;
    let credential = required_text("acom:credential_ref", &input.credential_ref)?;
    if let Some(s) = &input.seller_id {
        check_id("seller_id", "slr", s)?;
    }
    let signing = match input.signing_secret_ref.as_deref().map(str::trim) {
        Some(s) if s.len() < 16 => {
            return Err(ToolError::invalid_input(
                "`acom:signing_secret_ref` is too short to sign events with (16 characters at least).",
            )
            .detail(json!({"field": "acom:signing_secret_ref", "reason": "range"})));
        }
        Some(s) => Some(s.to_owned()),
        None => None,
    };
    let test = connection_test(&channel_rules, &url, &credential)?;
    let map = mapping(&input.status_mapping)?;
    let cover = coverage(&mapping_rules, input.channel_type, &map)?;
    if let Some(existing) = repo::channel_named(tx, &name, &url).await? {
        return Err(duplicate(&name, &existing));
    }
    let base = format!("chn_{}_{}", input.channel_type.as_str(), slug(&name));
    let channel_id = if repo::channel(tx, &base).await?.is_some() {
        format!("{base}_{}", acom_core::id::new_suffix())
    } else {
        base
    };
    let words: BTreeMap<&str, usize> = ["order", "claim"]
        .into_iter()
        .map(|p| (p, map[p].as_object().map_or(0, Map::len)))
        .collect();
    let plan = RegisterChannelPlan {
        channel_id,
        name,
        channel_type: input.channel_type,
        url,
        seller_id: input.seller_id,
        // Secrets are hashed here: neither the preview store nor the plan ever holds them.
        credential_hash: sha256_hex(&credential),
        signing_secret_hash: signing.as_deref().map(sha256_hex),
        status_mapping: map,
    };
    let signing_text = if plan.signing_secret_hash.is_some() {
        "given; kept only as a hash"
    } else {
        "made when this runs, shown once, kept only as a hash"
    };
    let vaulting = if channel_rules.vaulting_required {
        "a secret-store reference, kept only as a hash"
    } else {
        "kept only as a hash"
    };
    Ok(Planned {
        changes: vec![Change {
            op: ChangeOp::Create,
            entity: "channel".into(),
            id: String::new(),
            from: Value::Null,
            to: json!({
                "name": plan.name, "acom:channel_type": plan.channel_type, "url": plan.url,
                "seller_id": plan.seller_id, "acom:connection_status": "connected",
                "acom:status_mapping": plan.status_mapping,
                "acom:credential": vaulting, "acom:signing_secret": signing_text,
            }),
            note: Some(format!(
                "Maps {} order and {} claim status words. Stores credentials for an outside system.",
                words["order"], words["claim"]
            )),
        }],
        checks: vec![test, cover],
        basis: Vec::new(),
        approval: ApprovalNeed::Needed {
            reason: "Stores credentials for an outside system".into(),
        },
        plan,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use acom_contract::schema::{closed_schema_for, violations};

    fn input(v: Value) -> RegisterChannelInput {
        serde_json::from_value(v).unwrap()
    }

    fn ups() -> Value {
        json!({"name": "UPS", "acom:channel_type": "carrier", "url": "https://api.ups.example/v1",
               "acom:credential_ref": "vault://integration/ups/api-key", "acom:status_mapping": {}})
    }

    #[test]
    fn secrets_are_write_only_and_the_input_is_closed() {
        let s = closed_schema_for::<RegisterChannelInput>();
        assert!(violations(&s).is_empty(), "{s:#}");
        assert_eq!(s["properties"]["acom:credential_ref"]["writeOnly"], true);
        assert_eq!(
            s["properties"]["acom:signing_secret_ref"]["writeOnly"],
            true
        );
        let stored = acom_changeset::storable_input(&s, &ups());
        assert_eq!(stored["acom:credential_ref"], acom_changeset::NOT_STORED);
        assert_eq!(
            s["properties"]["acom:status_mapping"]["properties"]["order"]["items"]["properties"]["order_status"]
                ["enum"][1],
            "processing"
        );
    }

    #[test]
    fn marketplaces_map_orders_and_claims_but_carriers_need_not() {
        let rules = MappingRules::from_policy(&crate::testing::policies()).unwrap();
        let empty = mapping(&StatusMappingInput::default()).unwrap();
        assert!(
            coverage(&rules, ChannelType::Carrier, &empty)
                .unwrap()
                .passed
        );
        let e = coverage(&rules, ChannelType::Marketplace, &empty).unwrap_err();
        assert_eq!(
            (e.code.as_ref(), e.next_tool.as_deref()),
            ("mapping_incomplete", Some("register_channel"))
        );
        let full: StatusMappingInput = serde_json::from_value(json!({
            "order": [{"external_status": "shipped", "order_status": "in_transit"}],
            "claim": [{"external_status": "return_requested", "claim_status": "requested"}]}))
        .unwrap();
        let map = mapping(&full).unwrap();
        assert_eq!(map["order"]["shipped"], "in_transit");
        assert!(
            coverage(&rules, ChannelType::Marketplace, &map)
                .unwrap()
                .passed
        );
        let twice: StatusMappingInput = serde_json::from_value(json!({"order": [
            {"external_status": "shipped", "order_status": "in_transit"},
            {"external_status": "shipped", "order_status": "delivered"}]}))
        .unwrap();
        assert_eq!(mapping(&twice).unwrap_err().code, "invalid_input");
    }

    #[test]
    fn the_connection_test_checks_form_and_says_no_call_was_made() {
        let rules = ChannelRules::from_policy(&crate::testing::policies()).unwrap();
        let ok = connection_test(&rules, "https://api.ups.example/v1", "vault://x").unwrap();
        assert!(ok.passed && ok.detail.unwrap().contains("no call was made"));
        let e = connection_test(&rules, "http://api.ups.example", "vault://x").unwrap_err();
        assert_eq!(e.code, "connection_failed");
        assert_eq!(slug("Marketplace A"), "marketplace_a");
    }

    #[tokio::test]
    #[ignore = "integration: needs the demo database (DATABASE_URL)"]
    async fn registers_with_hashes_only_shows_the_secret_once_and_undoes() {
        let cx = crate::testing::db("adm_sj").await;
        let mut tx = cx.begin().await.unwrap();
        let p = plan(&mut tx, input(ups())).await.unwrap_err();
        assert_eq!(p.code, "duplicate_channel", "{p:?}");
        let mut v = ups();
        v["name"] = json!("Parcel Express");
        v["url"] = json!("https://api.parcel-express.example/v2");
        let planned = plan(&mut tx, input(v)).await.unwrap();
        assert_eq!(planned.plan.channel_id, "chn_carrier_parcel_express");
        assert!(matches!(planned.approval, ApprovalNeed::Needed { .. }));
        let stored = serde_json::to_string(&planned.plan).unwrap();
        assert!(!stored.contains("vault://"), "{stored}");
        let done = RegisterChannel
            .commit(&mut tx, &planned.plan)
            .await
            .unwrap();
        let secret = done.produced["shown_once"]["signing_secret"]
            .as_str()
            .unwrap()
            .to_owned();
        let row = repo::channel(&mut tx, "chn_carrier_parcel_express")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            row.signing_secret_hash.as_deref(),
            Some(sha256_hex(&secret).as_str())
        );
        let again = RegisterChannel
            .commit(&mut tx, &planned.plan)
            .await
            .unwrap();
        assert!(!again.produced.contains_key("shown_once"));
        RegisterChannel
            .undo(&mut tx, &planned.plan, &done)
            .await
            .unwrap();
        assert!(
            repo::channel(&mut tx, "chn_carrier_parcel_express")
                .await
                .unwrap()
                .is_none()
        );
        drop(tx);
    }
}
