//! `update_seller`: a seller's profile (contacts, address, categories). Contract and payout
//! terms change with `set_contract`, the status with `set_seller_status`. Which fields this may
//! change is the store's policy (`profile.editable_fields`). A seller's own staff may update
//! their own profile.

use acom_contract::{
    ActionType, ApprovalNeed, Audience, Basis, Change, ChangeOp, Check, Committed, ErrorCode,
    Group, Kind, Method, OpMeta, Operation, Planned, Slot, ToolError, UndoSupport, Verification,
    WriteOp,
};
use acom_core::enums::SellerStatus;
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::{ContactPointInput, PostalAddressInput};
use crate::repo::{self, Profile};
use crate::rules::Rules;
use crate::viewer;
use crate::{IMMUTABLE_FIELD, PROHIBITED_CATEGORY, SELLER_NOT_FOUND, seller_not_found, store};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateSellerInput {
    /// The seller, from `search_sellers`.
    #[schemars(regex(pattern = r"^slr_[a-z0-9_]+$"), example = &"slr_fieldstone")]
    pub seller_id: String,
    /// The full new list of contacts (replaces the old one).
    #[schemars(length(min = 1, max = 10))]
    pub contact_point: Option<Vec<ContactPointInput>>,
    /// The new business address.
    pub address: Option<PostalAddressInput>,
    /// The full new list of categories (`cat_…`) the seller sells in.
    #[serde(rename = "acom:business_category", alias = "business_category")]
    #[schemars(length(min = 1, max = 20), inner(regex(pattern = r"^cat_[a-z0-9_]+$")),
               example = ["cat_shoes", "cat_bags"])]
    pub business_category: Option<Vec<String>>,
}

/// The profile before and after, fixed at preview time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProfilePlan {
    pub seller_id: String,
    pub before: Profile,
    pub after: Profile,
    /// Fields the policy does not let this tool change (the check failed when not empty).
    pub blocked: Vec<String>,
    /// New categories the store does not take sellers for.
    pub prohibited: Vec<String>,
}

pub struct UpdateSeller;

impl Operation for UpdateSeller {
    const META: OpMeta = OpMeta {
        address: "partner.seller.update_seller",
        tool: "update_seller",
        group: Group::Operate,
        kind: Kind::Write,
        method: Method::Patch,
        path: "/v1/partner/sellers/{seller_id}",
        summary: "Change a seller's profile: contacts, business address or the categories it sells in. Contract terms change with set_contract and the status with set_seller_status.",
        consumes: &[Slot::one("seller_id")],
        produces: &[Slot::one("seller_id")],
        checks: &[
            "profile.editable_fields",
            "onboarding.prohibited_categories",
        ],
        errors: &[SELLER_NOT_FOUND, IMMUTABLE_FIELD, PROHIBITED_CATEGORY],
        audience: Audience::Operator,
        action_type: ActionType::UPDATE,
        person_scope: "",
    };
    type Input = UpdateSellerInput;
    type Output = Value;
}

fn wire(p: &Profile) -> Value {
    let mut m = Map::new();
    if let Some(v) = &p.contact_point {
        m.insert("contact_point".into(), v.clone());
    }
    if let Some(v) = &p.address {
        m.insert("address".into(), v.clone());
    }
    if let Some(v) = &p.business_category {
        m.insert("acom:business_category".into(), json!(v));
    }
    Value::Object(m)
}

/// The fields an input changes, by their policy names.
fn asked_fields(input: &UpdateSellerInput) -> Vec<&'static str> {
    let mut f = Vec::new();
    if input.contact_point.is_some() {
        f.push("contact_point");
    }
    if input.address.is_some() {
        f.push("address");
    }
    if input.business_category.is_some() {
        f.push("business_category");
    }
    f
}

impl WriteOp<Cx> for UpdateSeller {
    type Plan = ProfilePlan;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(
        &self,
        cx: &Cx,
        input: UpdateSellerInput,
    ) -> Result<Planned<ProfilePlan>, ToolError> {
        viewer::staff(cx)?;
        let asked = asked_fields(&input);
        if asked.is_empty() {
            return Err(ToolError::invalid_input(
                "Say what to change: contact_point, address or acom:business_category.",
            )
            .detail(json!({"field": "contact_point", "reason": "missing"})));
        }
        if input.contact_point.as_ref().is_some_and(Vec::is_empty)
            || input.business_category.as_ref().is_some_and(Vec::is_empty)
        {
            return Err(ToolError::invalid_input(
                "A new list of contacts or categories cannot be empty.",
            )
            .detail(json!({"reason": "range"})));
        }
        let rules = Rules::load(&cx.policy)?;
        let blocked: Vec<String> = asked
            .iter()
            .filter(|f| !rules.editable_fields.iter().any(|e| e == **f))
            .map(|f| (*f).to_owned())
            .collect();
        let prohibited: Vec<String> = input
            .business_category
            .iter()
            .flatten()
            .filter(|c| rules.prohibited_categories.contains(c))
            .cloned()
            .collect();

        let mut tx = cx.read().await.map_err(store)?;
        let viewer = viewer::viewer(cx, &mut tx).await?;
        viewer.may_see(&input.seller_id)?;
        let s = repo::seller(&mut tx, &input.seller_id)
            .await?
            .ok_or_else(|| seller_not_found(&input.seller_id))?;
        tx.commit().await.map_err(store)?;
        if s.seller_status == SellerStatus::Terminated.as_str() {
            return Err(ToolError::new(
                ErrorCode::INVALID_STATE,
                format!("{} is terminated; its profile no longer changes.", s.id),
            )
            .detail(json!({"entity": "seller", "from": s.seller_status, "allowed_to": []}))
            .next("set_seller_status"));
        }

        let after = Profile {
            contact_point: input.contact_point.as_ref().map(|c| json!(c)),
            address: input.address.as_ref().map(|a| json!(a)),
            business_category: input.business_category.clone(),
        };
        let before = Profile {
            contact_point: after
                .contact_point
                .as_ref()
                .map(|_| s.contact_point.clone()),
            address: after.address.as_ref().map(|_| s.address.clone()),
            business_category: after
                .business_category
                .as_ref()
                .map(|_| s.business_category.clone()),
        };
        let mut checks = vec![Check {
            rule: "profile.editable_fields".into(),
            passed: blocked.is_empty(),
            detail: Some(if blocked.is_empty() {
                format!("{} can be changed here", asked.join(", "))
            } else {
                format!(
                    "The store's policy keeps {} out of profile edits",
                    blocked.join(", ")
                )
            }),
            fix_tool: None,
        }];
        if input.business_category.is_some() {
            checks.push(Check {
                rule: "onboarding.prohibited_categories".into(),
                passed: prohibited.is_empty(),
                detail: Some(if prohibited.is_empty() {
                    "No prohibited category".into()
                } else {
                    format!(
                        "The store does not take sellers for {}",
                        prohibited.join(", ")
                    )
                }),
                fix_tool: (!prohibited.is_empty()).then(|| "update_seller".into()),
            });
        }
        let approval = cx.policy.approval_for(Self::META.tool, &json!({}));
        Ok(Planned {
            changes: vec![Change {
                op: ChangeOp::Update,
                entity: "seller".into(),
                id: s.id.clone(),
                from: wire(&before),
                to: wire(&after),
                note: None,
            }],
            checks,
            basis: vec![Basis {
                entity: "partner.seller".into(),
                id: s.id.clone(),
                version: i64::from(s.version),
            }],
            approval: if approval.needed {
                ApprovalNeed::Needed {
                    reason: approval
                        .reason
                        .unwrap_or_else(|| "Changes a seller's profile".into()),
                }
            } else {
                ApprovalNeed::NotNeeded
            },
            plan: ProfilePlan {
                seller_id: s.id,
                before,
                after,
                blocked,
                prohibited,
            },
        })
    }

    async fn commit(&self, tx: &mut TxCx, plan: &ProfilePlan) -> Result<Committed, ToolError> {
        if !plan.blocked.is_empty() {
            return Err(ToolError::new(
                IMMUTABLE_FIELD,
                format!(
                    "{} cannot change with update_seller; contract terms change with set_contract and the status with set_seller_status.",
                    plan.blocked.join(", ")
                ),
            )
            .detail(json!({"rule": "profile.editable_fields", "items": plan.blocked}))
            .next("set_contract"));
        }
        if !plan.prohibited.is_empty() {
            return Err(ToolError::new(
                PROHIBITED_CATEGORY,
                format!(
                    "The store does not take sellers for {}.",
                    plan.prohibited.join(", ")
                ),
            )
            .detail(json!({"rule": "onboarding.prohibited_categories", "items": plan.prohibited}))
            .next("update_seller"));
        }
        if repo::update_profile(tx, &plan.seller_id, &plan.after).await? == 0 {
            return Err(seller_not_found(&plan.seller_id));
        }
        Ok(Committed {
            id: plan.seller_id.clone(),
            produced: Map::from_iter([("seller_id".to_owned(), json!(plan.seller_id))]),
        })
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &ProfilePlan,
        _: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(store)?;
        let s = repo::seller(&mut tx, &plan.seller_id).await?;
        tx.commit().await.map_err(store)?;
        let mut mismatches = Vec::new();
        match s {
            None => mismatches.push(format!("seller {} is missing", plan.seller_id)),
            Some(s) => {
                let a = &plan.after;
                if a.contact_point
                    .as_ref()
                    .is_some_and(|c| *c != s.contact_point)
                {
                    mismatches.push("contacts differ".into());
                }
                if a.address.as_ref().is_some_and(|c| *c != s.address) {
                    mismatches.push("address differs".into());
                }
                if a.business_category
                    .as_ref()
                    .is_some_and(|c| *c != s.business_category)
                {
                    mismatches.push("categories differ".into());
                }
            }
        }
        Ok(Verification {
            matched: mismatches.is_empty(),
            mismatches,
        })
    }

    async fn undo(
        &self,
        tx: &mut TxCx,
        plan: &ProfilePlan,
        _: &Committed,
    ) -> Result<(), ToolError> {
        repo::update_profile(tx, &plan.seller_id, &plan.before)
            .await
            .map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{input, offline};

    #[tokio::test]
    async fn an_update_must_change_something() {
        let e = UpdateSeller
            .preview(
                &offline("adm_sj"),
                input(json!({"seller_id": "slr_fieldstone"})),
            )
            .await
            .unwrap_err();
        assert_eq!(e.code, "invalid_input");
    }

    #[test]
    fn contract_fields_are_not_part_of_the_profile_input() {
        let r: Result<UpdateSellerInput, _> = serde_json::from_value(
            json!({"seller_id": "slr_fieldstone", "acom:commission_rate": [{"rate": 0.1}]}),
        );
        assert!(r.is_err());
    }
}
