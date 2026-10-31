//! `publish_notice`: a draft notice goes live for every shopper (`draft` → `published`).

use acom_contract::{
    ActionType, Audience, Basis, Change, ChangeOp, Committed, Group, Kind, Method, OpMeta,
    Operation, Planned, Slot, ToolError, UndoSupport, Verification, WriteOp,
};
use acom_store::{Cx, TxCx};
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::{notice, notice_not_found};
use crate::access::{approval, day, db, parse_time, staff_only};
use crate::{ALREADY_PUBLISHED, NOTICE_NOT_FOUND};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PublishNoticeInput {
    /// The draft notice (`list_notices`).
    #[schemars(regex(pattern = r"^ntc_[a-z0-9_]+$"), example = &"ntc_holiday")]
    pub notice_id: String,
    /// The publication date shoppers see, store time. Now when left out; a later time keeps it
    /// hidden from shoppers until then.
    #[schemars(example = &"2026-11-07T09:00")]
    pub date_published: Option<String>,
    /// Show it above the other notices. Kept as it is when left out.
    #[schemars(example = true)]
    pub pinned: Option<bool>,
}

/// A notice's publication fields.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoticeState {
    pub status: String,
    pub date_published: Option<DateTime<Utc>>,
    pub pinned: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublishNoticePlan {
    pub notice_id: String,
    pub before: NoticeState,
    pub after: NoticeState,
}

pub struct PublishNotice;

impl Operation for PublishNotice {
    const META: OpMeta = OpMeta {
        address: "customer.faq_notice.publish_notice",
        tool: "publish_notice",
        group: Group::Engage,
        kind: Kind::Write,
        method: Method::Post,
        path: "/v1/customer/notices/{notice_id}/publish",
        summary: "Publish a draft notice so every shopper sees it, optionally pinned on top or dated later.",
        consumes: &[Slot::one("notice_id")],
        produces: &[Slot::one("notice_id")],
        checks: &[],
        errors: &[NOTICE_NOT_FOUND, ALREADY_PUBLISHED],
        audience: Audience::Operator,
        action_type: ActionType::UPDATE,
        person_scope: "",
    };
    type Input = PublishNoticeInput;
    type Output = Value;
}

async fn write(tx: &mut TxCx, id: &str, s: &NoticeState) -> Result<(), ToolError> {
    sqlx::query(
        "UPDATE customer.news_article SET notice_status = $3, date_published = $4,
           is_pinned = $5, updated_at = now(), version = version + 1
         WHERE tenant_id = $1 AND id = $2
           AND (notice_status, date_published, is_pinned) IS DISTINCT FROM ($3, $4, $5)",
    )
    .bind(tx.tenant.as_str())
    .bind(id)
    .bind(&s.status)
    .bind(s.date_published)
    .bind(s.pinned)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(())
}

impl WriteOp<Cx> for PublishNotice {
    type Plan = PublishNoticePlan;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(
        &self,
        cx: &Cx,
        input: PublishNoticeInput,
    ) -> Result<Planned<PublishNoticePlan>, ToolError> {
        staff_only(cx)?;
        let when = parse_time(cx, "date_published", input.date_published.as_deref())?
            .unwrap_or_else(|| cx.clock.now());
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let n = notice(&mut tx, &input.notice_id)
            .await?
            .ok_or_else(|| notice_not_found(&input.notice_id))?;
        tx.commit().await.map_err(ToolError::from)?;
        if n.notice_status == "published" {
            return Err(ToolError::new(
                ALREADY_PUBLISHED,
                format!("{} is already published.", n.id),
            )
            .detail(json!({"entity": "notice", "from": "published"}))
            .next("get_notice"));
        }
        let before = NoticeState {
            status: n.notice_status.clone(),
            date_published: n.date_published,
            pinned: n.is_pinned,
        };
        let after = NoticeState {
            status: "published".into(),
            date_published: Some(when),
            pinned: input.pinned.unwrap_or(n.is_pinned),
        };
        let note = format!(
            "Shown to shoppers from {}{}",
            day(cx.time, when),
            if after.pinned { ", pinned on top" } else { "" }
        );
        let facts = json!({"notice_id": n.id, "reaches_customers": true});
        Ok(Planned {
            changes: vec![Change {
                op: ChangeOp::Update,
                entity: "notice".into(),
                id: n.id.clone(),
                from: json!(n.notice_status),
                to: json!("published"),
                note: Some(note),
            }],
            checks: vec![],
            basis: vec![Basis {
                entity: "customer.news_article".into(),
                id: n.id.clone(),
                version: i64::from(n.version),
            }],
            approval: approval(cx, Self::META.tool, &facts),
            plan: PublishNoticePlan {
                notice_id: n.id,
                before,
                after,
            },
        })
    }

    async fn commit(
        &self,
        tx: &mut TxCx,
        plan: &PublishNoticePlan,
    ) -> Result<Committed, ToolError> {
        write(tx, &plan.notice_id, &plan.after).await?;
        let mut produced = Map::new();
        produced.insert("notice_id".into(), json!(plan.notice_id));
        Ok(Committed {
            id: plan.notice_id.clone(),
            produced,
        })
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &PublishNoticePlan,
        _done: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let n = notice(&mut tx, &plan.notice_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let mismatches = match n {
            Some(n) if n.notice_status == "published" => vec![],
            Some(n) => vec![format!("{} is {}", n.id, n.notice_status)],
            None => vec![format!("{} is gone", plan.notice_id)],
        };
        Ok(Verification {
            matched: mismatches.is_empty(),
            mismatches,
        })
    }

    async fn undo(
        &self,
        tx: &mut TxCx,
        plan: &PublishNoticePlan,
        _done: &Committed,
    ) -> Result<(), ToolError> {
        write(tx, &plan.notice_id, &plan.before).await
    }
}

#[cfg(test)]
mod tests {
    use acom_contract::schema::{closed_schema_for, violations};

    use super::*;

    #[test]
    fn input_is_closed_and_it_is_an_operator_write() {
        let s = closed_schema_for::<PublishNoticeInput>();
        assert_eq!(violations(&s), Vec::<String>::new(), "{s:#}");
        assert_eq!(PublishNotice::META.kind, Kind::Write);
        assert_eq!(PublishNotice::META.audience, Audience::Operator);
    }
}
