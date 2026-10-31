//! `update_member`: change a member's name, email or phone. A new email or phone is verified
//! first (`verify_identity` with `purpose: contact_change`). Never changes the password.

use acom_contract::{
    ActionType, Audience, Change, ChangeOp, Check, Committed, Group, Kind, Method, OpMeta,
    Operation, Planned, Slot, ToolError, UndoSupport, Verification, WriteOp,
};
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::{account_basis, account_or_not_found, identifier_taken, shown_contact};
use crate::access::{approval, customer_for};
use crate::repo::{self, ContactFields};
use crate::verification::{usable, use_token};
use crate::{CUSTOMER_NOT_FOUND, DUPLICATE_MEMBER, REVERIFY_REQUIRED, secret, wording};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateMemberInput {
    /// The member; a shopper uses `me`.
    #[schemars(regex(pattern = r"^(me|cus_[a-z0-9_]+)$"), example = &"cus_alex")]
    pub customer_id: String,
    /// The new name to show.
    #[schemars(length(min = 1, max = 100), example = &"Alex Kim")]
    pub name: Option<String>,
    /// The new phone number; needs `verification_token` for it.
    #[schemars(length(min = 7, max = 30), example = &"+1 718 555 0142")]
    pub telephone: Option<String>,
    /// The new email; needs `verification_token` for it.
    #[schemars(length(min = 3, max = 200), example = &"alex.kim@example.org")]
    pub email: Option<String>,
    /// From `verify_identity` with `purpose: contact_change`, for the new email or phone.
    #[schemars(regex(pattern = r"^ivf_[a-z0-9_]+$"), example = &"ivf_203")]
    pub verification_token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateMemberPlan {
    pub customer_id: String,
    pub before: ContactFields,
    pub after: ContactFields,
    /// The identity check a contact change used.
    pub verification_id: Option<String>,
}

pub struct UpdateMember;

impl Operation for UpdateMember {
    const META: OpMeta = OpMeta {
        address: "customer.person.update",
        tool: "update_member",
        group: Group::Engage,
        kind: Kind::Write,
        method: Method::Patch,
        path: "/v1/customer/people/{customer_id}",
        summary: "Change a member's name, email or phone. A new email or phone needs verify_identity first. Never changes the password.",
        consumes: &[Slot::one("customer_id")],
        produces: &[Slot::one("customer_id")],
        checks: &["identity_verification.required_for"],
        errors: &[REVERIFY_REQUIRED, CUSTOMER_NOT_FOUND, DUPLICATE_MEMBER],
        audience: Audience::Both,
        action_type: ActionType::UPDATE,
        person_scope: "",
    };
    type Input = UpdateMemberInput;
    type Output = Value;
}

fn reverify(field: &str) -> ToolError {
    ToolError::new(
        REVERIFY_REQUIRED,
        format!(
            "The new {field} must be verified first (verify_identity, purpose contact_change)."
        ),
    )
    .detail(json!({"field": field, "rule": "identity_verification.required_for.contact_change"}))
    .next("verify_identity")
}

impl WriteOp<Cx> for UpdateMember {
    type Plan = UpdateMemberPlan;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(
        &self,
        cx: &Cx,
        input: UpdateMemberInput,
    ) -> Result<Planned<UpdateMemberPlan>, ToolError> {
        let customer_id = customer_for(cx, &input.customer_id)?;
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let a = account_or_not_found(&mut tx, &customer_id).await?;
        let before = ContactFields::from(&a);
        let mut after = before.clone();
        if let Some(n) = input
            .name
            .map(|n| n.trim().to_owned())
            .filter(|n| !n.is_empty())
        {
            after.name = n;
        }
        let differs = |new: &Option<String>, old: &Option<String>| match new {
            Some(n) => old.as_deref().map(secret::normalize) != Some(secret::normalize(n)),
            None => false,
        };
        let email = input.email.map(|e| e.trim().to_owned());
        let phone = input.telephone.map(|t| t.trim().to_owned());
        let email_changes = differs(&email, &before.email);
        let phone_changes = differs(&phone, &before.telephone);
        if let Some(e) = email.as_deref().filter(|_| email_changes)
            && !wording::looks_like_email(e)
        {
            return Err(
                ToolError::invalid_input(format!("`{e}` is not an email address."))
                    .detail(json!({"field": "email", "reason": "format"})),
            );
        }
        if let Some(t) = phone.as_deref().filter(|_| phone_changes)
            && !wording::looks_like_phone(t)
        {
            return Err(
                ToolError::invalid_input(format!("`{t}` is not a phone number."))
                    .detail(json!({"field": "telephone", "reason": "format"})),
            );
        }
        if email_changes && phone_changes {
            return Err(ToolError::new(
                REVERIFY_REQUIRED,
                "Change the email and the phone one at a time; each is verified on its own.",
            )
            .detail(json!({"field": "telephone", "reason": "one_contact_at_a_time"}))
            .next("update_member"));
        }
        let mut verification_id = None;
        let mut check_detail = "No contact change; nothing to verify".to_owned();
        if email_changes || phone_changes {
            let (field, value) = if email_changes {
                ("email", email.clone().unwrap_or_default())
            } else {
                ("telephone", phone.clone().unwrap_or_default())
            };
            let token = input
                .verification_token
                .as_deref()
                .ok_or_else(|| reverify(field))?;
            let c = usable(
                &mut tx,
                token,
                "contact_change",
                Some(&value),
                "verify_identity",
            )
            .await
            .map_err(|_| reverify(field))?;
            if identifier_taken(&mut tx, field, &secret::normalize(&value)).await? {
                return Err(ToolError::new(
                    DUPLICATE_MEMBER,
                    format!("Another member already uses this {field}."),
                )
                .detail(json!({"entity": "person", "field": field}))
                .next("check_availability"));
            }
            if email_changes {
                after.email = Some(value);
                after.email_verified_at = c.verified_at;
            } else {
                after.telephone = Some(value);
                after.telephone_verified_at = c.verified_at;
            }
            check_detail = format!("Verified for contact change ({})", c.method);
            verification_id = Some(c.id);
        }
        tx.commit().await.map_err(ToolError::from)?;
        let mut changes = Vec::new();
        if after.name != before.name {
            changes.push(Change {
                op: ChangeOp::Update,
                entity: "person".into(),
                id: a.id.clone(),
                from: shown_name(cx, &before.name),
                to: shown_name(cx, &after.name),
                note: Some("name".into()),
            });
        }
        for (field, old, new) in [
            ("email", &before.email, &after.email),
            ("telephone", &before.telephone, &after.telephone),
        ] {
            if old != new {
                changes.push(Change {
                    op: ChangeOp::Update,
                    entity: "person".into(),
                    id: a.id.clone(),
                    from: shown_contact(cx, old.as_deref()),
                    to: shown_contact(cx, new.as_deref()),
                    note: Some(field.into()),
                });
            }
        }
        if changes.is_empty() {
            return Err(ToolError::invalid_input(
                "Nothing to change: every given field already has that value.",
            )
            .detail(json!({"field": "customer_id", "reason": "unchanged"}))
            .next("get_member"));
        }
        let facts = json!({"customer_id": a.id});
        Ok(Planned {
            changes,
            checks: vec![Check {
                rule: "identity_verification.required_for".into(),
                passed: true,
                detail: Some(check_detail),
                fix_tool: Some("verify_identity".into()),
            }],
            basis: vec![account_basis(&a)],
            approval: approval(cx, Self::META.tool, &facts),
            plan: UpdateMemberPlan {
                customer_id: a.id,
                before,
                after,
                verification_id,
            },
        })
    }

    async fn commit(&self, tx: &mut TxCx, plan: &UpdateMemberPlan) -> Result<Committed, ToolError> {
        repo::write_contact(tx, &plan.customer_id, &plan.after).await?;
        if let Some(v) = &plan.verification_id {
            use_token(tx, v).await?;
        }
        let mut produced = Map::new();
        produced.insert("customer_id".into(), json!(plan.customer_id));
        Ok(Committed {
            id: plan.customer_id.clone(),
            produced,
        })
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &UpdateMemberPlan,
        _done: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let a = repo::account(&mut tx, &plan.customer_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let mismatches = match a {
            Some(a) if ContactFields::from(&a) == plan.after => vec![],
            Some(a) => vec![format!("{} differs from the plan", a.id)],
            None => vec![format!("{} is gone", plan.customer_id)],
        };
        Ok(Verification {
            matched: mismatches.is_empty(),
            mismatches,
        })
    }

    async fn undo(
        &self,
        tx: &mut TxCx,
        plan: &UpdateMemberPlan,
        _done: &Committed,
    ) -> Result<(), ToolError> {
        repo::write_contact(tx, &plan.customer_id, &plan.before).await
    }
}

/// A name as the caller may see it: masked for operators.
fn shown_name(cx: &Cx, name: &str) -> Value {
    match crate::access::caller(&cx.principal) {
        crate::access::Caller::Shopper(_) => json!(name),
        _ => json!(wording::mask_name(name)),
    }
}

#[cfg(test)]
mod tests {
    use acom_contract::schema::{closed_schema_for, violations};

    use super::*;

    #[test]
    fn input_is_closed_and_has_no_password() {
        let s = closed_schema_for::<UpdateMemberInput>();
        assert_eq!(violations(&s), Vec::<String>::new(), "{s:#}");
        assert!(s["properties"].get("password").is_none());
    }
}
