//! `sync_claim_status` (`integration.claim.sync_claim_status`): keep a claim's status in step
//! with its outside channel. Inbound, the channel's word becomes a claim status through the
//! channel's mapping and the claim moves only the way fulfillment allows (a return completes only
//! after `refund_return` sent its money). Outbound, the store's status is recorded as the word the
//! channel gets; no channel is connected in this store, so nothing is sent and the answer says
//! so. Either way it records a fact and is not undone.

use acom_contract::{
    ActionType, ApprovalNeed, Audience, Basis, Change, ChangeOp, Committed, ErrorCode, Group, Kind,
    Method, OpMeta, Operation, Planned, Slot, ToolError, UndoSupport, Verification, WriteOp,
};
use acom_core::enums::{ClaimType, SyncDirection};
use acom_fulfillment::service::for_integration as fulfillment;
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::not_mapped;
use crate::channel::registered;
use crate::model::{holds_orders, map_status, transition_check};
use crate::repo::{self, CopyKind};
use crate::rules::MappingRules;
use crate::{
    CHANNEL_NOT_REGISTERED, CLAIM_NOT_MAPPED, INVALID_TRANSITION, STATUS_UNMAPPED, check_id,
    not_connected, required_text,
};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SyncClaimStatusInput {
    /// The marketplace or outside warehouse that holds the request (`list_channels`).
    #[schemars(regex(pattern = r"^chn_[a-z0-9_]+$"), example = &"chn_marketplace_a")]
    pub channel_id: String,
    /// The channel's own request number (`lookup_external_claim`).
    #[schemars(length(min = 1, max = 120), example = &"MK-R-2210")]
    pub external_claim_ref: String,
    /// What the request is. cancel: the order is called off; return: items come back for
    /// money; exchange: items come back for others.
    #[schemars(example = &"return")]
    pub claim_type: ClaimType,
    /// The request's status in the channel's own word; the channel's mapping turns it into a
    /// claim status.
    #[serde(rename = "acom:external_status", alias = "external_status")]
    #[schemars(length(min = 1, max = 80), example = &"return_approved")]
    pub external_status: String,
    /// inbound: the channel's status comes into the store; outbound: the store's status goes to
    /// the channel (recorded; no channel is connected in this store).
    #[serde(rename = "acom:direction", alias = "direction")]
    #[schemars(example = &"inbound")]
    pub direction: SyncDirection,
}

/// What a sync runs, fixed at preview.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SyncClaimPlan {
    pub channel_id: String,
    pub external_claim_ref: String,
    pub claim_id: String,
    pub direction: SyncDirection,
    pub external_status: String,
    /// The claim's status at preview.
    pub from: String,
    /// The status it moves to (inbound); `None` when nothing moves.
    pub to: Option<String>,
}

/// Bring a claim's status in step with its channel.
#[derive(Debug)]
pub struct SyncClaimStatus;

impl Operation for SyncClaimStatus {
    const META: OpMeta = OpMeta {
        address: "integration.claim.sync_claim_status",
        tool: "sync_claim_status",
        group: Group::Operate,
        kind: Kind::Write,
        method: Method::Post,
        path: "/v1/integration/claims/sync-status",
        summary: "Bring a cancel, return or exchange request in step with its outside channel: inbound, the channel's status word moves the store's claim the way fulfillment allows; outbound, the store's status is recorded for the channel. Never refunds; not undone.",
        consumes: &[Slot::one("channel_id"), Slot::one("external_claim_ref")],
        produces: &[Slot::one("claim_id")],
        checks: &[
            "status_mapping.unmapped",
            "status_mapping.transitions_from_core",
        ],
        errors: &[
            STATUS_UNMAPPED,
            INVALID_TRANSITION,
            CLAIM_NOT_MAPPED,
            CHANNEL_NOT_REGISTERED,
        ],
        audience: Audience::Operator,
        action_type: ActionType::UPDATE,
        person_scope: "",
    };
    type Input = SyncClaimStatusInput;
    type Output = Value;
}

fn invalid_transition(
    claim_id: &str,
    from: &str,
    to: &str,
    allowed: &[&str],
    why: &str,
) -> ToolError {
    ToolError::new(
        INVALID_TRANSITION,
        format!("Claim {claim_id} is {}; {why}.", from.replace('_', " ")),
    )
    .detail(
        json!({"entity": "claim", "from": from, "to": to, "allowed_to": allowed,
                   "rule": "status_mapping.transitions_from_core"}),
    )
    .next("lookup_external_claim")
}

/// The preview's work, inside one transaction.
pub(crate) async fn plan(
    tx: &mut TxCx,
    input: SyncClaimStatusInput,
) -> Result<Planned<SyncClaimPlan>, ToolError> {
    check_id("channel_id", "chn", &input.channel_id)?;
    let reference = required_text("external_claim_ref", &input.external_claim_ref)?;
    let external = required_text("acom:external_status", &input.external_status)?;
    let rules = MappingRules::from_policy(&tx.policy)?;
    let channel = registered(tx, &input.channel_id, CHANNEL_NOT_REGISTERED).await?;
    holds_orders(&channel, "list_channels")?;
    let copy = repo::copy(tx, CopyKind::Claim, &channel.id, &reference)
        .await?
        .ok_or_else(|| not_mapped(&channel.id, &reference, "is not a request the channel sent"))?;
    let claim_id = copy
        .mapped_id
        .clone()
        .ok_or_else(|| not_mapped(&channel.id, &reference, "has no claim in the store yet"))?;
    let claim = fulfillment::claim(tx, &claim_id)
        .await?
        .ok_or_else(|| not_mapped(&channel.id, &reference, "maps to a claim that is gone"))?;
    if claim.claim_type != input.claim_type.as_str() {
        return Err(ToolError::invalid_input(format!(
            "{reference} is a {} on the store's side ({}), not a {}.",
            claim.claim_type,
            claim.id,
            input.claim_type.as_str()
        ))
        .detail(json!({"field": "claim_type", "reason": "enum", "allowed": [claim.claim_type]}))
        .next("lookup_external_claim"));
    }
    let (mapped, unmapped_check) = map_status(&rules, &channel, CopyKind::Claim, &external)?;
    let from = claim.claim_status.clone();
    let to = match (input.direction, mapped.as_deref()) {
        (_, None) => None,
        (SyncDirection::Inbound, Some(word)) if word == from => None,
        (SyncDirection::Inbound, Some(word)) => {
            let allowed = fulfillment::claim_moves(&claim.claim_type, &from);
            if allowed.contains(&word) {
                Some(word.to_owned())
            } else {
                return Err(invalid_transition(
                    &claim.id,
                    &from,
                    word,
                    allowed,
                    &format!("fulfillment does not move it to {}", word.replace('_', " ")),
                ));
            }
        }
        (SyncDirection::Outbound, Some(word)) => {
            if word != from {
                return Err(invalid_transition(
                    &claim.id,
                    &from,
                    word,
                    &[],
                    &format!(
                        "`{external}` means {}, so it is not what the store can tell the channel",
                        word.replace('_', " ")
                    ),
                ));
            }
            None
        }
    };
    let transition = transition_check(&rules, "fulfillment", &from, to.as_deref());
    let mut changes = Vec::new();
    if let Some(to) = &to {
        changes.push(Change {
            op: ChangeOp::Update,
            entity: "claim".into(),
            id: claim.id.clone(),
            from: json!(from),
            to: json!(to),
            note: Some(format!(
                "{} {reference} on {}: {} says `{external}`. Refunds go through refund_return.",
                claim.claim_type, channel.id, channel.name
            )),
        });
    }
    if input.direction == SyncDirection::Outbound && copy.external_status != external {
        changes.push(Change {
            op: ChangeOp::Send,
            entity: "external_claim".into(),
            id: reference.clone(),
            from: json!({"acom:external_status": copy.external_status}),
            to: json!({"acom:external_status": external}),
            note: Some(format!(
                "{} is not connected in this store: recorded on the store's copy, not sent",
                channel.id
            )),
        });
    }
    let approval = tx
        .policy
        .approval_for(SyncClaimStatus::META.tool, &json!({}));
    Ok(Planned {
        plan: SyncClaimPlan {
            channel_id: channel.id,
            external_claim_ref: reference,
            claim_id: claim.id.clone(),
            direction: input.direction,
            external_status: external,
            from,
            to,
        },
        changes,
        checks: vec![unmapped_check, transition],
        basis: vec![Basis {
            entity: "fulfillment.claim".into(),
            id: claim.id,
            version: i64::from(claim.version),
        }],
        approval: match approval.reason {
            Some(reason) if approval.needed => ApprovalNeed::Needed { reason },
            _ => ApprovalNeed::NotNeeded,
        },
    })
}

impl WriteOp<Cx> for SyncClaimStatus {
    type Plan = SyncClaimPlan;
    // A status the channel reports already happened outside; it is not taken back.
    const UNDO: UndoSupport = UndoSupport::NotSupported;

    async fn preview(
        &self,
        cx: &Cx,
        input: SyncClaimStatusInput,
    ) -> Result<Planned<SyncClaimPlan>, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let planned = plan(&mut tx, input).await?;
        tx.commit().await.map_err(ToolError::from)?;
        Ok(planned)
    }

    async fn commit(&self, tx: &mut TxCx, plan: &SyncClaimPlan) -> Result<Committed, ToolError> {
        let now = tx.clock.now();
        if let Some(to) = &plan.to {
            let current = fulfillment::claim(tx, &plan.claim_id)
                .await?
                .map(|c| c.claim_status);
            if current.as_deref() != Some(to.as_str()) {
                fulfillment::move_claim(tx, &plan.claim_id, &plan.from, to, Self::META.tool)
                    .await
                    .map_err(|e| {
                        if e.code == ErrorCode::INVALID_STATE.code {
                            ToolError {
                                code: INVALID_TRANSITION.code.into(),
                                next_tool: e
                                    .next_tool
                                    .clone()
                                    .or(Some("lookup_external_claim".into())),
                                ..e
                            }
                        } else {
                            e
                        }
                    })?;
            }
        }
        repo::set_copy_status(
            tx,
            CopyKind::Claim,
            &plan.channel_id,
            &plan.external_claim_ref,
            &plan.external_status,
            now,
        )
        .await?;
        repo::touch_channel(tx, &plan.channel_id, now).await?;
        let mut produced = Map::new();
        produced.insert("claim_id".into(), json!(plan.claim_id));
        if plan.direction == SyncDirection::Outbound {
            produced.insert(
                "acom:outside_system".into(),
                not_connected(
                    &plan.channel_id,
                    "the status was recorded on the store's copy and not sent",
                ),
            );
        }
        Ok(Committed {
            id: plan.claim_id.clone(),
            produced,
        })
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &SyncClaimPlan,
        _done: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let claim = fulfillment::claim(&mut tx, &plan.claim_id).await?;
        let copy = repo::copy(
            &mut tx,
            CopyKind::Claim,
            &plan.channel_id,
            &plan.external_claim_ref,
        )
        .await?;
        tx.commit().await.map_err(ToolError::from)?;
        let mut mismatches = Vec::new();
        let want = plan.to.as_deref().unwrap_or(&plan.from);
        match claim {
            Some(c) if c.claim_status == want => {}
            Some(c) => mismatches.push(format!("{} is {} instead of {want}", c.id, c.claim_status)),
            None => mismatches.push(format!("{} is gone", plan.claim_id)),
        }
        if copy.is_none_or(|c| c.external_status != plan.external_status) {
            mismatches.push(format!(
                "the copy of {} does not say {}",
                plan.external_claim_ref, plan.external_status
            ));
        }
        Ok(Verification {
            matched: mismatches.is_empty(),
            mismatches,
        })
    }

    async fn undo(
        &self,
        _: &mut TxCx,
        plan: &SyncClaimPlan,
        _: &Committed,
    ) -> Result<(), ToolError> {
        Err(ToolError::new(
            ErrorCode::UNDO_WINDOW_CLOSED,
            format!(
                "The status of {} came from its channel; it is not taken back.",
                plan.claim_id
            ),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use acom_contract::schema::{closed_schema_for, violations};

    #[test]
    fn the_input_is_closed() {
        let s = closed_schema_for::<SyncClaimStatusInput>();
        assert!(violations(&s).is_empty(), "{s:#}");
        assert_eq!(
            s["required"],
            json!([
                "acom:direction",
                "acom:external_status",
                "channel_id",
                "claim_type",
                "external_claim_ref"
            ])
        );
    }

    fn sync(status: &str, claim_type: &str) -> SyncClaimStatusInput {
        serde_json::from_value(json!({
            "channel_id": "chn_marketplace_a", "external_claim_ref": "MK-R-T1",
            "claim_type": claim_type, "acom:external_status": status,
            "acom:direction": "inbound"}))
        .unwrap()
    }

    #[tokio::test]
    #[ignore = "integration: needs the demo database (DATABASE_URL)"]
    async fn moves_a_return_the_way_fulfillment_allows() {
        let cx = crate::testing::db("adm_sj").await;
        let mut tx = cx.begin().await.unwrap();
        sqlx::query(
            "INSERT INTO integration.external_claim (tenant_id, channel_id, external_claim_ref,
               external_order_ref, claim_type, claim_id, external_status, ordered_item,
               start_time, last_synced_at)
             VALUES ('store_northwind', 'chn_marketplace_a', 'MK-R-T1', 'MK-T1', 'return',
                     'clm_20333', 'return_requested', '[]', '2026-11-04T09:12:00-05:00',
                     '2026-11-04T09:12:00-05:00')",
        )
        .execute(&mut *tx.tx)
        .await
        .unwrap();
        let e = plan(&mut tx, sync("return_approved", "exchange"))
            .await
            .unwrap_err();
        assert_eq!(e.code, "invalid_input");
        let e = plan(&mut tx, sync("return_completed", "return"))
            .await
            .unwrap_err();
        assert_eq!(e.code, "invalid_transition", "requested cannot complete");
        let p = plan(&mut tx, sync("return_approved", "return"))
            .await
            .unwrap();
        assert_eq!(
            (p.plan.from.as_str(), p.plan.to.as_deref()),
            ("requested", Some("approved"))
        );
        SyncClaimStatus.commit(&mut tx, &p.plan).await.unwrap();
        let c = fulfillment::claim(&mut tx, "clm_20333")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(c.claim_status, "approved");
        let again = plan(&mut tx, sync("return_approved", "return"))
            .await
            .unwrap();
        assert!(again.changes.is_empty());
        drop(tx);
    }
}
