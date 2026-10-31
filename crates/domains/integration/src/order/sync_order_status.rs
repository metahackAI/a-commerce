//! `sync_order_status` (`integration.order.sync_order_status`): keep an order's status in step
//! with its outside channel. Inbound, the channel's word becomes an order status through the
//! channel's mapping and the order moves only the way sales allows. Outbound, the store's status
//! is recorded as the word the channel gets; no channel is connected in this store, so nothing is
//! sent and the answer says so. Either way it records a fact and is not undone.

use acom_contract::{
    ActionType, ApprovalNeed, Audience, Basis, Change, ChangeOp, Committed, ErrorCode, Group, Kind,
    Method, OpMeta, Operation, Planned, Slot, ToolError, UndoSupport, Verification, WriteOp,
};
use acom_core::enums::SyncDirection;
use acom_sales::service::for_integration as sales;
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
    CHANNEL_NOT_REGISTERED, INVALID_TRANSITION, ORDER_NOT_MAPPED, STATUS_UNMAPPED, check_id,
    not_connected, required_text,
};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SyncOrderStatusInput {
    /// The marketplace or outside warehouse that holds the order (`list_channels`).
    #[schemars(regex(pattern = r"^chn_[a-z0-9_]+$"), example = &"chn_marketplace_a")]
    pub channel_id: String,
    /// The channel's own order number (`lookup_external_order`).
    #[schemars(length(min = 1, max = 120), example = &"MK-55102")]
    pub external_order_ref: String,
    /// The order's status in the channel's own word; the channel's mapping turns it into an
    /// order status.
    #[serde(rename = "acom:external_status", alias = "external_status")]
    #[schemars(length(min = 1, max = 80), example = &"shipped")]
    pub external_status: String,
    /// inbound: the channel's status comes into the store; outbound: the store's status goes to
    /// the channel (recorded; no channel is connected in this store).
    #[serde(rename = "acom:direction", alias = "direction")]
    #[schemars(example = &"inbound")]
    pub direction: SyncDirection,
}

/// What a sync runs, fixed at preview.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SyncOrderPlan {
    pub channel_id: String,
    pub external_order_ref: String,
    pub order_id: String,
    pub direction: SyncDirection,
    pub external_status: String,
    /// The order's status at preview.
    pub from: String,
    /// The status it moves to (inbound); `None` when nothing moves.
    pub to: Option<String>,
}

/// Bring an order's status in step with its channel.
#[derive(Debug)]
pub struct SyncOrderStatus;

impl Operation for SyncOrderStatus {
    const META: OpMeta = OpMeta {
        address: "integration.order.sync_order_status",
        tool: "sync_order_status",
        group: Group::Operate,
        kind: Kind::Write,
        method: Method::Post,
        path: "/v1/integration/orders/sync-status",
        summary: "Bring an order's status in step with its outside channel: inbound, the channel's status word moves the store's order the way sales allows; outbound, the store's status is recorded for the channel. Records a fact; not undone.",
        consumes: &[Slot::one("channel_id"), Slot::one("external_order_ref")],
        produces: &[Slot::one("order_id")],
        checks: &[
            "status_mapping.unmapped",
            "status_mapping.transitions_from_core",
        ],
        errors: &[
            STATUS_UNMAPPED,
            INVALID_TRANSITION,
            ORDER_NOT_MAPPED,
            CHANNEL_NOT_REGISTERED,
        ],
        audience: Audience::Operator,
        action_type: ActionType::UPDATE,
        person_scope: "",
    };
    type Input = SyncOrderStatusInput;
    type Output = Value;
}

fn invalid_transition(
    order_id: &str,
    from: &str,
    to: &str,
    allowed: &[&str],
    why: &str,
) -> ToolError {
    ToolError::new(
        INVALID_TRANSITION,
        format!("Order {order_id} is {}; {why}.", from.replace('_', " ")),
    )
    .detail(
        json!({"entity": "order", "from": from, "to": to, "allowed_to": allowed,
                   "rule": "status_mapping.transitions_from_core"}),
    )
    .next("lookup_external_order")
}

/// The preview's work, inside one transaction.
pub(crate) async fn plan(
    tx: &mut TxCx,
    input: SyncOrderStatusInput,
) -> Result<Planned<SyncOrderPlan>, ToolError> {
    check_id("channel_id", "chn", &input.channel_id)?;
    let reference = required_text("external_order_ref", &input.external_order_ref)?;
    let external = required_text("acom:external_status", &input.external_status)?;
    let rules = MappingRules::from_policy(&tx.policy)?;
    let channel = registered(tx, &input.channel_id, CHANNEL_NOT_REGISTERED).await?;
    holds_orders(&channel, "list_channels")?;
    let copy = repo::copy(tx, CopyKind::Order, &channel.id, &reference)
        .await?
        .ok_or_else(|| not_mapped(&channel.id, &reference, "is not an order the channel sent"))?;
    let order_id = copy
        .mapped_id
        .clone()
        .ok_or_else(|| not_mapped(&channel.id, &reference, "has no order in the store yet"))?;
    let order = sales::order(tx, &order_id)
        .await?
        .ok_or_else(|| not_mapped(&channel.id, &reference, "maps to an order that is gone"))?;
    let (mapped, unmapped_check) = map_status(&rules, &channel, CopyKind::Order, &external)?;
    let from = order.order_status.clone();
    let to = match (input.direction, mapped.as_deref()) {
        (_, None) => None,
        (SyncDirection::Inbound, Some(word)) if word == from => None,
        (SyncDirection::Inbound, Some(word)) => {
            let allowed = sales::order_moves(&from);
            if allowed.contains(&word) {
                Some(word.to_owned())
            } else {
                return Err(invalid_transition(
                    &order.id,
                    &from,
                    word,
                    allowed,
                    &format!("sales does not move it to {}", word.replace('_', " ")),
                ));
            }
        }
        (SyncDirection::Outbound, Some(word)) => {
            if word != from {
                return Err(invalid_transition(
                    &order.id,
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
    let transition = transition_check(&rules, "sales", &from, to.as_deref());
    let mut changes = Vec::new();
    if let Some(to) = &to {
        changes.push(Change {
            op: ChangeOp::Update,
            entity: "order".into(),
            id: order.id.clone(),
            from: json!(from),
            to: json!(to),
            note: Some(format!(
                "#{} on {}: {} says `{external}`",
                order.order_number, channel.id, channel.name
            )),
        });
    }
    if input.direction == SyncDirection::Outbound && copy.external_status != external {
        changes.push(Change {
            op: ChangeOp::Send,
            entity: "external_order".into(),
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
        .approval_for(SyncOrderStatus::META.tool, &json!({}));
    Ok(Planned {
        plan: SyncOrderPlan {
            channel_id: channel.id,
            external_order_ref: reference,
            order_id: order.id.clone(),
            direction: input.direction,
            external_status: external,
            from,
            to,
        },
        changes,
        checks: vec![unmapped_check, transition],
        basis: vec![Basis {
            entity: "sales.sales_order".into(),
            id: order.id,
            version: i64::from(order.version),
        }],
        approval: match approval.reason {
            Some(reason) if approval.needed => ApprovalNeed::Needed { reason },
            _ => ApprovalNeed::NotNeeded,
        },
    })
}

impl WriteOp<Cx> for SyncOrderStatus {
    type Plan = SyncOrderPlan;
    // A status the channel reports already happened outside; it is not taken back.
    const UNDO: UndoSupport = UndoSupport::NotSupported;

    async fn preview(
        &self,
        cx: &Cx,
        input: SyncOrderStatusInput,
    ) -> Result<Planned<SyncOrderPlan>, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let planned = plan(&mut tx, input).await?;
        tx.commit().await.map_err(ToolError::from)?;
        Ok(planned)
    }

    async fn commit(&self, tx: &mut TxCx, plan: &SyncOrderPlan) -> Result<Committed, ToolError> {
        let now = tx.clock.now();
        if let Some(to) = &plan.to {
            let current = sales::order(tx, &plan.order_id)
                .await?
                .map(|o| o.order_status);
            // Already moved by an earlier run of the same preview: the same result again.
            if current.as_deref() != Some(to.as_str()) {
                sales::move_order(tx, &plan.order_id, &plan.from, to, Self::META.tool)
                    .await
                    .map_err(|e| {
                        if e.code == ErrorCode::INVALID_STATE.code {
                            ToolError {
                                code: INVALID_TRANSITION.code.into(),
                                next_tool: Some("lookup_external_order".into()),
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
            CopyKind::Order,
            &plan.channel_id,
            &plan.external_order_ref,
            &plan.external_status,
            now,
        )
        .await?;
        repo::touch_channel(tx, &plan.channel_id, now).await?;
        let mut produced = Map::new();
        produced.insert("order_id".into(), json!(plan.order_id));
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
            id: plan.order_id.clone(),
            produced,
        })
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &SyncOrderPlan,
        _done: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let order = sales::order(&mut tx, &plan.order_id).await?;
        let copy = repo::copy(
            &mut tx,
            CopyKind::Order,
            &plan.channel_id,
            &plan.external_order_ref,
        )
        .await?;
        tx.commit().await.map_err(ToolError::from)?;
        let mut mismatches = Vec::new();
        let want = plan.to.as_deref().unwrap_or(&plan.from);
        match order {
            Some(o) if o.order_status == want => {}
            Some(o) => mismatches.push(format!("{} is {} instead of {want}", o.id, o.order_status)),
            None => mismatches.push(format!("{} is gone", plan.order_id)),
        }
        if copy.is_none_or(|c| c.external_status != plan.external_status) {
            mismatches.push(format!(
                "the copy of {} does not say {}",
                plan.external_order_ref, plan.external_status
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
        plan: &SyncOrderPlan,
        _: &Committed,
    ) -> Result<(), ToolError> {
        Err(ToolError::new(
            ErrorCode::UNDO_WINDOW_CLOSED,
            format!(
                "The status of {} came from its channel; it is not taken back.",
                plan.order_id
            ),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use acom_contract::schema::{closed_schema_for, violations};

    fn input(v: Value) -> SyncOrderStatusInput {
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn the_input_is_closed_and_needs_a_direction() {
        let s = closed_schema_for::<SyncOrderStatusInput>();
        assert!(violations(&s).is_empty(), "{s:#}");
        assert_eq!(
            s["properties"]["acom:direction"]["enum"],
            json!(["inbound", "outbound"])
        );
        assert!(
            serde_json::from_value::<SyncOrderStatusInput>(json!({
                "channel_id": "chn_marketplace_a", "external_order_ref": "MK-1",
                "acom:external_status": "shipped"}))
            .is_err()
        );
        assert_eq!(SyncOrderStatus::UNDO, UndoSupport::NotSupported);
    }

    /// Maps `MK-T1` on the marketplace to an order, inside the test's transaction.
    async fn fixture(tx: &mut TxCx, order_id: &str, status: &str) {
        sqlx::query(
            "INSERT INTO integration.external_order (tenant_id, channel_id, external_order_ref,
               order_id, external_status, ordered_item, order_date, last_synced_at)
             VALUES ('store_northwind', 'chn_marketplace_a', 'MK-T1', $1, $2, '[]',
                     '2026-11-05T10:00:00-05:00', '2026-11-05T10:00:00-05:00')",
        )
        .bind(order_id)
        .bind(status)
        .execute(&mut *tx.tx)
        .await
        .unwrap();
    }

    fn sync(status: &str, direction: &str) -> SyncOrderStatusInput {
        input(
            json!({"channel_id": "chn_marketplace_a", "external_order_ref": "MK-T1",
                     "acom:external_status": status, "acom:direction": direction}),
        )
    }

    #[tokio::test]
    #[ignore = "integration: needs the demo database (DATABASE_URL)"]
    async fn moves_an_order_the_way_sales_allows_and_only_once() {
        let cx = crate::testing::db("adm_sj").await;
        let mut tx = cx.begin().await.unwrap();
        fixture(&mut tx, "ord_29001", "awaiting_payment").await;
        let p = plan(&mut tx, sync("paid_waiting_shipment", "inbound"))
            .await
            .unwrap();
        assert_eq!(
            (p.plan.from.as_str(), p.plan.to.as_deref()),
            ("payment_due", Some("processing"))
        );
        assert_eq!(p.changes.len(), 1);
        assert!(p.checks.iter().all(|c| c.passed));
        assert_eq!(p.approval, ApprovalNeed::NotNeeded);
        let done = SyncOrderStatus.commit(&mut tx, &p.plan).await.unwrap();
        assert_eq!(done.id, "ord_29001");
        let again = plan(&mut tx, sync("paid_waiting_shipment", "inbound"))
            .await
            .unwrap();
        assert!(again.changes.is_empty() && again.plan.to.is_none());
        let e = plan(&mut tx, sync("teleported", "inbound"))
            .await
            .unwrap_err();
        assert_eq!(
            (e.code.as_ref(), e.next_tool.as_deref()),
            ("status_unmapped", Some("register_channel"))
        );
        let e = plan(&mut tx, sync("delivered", "inbound"))
            .await
            .unwrap_err();
        assert_eq!(
            e.code, "invalid_transition",
            "processing cannot jump to delivered"
        );
        let out = plan(&mut tx, sync("paid_waiting_shipment", "outbound"))
            .await
            .unwrap();
        assert!(out.changes.is_empty(), "the copy already says so");
        let e = plan(&mut tx, sync("shipped", "outbound"))
            .await
            .unwrap_err();
        assert_eq!(e.code, "invalid_transition");
        drop(tx);
    }

    #[tokio::test]
    #[ignore = "integration: needs the demo database (DATABASE_URL)"]
    async fn an_order_the_store_does_not_have_is_not_mapped() {
        let cx = crate::testing::db("adm_sj").await;
        let mut tx = cx.begin().await.unwrap();
        let e = plan(&mut tx, sync("shipped", "inbound")).await.unwrap_err();
        assert_eq!(
            (e.code.as_ref(), e.next_tool.as_deref()),
            ("order_not_mapped", Some("lookup_external_order"))
        );
        drop(tx);
    }
}
