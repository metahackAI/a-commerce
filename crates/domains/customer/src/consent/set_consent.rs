//! `set_consent`: give or withdraw one consent. A new record is added; nothing is overwritten.
//! Terms and privacy can only be withdrawn by leaving the store.

use acom_contract::{
    ActionType, Audience, Basis, Change, ChangeOp, Check, Committed, Group, Kind, Method, OpMeta,
    Operation, Planned, Slot, ToolError, UndoSupport, Verification, WriteOp,
};
use acom_core::enums::ConsentType;
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::{current, new_id, record};
use crate::access::{approval, customer_for, policy_words};
use crate::person::customer_not_found;
use crate::{CANNOT_REVOKE_MANDATORY, CUSTOMER_NOT_FOUND, UNKNOWN_CONSENT_TYPE, repo};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetConsentInput {
    /// The member; a shopper uses `me`.
    #[schemars(regex(pattern = r"^(me|cus_[a-z0-9_]+)$"), example = &"me")]
    pub customer_id: String,
    /// What the consent is for: terms; privacy; marketing_email; marketing_sms; marketing_push;
    /// marketing_in_app. Event consents are given when entering an event.
    #[schemars(example = &"marketing_email")]
    pub consent_type: ConsentType,
    /// `true` gives the consent, `false` withdraws it.
    #[schemars(example = false)]
    pub granted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetConsentPlan {
    pub customer_id: String,
    pub consent_type: String,
    pub granted: bool,
    /// The record to add, or the current one when it already says the same.
    pub consent_id: String,
    /// `false` when the member's answer already is `granted`.
    pub changes_anything: bool,
}

pub struct SetConsent;

impl Operation for SetConsent {
    const META: OpMeta = OpMeta {
        address: "customer.consent.set",
        tool: "set_consent",
        group: Group::Engage,
        kind: Kind::Write,
        method: Method::Post,
        path: "/v1/customer/people/{customer_id}/consent",
        summary: "Give or withdraw one consent (marketing email, SMS, push, in app). Adds a record to the history; terms and privacy cannot be withdrawn.",
        consumes: &[Slot::one("customer_id")],
        produces: &[Slot::one("consent_id")],
        checks: &["consent.mandatory"],
        errors: &[
            CUSTOMER_NOT_FOUND,
            CANNOT_REVOKE_MANDATORY,
            UNKNOWN_CONSENT_TYPE,
        ],
        audience: Audience::Both,
        action_type: ActionType::UPDATE,
        person_scope: "",
    };
    type Input = SetConsentInput;
    type Output = Value;
}

impl WriteOp<Cx> for SetConsent {
    type Plan = SetConsentPlan;
    /// Consent history is never rewritten: the way back is `set_consent` with the other value.
    const UNDO: UndoSupport = UndoSupport::NotSupported;

    async fn preview(
        &self,
        cx: &Cx,
        input: SetConsentInput,
    ) -> Result<Planned<SetConsentPlan>, ToolError> {
        let customer_id = customer_for(cx, &input.customer_id)?;
        let kind = input.consent_type.as_str();
        let mandatory = policy_words(cx, "customer.consent.mandatory");
        let optional = policy_words(cx, "customer.consent.optional");
        let is_mandatory = mandatory.iter().any(|m| m == kind);
        if !is_mandatory && !optional.iter().any(|o| o == kind) {
            let allowed: Vec<&String> = mandatory.iter().chain(&optional).collect();
            return Err(ToolError::new(
                UNKNOWN_CONSENT_TYPE,
                format!("{kind} is not a consent members set here."),
            )
            .detail(json!({"field": "consent_type", "reason": "enum", "allowed": allowed}))
            .next("list_enums"));
        }
        if is_mandatory && !input.granted {
            return Err(ToolError::new(
                CANNOT_REVOKE_MANDATORY,
                format!(
                    "{kind} is required for an account; it ends only when the member withdraws."
                ),
            )
            .detail(json!({"rule": "consent.mandatory", "consent_type": kind}))
            .next("check_withdrawal_eligibility"));
        }
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let person = repo::person(&mut tx, &customer_id)
            .await?
            .ok_or_else(|| customer_not_found(&customer_id))?;
        let now = current(&mut tx, &customer_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let existing = now.iter().find(|r| r.consent_type == kind);
        let same = existing.is_some_and(|r| r.granted == input.granted);
        let changes = if same {
            vec![]
        } else {
            vec![Change {
                op: ChangeOp::Create,
                entity: "consent_record".into(),
                id: String::new(),
                from: existing.map_or(Value::Null, |r| json!({kind: r.granted})),
                to: json!({kind: input.granted}),
                note: None,
            }]
        };
        let detail = match (is_mandatory, same) {
            (_, true) => format!(
                "{kind} already is {}",
                if input.granted { "given" } else { "withdrawn" }
            ),
            (true, false) => format!("{kind} is required and is being given"),
            (false, false) => format!("{kind} is optional"),
        };
        let facts = json!({"customer_id": customer_id, "consent_type": kind});
        Ok(Planned {
            changes,
            checks: vec![Check {
                rule: "consent.mandatory".into(),
                passed: true,
                detail: Some(detail),
                fix_tool: None,
            }],
            basis: vec![Basis {
                entity: "customer.person".into(),
                id: person.id.clone(),
                version: i64::from(person.version),
            }],
            approval: approval(cx, Self::META.tool, &facts),
            plan: SetConsentPlan {
                consent_id: existing
                    .filter(|_| same)
                    .map_or_else(new_id, |r| r.id.clone()),
                customer_id,
                consent_type: kind.to_owned(),
                granted: input.granted,
                changes_anything: !same,
            },
        })
    }

    async fn commit(&self, tx: &mut TxCx, plan: &SetConsentPlan) -> Result<Committed, ToolError> {
        if plan.changes_anything {
            record(
                tx,
                &plan.consent_id,
                &plan.customer_id,
                &plan.consent_type,
                plan.granted,
                Self::META.tool,
            )
            .await?;
        }
        let mut produced = Map::new();
        produced.insert("consent_id".into(), json!(plan.consent_id));
        Ok(Committed {
            id: plan.consent_id.clone(),
            produced,
        })
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &SetConsentPlan,
        _done: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let now = current(&mut tx, &plan.customer_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let mismatches = match now.iter().find(|r| r.consent_type == plan.consent_type) {
            Some(r) if r.granted == plan.granted => vec![],
            Some(r) => vec![format!(
                "{} is {} in {}",
                plan.consent_type, r.granted, r.id
            )],
            None => vec![format!("no {} record", plan.consent_type)],
        };
        Ok(Verification {
            matched: mismatches.is_empty(),
            mismatches,
        })
    }

    async fn undo(
        &self,
        _tx: &mut TxCx,
        _plan: &SetConsentPlan,
        _done: &Committed,
    ) -> Result<(), ToolError> {
        // Not reached: UNDO is NotSupported, and the history is never rewritten.
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use acom_contract::schema::{closed_schema_for, violations};

    use super::*;

    #[test]
    fn input_is_closed_with_consent_words() {
        let s = closed_schema_for::<SetConsentInput>();
        assert_eq!(violations(&s), Vec::<String>::new(), "{s:#}");
        let words = s["properties"]["consent_type"]["enum"].as_array().unwrap();
        assert!(words.contains(&json!("marketing_email")));
        assert_eq!(SetConsent::UNDO, UndoSupport::NotSupported);
    }
}
