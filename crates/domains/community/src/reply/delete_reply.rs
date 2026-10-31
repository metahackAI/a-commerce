//! `delete_reply` (docs/api/community.md#reply): the writer deletes their reply (`deleted`); the
//! store blinds any reply (`blinded`) with a reason (`moderation.blind`). Nothing is removed, so
//! replies answering it keep their place in the thread. The store's blinding is undone with its
//! change set; a reply already in that state gives the same result.

use acom_contract::{
    ActionType, Audience, Basis, Change, ChangeOp, Check, Committed, ErrorCode, Group, Kind,
    Method, OpMeta, Operation, Planned, Slot, ToolError, UndoSupport, Verification, WriteOp,
};
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::reply_not_found;
use crate::repo;
use crate::rules::BlindRules;
use crate::{NOT_AUTHOR, REASON_REQUIRED, is_operator, not_author, policy_approval, trimmed};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeleteReplyInput {
    /// The reply (`list_replies`).
    #[schemars(regex(pattern = r"^rpl_[a-z0-9_]+$"), example = &"rpl_5521")]
    pub reply_id: String,
    /// Why, for the moderation record. Required when the store blinds a reply.
    #[schemars(length(min = 1, max = 500), example = &"Insults another shopper")]
    pub reason: Option<String>,
}

/// What `delete_reply` runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeleteReplyPlan {
    pub reply_id: String,
    pub from: String,
    pub to: String,
    pub from_reason: Option<String>,
    pub reason: Option<String>,
}

/// The writer deletes a reply, or the store blinds it.
#[derive(Debug)]
pub struct DeleteReply;

impl Operation for DeleteReply {
    const META: OpMeta = OpMeta {
        address: "community.reply.delete_reply",
        tool: "delete_reply",
        group: Group::Engage,
        kind: Kind::Write,
        method: Method::Delete,
        path: "/v1/community/replies/{reply_id}",
        summary: "Delete a reply on a community post: its writer deletes their own; the store blinds any reply with a reason. Replies answering it keep their place.",
        consumes: &[Slot::one("reply_id")],
        produces: &[Slot::one("reply_id")],
        checks: &["moderation.blind"],
        errors: &[NOT_AUTHOR, REASON_REQUIRED, ErrorCode::NOT_FOUND],
        audience: Audience::Both,
        action_type: ActionType::DELETE,
        person_scope: "",
    };
    type Input = DeleteReplyInput;
    type Output = Value;
}

/// The state a delete moves a reply to, and the reason kept with it: the store blinds with a
/// reason, the writer deletes.
pub(crate) fn target(
    operator: bool,
    is_author: bool,
    reason: Option<String>,
    rules: &BlindRules,
    reply_id: &str,
) -> Result<(&'static str, Option<String>), ToolError> {
    if operator {
        if reason.is_none() && rules.operator_reason_required {
            return Err(ToolError::new(
                REASON_REQUIRED,
                "Say why the store blinds this reply; the reason is kept with it.",
            )
            .detail(json!({"field": "reason", "reason": "missing", "rule": "moderation.blind"}))
            .next("delete_reply"));
        }
        return Ok(("blinded", reason));
    }
    if !is_author {
        return Err(not_author("reply", reply_id, "list_replies"));
    }
    Ok(("deleted", None))
}

impl WriteOp<Cx> for DeleteReply {
    type Plan = DeleteReplyPlan;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(
        &self,
        cx: &Cx,
        input: DeleteReplyInput,
    ) -> Result<Planned<DeleteReplyPlan>, ToolError> {
        let rules = BlindRules::from_policy(&cx.policy)?;
        let operator = is_operator(&cx.principal);
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let r = repo::reply(&mut tx, &input.reply_id)
            .await?
            .ok_or_else(|| reply_not_found(&input.reply_id))?;
        tx.commit().await.map_err(ToolError::from)?;
        let (to, reason) = target(
            operator,
            r.author_id == cx.principal.subject,
            trimmed(input.reason.as_deref()),
            &rules,
            &r.id,
        )?;
        let same = r.moderation_status == to;
        Ok(Planned {
            changes: if same {
                Vec::new()
            } else {
                vec![Change {
                    op: ChangeOp::Update,
                    entity: "reply".into(),
                    id: r.id.clone(),
                    from: json!(r.moderation_status),
                    to: json!(to),
                    note: None,
                }]
            },
            checks: vec![Check {
                rule: "moderation.blind".into(),
                passed: true,
                detail: Some(match &reason {
                    _ if same => format!("Already {to}"),
                    Some(why) => format!("Reason recorded: {why}"),
                    None => "Deleted by its writer; replies to it keep their place".into(),
                }),
                fix_tool: None,
            }],
            basis: vec![Basis {
                entity: "community.comment".into(),
                id: r.id.clone(),
                version: i64::from(r.version),
            }],
            approval: policy_approval(
                cx,
                Self::META.tool,
                &json!({"action": to, "by_store": operator}),
            ),
            plan: DeleteReplyPlan {
                reply_id: r.id,
                from: r.moderation_status,
                to: to.into(),
                from_reason: r.moderation_reason,
                reason,
            },
        })
    }

    async fn commit(&self, tx: &mut TxCx, plan: &DeleteReplyPlan) -> Result<Committed, ToolError> {
        if plan.from != plan.to
            && repo::move_reply(
                tx,
                &plan.reply_id,
                &plan.from,
                &plan.to,
                plan.reason.as_deref(),
            )
            .await?
                == 0
        {
            match repo::reply(tx, &plan.reply_id).await? {
                Some(r) if r.moderation_status == plan.to => {}
                Some(r) => {
                    return Err(ToolError::new(
                        ErrorCode::ESTIMATE_CHANGED,
                        format!(
                            "{} is {} now, not {}; preview the delete again.",
                            r.id, r.moderation_status, plan.from
                        ),
                    )
                    .next("delete_reply"));
                }
                None => return Err(reply_not_found(&plan.reply_id)),
            }
        }
        let mut produced = Map::new();
        produced.insert("reply_id".into(), json!(plan.reply_id));
        Ok(Committed {
            id: plan.reply_id.clone(),
            produced,
        })
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &DeleteReplyPlan,
        _done: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let row = repo::reply(&mut tx, &plan.reply_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let matched = row.is_some_and(|r| r.moderation_status == plan.to);
        Ok(Verification {
            matched,
            mismatches: if matched {
                Vec::new()
            } else {
                vec![format!("{} is not {}", plan.reply_id, plan.to)]
            },
        })
    }

    async fn undo(
        &self,
        tx: &mut TxCx,
        plan: &DeleteReplyPlan,
        _done: &Committed,
    ) -> Result<(), ToolError> {
        if plan.from == plan.to {
            return Ok(());
        }
        let n = repo::move_reply(
            tx,
            &plan.reply_id,
            &plan.to,
            &plan.from,
            plan.from_reason.as_deref(),
        )
        .await?;
        if n == 0 {
            return Err(ToolError::new(
                ErrorCode::INVALID_STATE,
                format!(
                    "{} changed after it was {}; it was not put back.",
                    plan.reply_id, plan.to
                ),
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing;

    #[test]
    fn writers_delete_and_the_store_blinds_with_a_reason() {
        let rules = BlindRules::from_policy(&testing::policies()).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            target(false, true, None, &rules, "rpl_5521").ok(),
            Some(("deleted", None))
        );
        let e = target(false, false, None, &rules, "rpl_5521").unwrap_err();
        assert_eq!(
            (e.code.as_ref(), e.next_tool.as_deref()),
            ("not_author", Some("list_replies"))
        );
        let e = target(true, false, None, &rules, "rpl_5521").unwrap_err();
        assert_eq!(e.code, "reason_required");
        assert_eq!(
            target(true, false, Some("Abuse".into()), &rules, "rpl_5521").ok(),
            Some(("blinded", Some("Abuse".into())))
        );
        let schema = acom_contract::schema::closed_schema_for::<DeleteReplyInput>();
        assert!(
            acom_contract::schema::violations(&schema).is_empty(),
            "{schema:#}"
        );
    }

    #[tokio::test]
    #[ignore = "integration: needs a PostgreSQL database"]
    async fn the_store_blinds_a_reply_and_undoes_it() {
        let store = testing::on_db(testing::operator()).await;
        let planned = DeleteReply
            .preview(
                &store,
                DeleteReplyInput {
                    reply_id: "rpl_5510".into(),
                    reason: Some("Checking the undo".into()),
                },
            )
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(planned.changes[0].to, json!("blinded"));
        let mut tx = store.begin().await.unwrap_or_else(|e| panic!("{e}"));
        let done = DeleteReply
            .commit(&mut tx, &planned.plan)
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        DeleteReply
            .undo(&mut tx, &planned.plan, &done)
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        let row = repo::reply(&mut tx, "rpl_5510")
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            row.map(|r| r.moderation_status).as_deref(),
            Some("published")
        );
        tx.tx.rollback().await.unwrap_or_else(|e| panic!("{e}"));
    }
}
