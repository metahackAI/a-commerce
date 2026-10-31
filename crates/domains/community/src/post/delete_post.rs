//! `delete_post` (docs/api/community.md#post): the writer deletes their post (`deleted`); the store
//! blinds it (`blinded`) or deletes it, always with a reason (`moderation.blind`). Nothing is
//! removed: a deleted post that others replied to keeps its place with its text hidden
//! (`moderation.blind.keep_replies_on_delete`). The store's decision is undone with its change
//! set; asking for the state a post is already in succeeds without a change.

use acom_contract::{
    ActionType, Audience, Basis, Change, ChangeOp, Check, Committed, ErrorCode, Group, Kind,
    Method, OpMeta, Operation, Planned, Slot, ToolError, UndoSupport, Verification, WriteOp,
};
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::repo;
use crate::rules::BlindRules;
use crate::{
    NOT_AUTHOR, POST_NOT_FOUND, REASON_REQUIRED, is_operator, not_author, policy_approval,
    post_not_found, trimmed,
};

/// What to do with the post.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DeleteAction {
    /// Delete it (the writer, or the store).
    #[default]
    Delete,
    /// Hide it from shoppers as a moderation decision (the store only).
    Blind,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeletePostInput {
    /// The post (`list_posts`).
    #[schemars(regex(pattern = r"^post_[a-z0-9_]+$"), example = &"post_8811")]
    pub post_id: String,
    /// delete: the writer or the store deletes it; blind: the store hides it from shoppers.
    /// Default delete.
    #[schemars(example = &"blind")]
    pub action: Option<DeleteAction>,
    /// Why, for the moderation record. Required when the store blinds or deletes.
    #[schemars(length(min = 1, max = 500), example = &"Reported 5 times for spam")]
    pub reason: Option<String>,
}

/// What `delete_post` runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeletePostPlan {
    pub post_id: String,
    pub from: String,
    pub to: String,
    /// The reason kept before, restored on undo.
    pub from_reason: Option<String>,
    /// The store's reason; `None` when the writer deletes.
    pub reason: Option<String>,
}

/// The writer deletes a post, or the store blinds or deletes it.
#[derive(Debug)]
pub struct DeletePost;

impl Operation for DeletePost {
    const META: OpMeta = OpMeta {
        address: "community.post.delete_post",
        tool: "delete_post",
        group: Group::Engage,
        kind: Kind::Write,
        method: Method::Delete,
        path: "/v1/community/posts/{post_id}",
        summary: "Delete a community post: its writer deletes their own; the store blinds or deletes any post with a reason. Others' replies keep the post's place with its text hidden.",
        consumes: &[Slot::one("post_id")],
        produces: &[Slot::one("post_id")],
        checks: &["moderation.blind"],
        errors: &[NOT_AUTHOR, REASON_REQUIRED, POST_NOT_FOUND],
        audience: Audience::Both,
        action_type: ActionType::DELETE,
        person_scope: "",
    };
    type Input = DeletePostInput;
    type Output = Value;
}

/// The state a delete moves a post to, and the reason kept with it.
pub(crate) fn target(
    operator: bool,
    is_author: bool,
    action: DeleteAction,
    reason: Option<String>,
    rules: &BlindRules,
    post_id: &str,
) -> Result<(&'static str, Option<String>), ToolError> {
    if !operator {
        if action == DeleteAction::Blind {
            return Err(ToolError::new(
                ErrorCode::FORBIDDEN,
                "Only the store can blind a post; its writer can delete it.",
            )
            .detail(json!({"reason": "role", "field": "action"})));
        }
        if !is_author {
            return Err(not_author("post", post_id, "get_post"));
        }
        return Ok(("deleted", None));
    }
    if reason.is_none() && rules.operator_reason_required {
        return Err(ToolError::new(
            REASON_REQUIRED,
            "Say why the store blinds or deletes this post; the reason is kept with it.",
        )
        .detail(json!({"field": "reason", "reason": "missing", "rule": "moderation.blind"}))
        .next("delete_post"));
    }
    Ok((
        match action {
            DeleteAction::Blind => "blinded",
            DeleteAction::Delete => "deleted",
        },
        reason,
    ))
}

impl WriteOp<Cx> for DeletePost {
    type Plan = DeletePostPlan;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(
        &self,
        cx: &Cx,
        input: DeletePostInput,
    ) -> Result<Planned<DeletePostPlan>, ToolError> {
        let rules = BlindRules::from_policy(&cx.policy)?;
        let operator = is_operator(&cx.principal);
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let p = repo::post(&mut tx, &input.post_id)
            .await?
            .ok_or_else(|| post_not_found(&input.post_id))?;
        let is_author = p.author_id == cx.principal.subject;
        if !operator && !is_author && p.is_secret {
            return Err(post_not_found(&p.id));
        }
        let (to, reason) = target(
            operator,
            is_author,
            input.action.unwrap_or_default(),
            trimmed(input.reason.as_deref()),
            &rules,
            &p.id,
        )?;
        let others = repo::others_replied(&mut tx, &p.id, &p.author_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let kept = (to == "deleted" && others > 0 && rules.keep_replies_on_delete).then(|| {
            format!(
                "{others} {} by others stay; the post keeps its place with its text hidden",
                if others == 1 { "reply" } else { "replies" }
            )
        });
        let detail = match (&reason, &kept) {
            _ if p.moderation_status == to => format!("Already {to}"),
            (Some(r), Some(k)) => format!("Reason recorded: {r}; {k}"),
            (Some(r), None) => format!("Reason recorded: {r}"),
            (None, Some(k)) => format!("Deleted by its writer; {k}"),
            (None, None) => "Deleted by its writer".into(),
        };
        let changes = if p.moderation_status == to {
            Vec::new()
        } else {
            vec![Change {
                op: ChangeOp::Update,
                entity: "post".into(),
                id: p.id.clone(),
                from: json!(p.moderation_status),
                to: json!(to),
                note: kept,
            }]
        };
        Ok(Planned {
            changes,
            checks: vec![Check {
                rule: "moderation.blind".into(),
                passed: true,
                detail: Some(detail),
                fix_tool: None,
            }],
            basis: vec![Basis {
                entity: "community.discussion_forum_posting".into(),
                id: p.id.clone(),
                version: i64::from(p.version),
            }],
            approval: policy_approval(
                cx,
                Self::META.tool,
                &json!({"action": to, "by_store": operator}),
            ),
            plan: DeletePostPlan {
                post_id: p.id,
                from: p.moderation_status,
                to: to.into(),
                from_reason: p.moderation_reason,
                reason,
            },
        })
    }

    async fn commit(&self, tx: &mut TxCx, plan: &DeletePostPlan) -> Result<Committed, ToolError> {
        if plan.from != plan.to
            && repo::move_post(
                tx,
                &plan.post_id,
                &plan.from,
                &plan.to,
                plan.reason.as_deref(),
            )
            .await?
                == 0
        {
            let now = repo::post(tx, &plan.post_id).await?;
            match now {
                Some(p) if p.moderation_status == plan.to => {}
                Some(p) => {
                    return Err(ToolError::new(
                        ErrorCode::ESTIMATE_CHANGED,
                        format!(
                            "{} is {} now, not {}; preview the delete again.",
                            p.id, p.moderation_status, plan.from
                        ),
                    )
                    .next("delete_post"));
                }
                None => return Err(post_not_found(&plan.post_id)),
            }
        }
        let mut produced = Map::new();
        produced.insert("post_id".into(), json!(plan.post_id));
        Ok(Committed {
            id: plan.post_id.clone(),
            produced,
        })
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &DeletePostPlan,
        _done: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let row = repo::post(&mut tx, &plan.post_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let matched = row.is_some_and(|p| p.moderation_status == plan.to);
        Ok(Verification {
            matched,
            mismatches: if matched {
                Vec::new()
            } else {
                vec![format!("{} is not {}", plan.post_id, plan.to)]
            },
        })
    }

    async fn undo(
        &self,
        tx: &mut TxCx,
        plan: &DeletePostPlan,
        _done: &Committed,
    ) -> Result<(), ToolError> {
        if plan.from == plan.to {
            return Ok(());
        }
        let n = repo::move_post(
            tx,
            &plan.post_id,
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
                    plan.post_id, plan.to
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
        let t = |op, author, action, reason: Option<&str>| {
            target(
                op,
                author,
                action,
                reason.map(str::to_owned),
                &rules,
                "post_8812",
            )
        };
        assert_eq!(
            t(false, true, DeleteAction::Delete, None).ok(),
            Some(("deleted", None))
        );
        let e = t(false, false, DeleteAction::Delete, None).unwrap_err();
        assert_eq!(
            (e.code.as_ref(), e.next_tool.as_deref()),
            ("not_author", Some("get_post"))
        );
        assert_eq!(
            t(false, true, DeleteAction::Blind, None).unwrap_err().code,
            "forbidden"
        );
        let e = t(true, false, DeleteAction::Blind, None).unwrap_err();
        assert_eq!(
            (e.code.as_ref(), e.next_tool.as_deref()),
            ("reason_required", Some("delete_post"))
        );
        assert_eq!(
            t(true, false, DeleteAction::Blind, Some("Spam")).ok(),
            Some(("blinded", Some("Spam".into())))
        );
        let schema = acom_contract::schema::closed_schema_for::<DeletePostInput>();
        assert!(
            acom_contract::schema::violations(&schema).is_empty(),
            "{schema:#}"
        );
    }

    #[tokio::test]
    #[ignore = "integration: needs a PostgreSQL database"]
    async fn the_store_blinds_a_reported_post_and_undoes_it() {
        let store = testing::on_db(testing::operator()).await;
        let planned = DeletePost
            .preview(
                &store,
                DeletePostInput {
                    post_id: "post_8811".into(),
                    action: Some(DeleteAction::Blind),
                    reason: Some("Advertises another shop".into()),
                },
            )
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(planned.changes[0].from, json!("published"));
        assert_eq!(planned.changes[0].to, json!("blinded"));
        assert_eq!(
            planned.checks[0].detail.as_deref(),
            Some("Reason recorded: Advertises another shop")
        );
        let mut tx = store.begin().await.unwrap_or_else(|e| panic!("{e}"));
        let done = DeletePost
            .commit(&mut tx, &planned.plan)
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        let status = |p: Option<repo::PostRow>| p.map(|p| p.moderation_status);
        assert_eq!(
            status(
                repo::post(&mut tx, "post_8811")
                    .await
                    .unwrap_or_else(|e| panic!("{e}"))
            )
            .as_deref(),
            Some("blinded")
        );
        // Running it again changes nothing.
        DeletePost
            .commit(&mut tx, &planned.plan)
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        DeletePost
            .undo(&mut tx, &planned.plan, &done)
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            status(
                repo::post(&mut tx, "post_8811")
                    .await
                    .unwrap_or_else(|e| panic!("{e}"))
            )
            .as_deref(),
            Some("published")
        );
        tx.tx.rollback().await.unwrap_or_else(|e| panic!("{e}"));

        // Jamie deletes her own post; Grace's reply keeps its place.
        let jamie = testing::on_db(testing::shopper()).await;
        let mine = DeletePost
            .preview(
                &jamie,
                DeletePostInput {
                    post_id: "post_8812".into(),
                    action: None,
                    reason: None,
                },
            )
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(mine.changes[0].to, json!("deleted"));
        assert!(
            mine.changes[0]
                .note
                .as_deref()
                .is_some_and(|n| n.contains("keeps its place")),
            "{:?}",
            mine.changes
        );
    }
}
