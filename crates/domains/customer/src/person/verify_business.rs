//! `verify_business`: confirm a business member's registration (tax id and legal name). It turns
//! on business pricing and tax receipts, so a person always approves it. The registry lookup is a
//! stub (no registry provider is connected): it compares the details with the ones the member
//! registered, field by field (`business_verification.match_fields`), and says so.

use acom_contract::{
    ActionType, Audience, Change, ChangeOp, Check, Committed, Group, Kind, Method, OpMeta,
    Operation, Planned, Slot, ToolError, UndoSupport, Verification, WriteOp,
};
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::{account_basis, account_or_not_found};
use crate::access::{always, db, policy, policy_words, staff_only};
use crate::{ALREADY_VERIFIED, CUSTOMER_NOT_FOUND, NOT_BUSINESS_MEMBER, REGISTRY_MISMATCH, repo};

/// A registration document the member uploaded.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RegistrationDocument {
    /// Where the uploaded file is (`upl_…` upload or a URL).
    #[schemars(length(min = 1, max = 500), example = &"upl_harbor_registration")]
    pub content_url: String,
    /// The file name people see.
    #[schemars(length(max = 200), example = &"registration.pdf")]
    pub name: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VerifyBusinessInput {
    /// The business member (`search_members` with `type: business`).
    #[schemars(regex(pattern = r"^cus_[a-z0-9_]+$"), example = &"cus_biz_31")]
    pub customer_id: String,
    /// The business registration number.
    #[schemars(length(min = 4, max = 40), example = &"12-3456789")]
    pub tax_id: String,
    /// The registered business name.
    #[schemars(length(min = 1, max = 200), example = &"Harbor Supply Co.")]
    pub legal_name: String,
    /// The registration document, when the store asks for one
    /// (`business_verification.document_required`).
    pub registration_document: Option<RegistrationDocument>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerifyBusinessPlan {
    pub customer_id: String,
    pub before: String,
}

pub struct VerifyBusiness;

impl Operation for VerifyBusiness {
    const META: OpMeta = OpMeta {
        address: "customer.person.verify_business",
        tool: "verify_business",
        group: Group::Engage,
        kind: Kind::WriteApproval,
        method: Method::Post,
        path: "/v1/customer/people/{customer_id}/verify-business",
        summary: "Confirm a business member's tax id and legal name against the business registry, which turns on business pricing and tax receipts. A person always approves.",
        consumes: &[Slot::one("customer_id")],
        produces: &[Slot::one("customer_id")],
        checks: &["business_verification.match_fields"],
        errors: &[
            REGISTRY_MISMATCH,
            ALREADY_VERIFIED,
            NOT_BUSINESS_MEMBER,
            CUSTOMER_NOT_FOUND,
        ],
        audience: Audience::Operator,
        action_type: ActionType::ASSESS,
        person_scope: "",
    };
    type Input = VerifyBusinessInput;
    type Output = Value;
}

/// How registry fields compare: tax ids by their letters and digits, names without case and
/// punctuation.
fn same(field: &str, a: &str, b: &str) -> bool {
    let norm = |s: &str| -> String {
        s.chars()
            .filter(|c| c.is_alphanumeric())
            .flat_map(char::to_lowercase)
            .collect()
    };
    match field {
        "tax_id" | "legal_name" => norm(a) == norm(b),
        _ => a.trim() == b.trim(),
    }
}

async fn write(tx: &mut TxCx, id: &str, value: &str) -> Result<(), ToolError> {
    sqlx::query(
        "UPDATE customer.person SET business_verification = $3, updated_at = now(),
           version = version + 1
         WHERE tenant_id = $1 AND id = $2 AND business_verification <> $3",
    )
    .bind(tx.tenant.as_str())
    .bind(id)
    .bind(value)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(())
}

impl WriteOp<Cx> for VerifyBusiness {
    type Plan = VerifyBusinessPlan;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(
        &self,
        cx: &Cx,
        input: VerifyBusinessInput,
    ) -> Result<Planned<VerifyBusinessPlan>, ToolError> {
        staff_only(cx)?;
        let document_required = policy(cx, "customer.business_verification.document_required")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if document_required && input.registration_document.is_none() {
            return Err(ToolError::invalid_input(
                "This store needs the registration document to verify a business.",
            )
            .detail(
                json!({"field": "registration_document", "reason": "missing",
                           "rule": "business_verification.document_required"}),
            )
            .next("verify_business"));
        }
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let a = account_or_not_found(&mut tx, &input.customer_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        if a.member_type != "business" {
            return Err(ToolError::new(
                NOT_BUSINESS_MEMBER,
                format!("{} is an individual member.", a.id),
            )
            .detail(json!({"entity": "person", "from": a.member_type}))
            .next("get_member"));
        }
        if a.business_verification == "verified" {
            return Err(ToolError::new(
                ALREADY_VERIFIED,
                format!("{} is already a verified business.", a.id),
            )
            .detail(json!({"entity": "person", "from": "verified"}))
            .next("get_member"));
        }
        // The registry stub knows the details the member registered with.
        let fields = policy_words(cx, "customer.business_verification.match_fields");
        let registry = |f: &str| match f {
            "tax_id" => a.tax_id.clone(),
            "legal_name" => a.legal_name.clone(),
            _ => None,
        };
        let given = |f: &str| match f {
            "tax_id" => Some(input.tax_id.clone()),
            "legal_name" => Some(input.legal_name.clone()),
            _ => None,
        };
        let differ: Vec<&String> = fields
            .iter()
            .filter(|f| match (registry(f), given(f)) {
                (Some(r), Some(g)) => !same(f, &r, &g),
                _ => true,
            })
            .collect();
        if !differ.is_empty() {
            return Err(ToolError::new(
                REGISTRY_MISMATCH,
                format!(
                    "The registry has another {} for this business.",
                    differ
                        .iter()
                        .map(|f| f.replace('_', " "))
                        .collect::<Vec<_>>()
                        .join(" and ")
                ),
            )
            .detail(json!({"rule": "business_verification.match_fields", "fields": differ}))
            .next("verify_business"));
        }
        let facts = json!({"customer_id": a.id});
        Ok(Planned {
            changes: vec![Change {
                op: ChangeOp::Update,
                entity: "person".into(),
                id: a.id.clone(),
                from: json!({"acom:business_verification": a.business_verification}),
                to: json!({"acom:business_verification": "verified"}),
                note: Some("Registry lookup stub: compared with the registered details".into()),
            }],
            checks: vec![Check {
                rule: "business_verification.match_fields".into(),
                passed: true,
                detail: Some(format!(
                    "{} match the registry",
                    fields
                        .iter()
                        .map(|f| f.replace('_', " "))
                        .collect::<Vec<_>>()
                        .join(" and ")
                )),
                fix_tool: None,
            }],
            basis: vec![account_basis(&a)],
            approval: always(
                cx,
                Self::META.tool,
                &facts,
                "Turns on business pricing and tax receipts".into(),
            ),
            plan: VerifyBusinessPlan {
                customer_id: a.id,
                before: a.business_verification,
            },
        })
    }

    async fn commit(
        &self,
        tx: &mut TxCx,
        plan: &VerifyBusinessPlan,
    ) -> Result<Committed, ToolError> {
        write(tx, &plan.customer_id, "verified").await?;
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
        plan: &VerifyBusinessPlan,
        _done: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let a = repo::account(&mut tx, &plan.customer_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let mismatches = match a {
            Some(a) if a.business_verification == "verified" => vec![],
            Some(a) => vec![format!("{} is {}", a.id, a.business_verification)],
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
        plan: &VerifyBusinessPlan,
        _done: &Committed,
    ) -> Result<(), ToolError> {
        write(tx, &plan.customer_id, &plan.before).await
    }
}

#[cfg(test)]
mod tests {
    use acom_contract::schema::{closed_schema_for, violations};

    use super::*;

    #[test]
    fn compares_registry_fields_loosely() {
        let s = closed_schema_for::<VerifyBusinessInput>();
        assert_eq!(violations(&s), Vec::<String>::new(), "{s:#}");
        assert!(same("tax_id", "12-3456789", "123456789"));
        assert!(same("legal_name", "Harbor Supply Co.", "harbor supply co"));
        assert!(!same("legal_name", "Harbor Supply Co.", "Harbour Supply"));
    }
}
