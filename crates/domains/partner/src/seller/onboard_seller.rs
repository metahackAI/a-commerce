//! `onboard_seller`: registers a seller's application and starts its review. A payee comes into
//! being, so a person always approves it (W$). The business registry lookup is a stub (no
//! registry provider is connected): the tax id is checked for its form and against the store's
//! other sellers only, and the preview says so.

use acom_contract::{
    ActionType, ApprovalNeed, Audience, Basis, Change, ChangeOp, Check, Committed, ErrorCode,
    Group, Kind, Method, OpMeta, Operation, Planned, Slot, ToolError, UndoSupport, Verification,
    WriteOp,
};
use acom_core::enums::{SellerStatus, SellerType};
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::{ContactPointInput, DocumentInput, PostalAddressInput};
use crate::rules::Rules;
use crate::viewer;
use crate::{
    DUPLICATE_TAX_ID, INVALID_TAX_ID, MISSING_DOCUMENT, PROHIBITED_CATEGORY, model, repo, store,
};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OnboardSellerInput {
    /// The registered name of the business.
    #[schemars(length(min = 1, max = 200), example = &"Harbor Goods LLC")]
    pub legal_name: String,
    /// Business registration (tax) number as on the registration certificate; checked for its
    /// form and against the store's other sellers.
    #[schemars(length(min = 5, max = 40), example = &"512-73-4410")]
    pub tax_id: String,
    /// marketplace: the seller sells and ships, the store takes a commission; consignment: the
    /// store sells the seller's stock; direct_supply: the store buys and resells.
    #[serde(rename = "acom:seller_type", alias = "seller_type")]
    #[schemars(example = &"marketplace")]
    pub seller_type: SellerType,
    /// The business address.
    pub address: PostalAddressInput,
    /// Who handles settlement, customer service and products; at least one.
    #[schemars(length(min = 1, max = 10))]
    pub contact_point: Vec<ContactPointInput>,
    /// Categories (`cat_…`) the seller wants to sell in; checked against prohibited categories.
    #[serde(rename = "acom:business_category", alias = "business_category")]
    #[schemars(length(min = 1, max = 20), inner(regex(pattern = r"^cat_[a-z0-9_]+$")),
               example = ["cat_bags", "cat_hats"])]
    pub business_category: Vec<String>,
    /// The uploaded papers: the business registration and the payout account proof.
    #[serde(rename = "acom:documents", alias = "documents")]
    #[schemars(length(max = 10))]
    pub documents: Vec<DocumentInput>,
}

/// The application as previewed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OnboardPlan {
    pub seller_id: String,
    pub legal_name: String,
    pub tax_id: String,
    pub seller_type: String,
    pub address: Value,
    pub contact_point: Value,
    pub business_category: Vec<String>,
    pub documents: Value,
    /// Categories the policy prohibits (the check failed when not empty).
    pub prohibited: Vec<String>,
    /// Required documents not attached.
    pub missing: Vec<String>,
}

pub struct OnboardSeller;

impl Operation for OnboardSeller {
    const META: OpMeta = OpMeta {
        address: "partner.seller.onboard_seller",
        tool: "onboard_seller",
        group: Group::Operate,
        kind: Kind::WriteApproval,
        method: Method::Post,
        path: "/v1/partner/sellers",
        summary: "Register a seller's application (legal name, tax id, type, address, contacts, categories, documents) and start its review. Creates a payee, so a person always approves.",
        consumes: &[],
        produces: &[Slot::one("seller_id")],
        checks: &[
            "tax_id_unique",
            "onboarding.prohibited_categories",
            "onboarding.required_documents",
        ],
        errors: &[
            DUPLICATE_TAX_ID,
            PROHIBITED_CATEGORY,
            MISSING_DOCUMENT,
            INVALID_TAX_ID,
        ],
        audience: Audience::Operator,
        action_type: ActionType::REGISTER,
        person_scope: "",
    };
    type Input = OnboardSellerInput;
    type Output = Value;
}

fn missing_field(field: &str, what: &str) -> ToolError {
    ToolError::invalid_input(format!("Give {what}."))
        .detail(json!({"field": field, "reason": "missing"}))
}

/// What the input alone decides: the form of the tax id, and the two document and category
/// checks against the policy.
pub(crate) fn screen(
    rules: &Rules,
    input: &OnboardSellerInput,
) -> Result<(Vec<String>, Vec<String>), ToolError> {
    if input.legal_name.trim().is_empty() {
        return Err(missing_field(
            "legal_name",
            "the legal name of the business",
        ));
    }
    if input.contact_point.is_empty() {
        return Err(missing_field("contact_point", "at least one contact"));
    }
    if input.business_category.is_empty() {
        return Err(missing_field(
            "acom:business_category",
            "the categories the seller wants to sell in",
        ));
    }
    if !model::tax_id_well_formed(input.tax_id.trim(), rules.tax_id_digits) {
        return Err(ToolError::new(
            INVALID_TAX_ID,
            format!(
                "`{}` is not a business registration number: use letters, digits and dashes, with {} to {} digits.",
                input.tax_id.trim(),
                rules.tax_id_digits.0,
                rules.tax_id_digits.1
            ),
        )
        .detail(json!({"field": "tax_id", "reason": "format", "rule": "onboarding.tax_id_digits"}))
        .next("onboard_seller"));
    }
    let prohibited: Vec<String> = input
        .business_category
        .iter()
        .filter(|c| rules.prohibited_categories.contains(c))
        .cloned()
        .collect();
    let missing: Vec<String> = rules
        .required_documents
        .iter()
        .filter(|d| {
            !input
                .documents
                .iter()
                .any(|x| x.document_type.as_str() == d.as_str())
        })
        .cloned()
        .collect();
    Ok((prohibited, missing))
}

fn document_checks(prohibited: &[String], missing: &[String], attached: usize) -> Vec<Check> {
    vec![
        Check {
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
            fix_tool: (!prohibited.is_empty()).then(|| "onboard_seller".into()),
        },
        Check {
            rule: "onboarding.required_documents".into(),
            passed: missing.is_empty(),
            detail: Some(if missing.is_empty() {
                format!("All required documents attached ({attached})")
            } else {
                format!("Missing {}", missing.join(", ").replace('_', " "))
            }),
            fix_tool: (!missing.is_empty()).then(|| "onboard_seller".into()),
        },
    ]
}

fn duplicate(tax_id: &str, existing: &(String, String)) -> ToolError {
    ToolError::new(
        DUPLICATE_TAX_ID,
        format!(
            "{} ({}) already uses tax id {tax_id}.",
            existing.1, existing.0
        ),
    )
    .detail(
        json!({"entity": "seller", "field": "tax_id", "existing_id": existing.0,
                   "rule": "tax_id_unique"}),
    )
    .next("get_seller")
}

/// The first free seller id for a legal name: `slr_harbor_goods`, then `slr_harbor_goods_2`, ….
async fn free_id(tx: &mut TxCx, legal_name: &str) -> Result<String, ToolError> {
    let base = format!("slr_{}", model::seller_slug(legal_name));
    let mut id = base.clone();
    let mut n = 1;
    while repo::seller_exists(tx, &id).await? {
        n += 1;
        id = format!("{base}_{n}");
    }
    Ok(id)
}

impl WriteOp<Cx> for OnboardSeller {
    type Plan = OnboardPlan;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(
        &self,
        cx: &Cx,
        input: OnboardSellerInput,
    ) -> Result<Planned<OnboardPlan>, ToolError> {
        viewer::staff(cx)?;
        let rules = Rules::load(&cx.policy)?;
        let (prohibited, missing) = screen(&rules, &input)?;
        let tax_id = input.tax_id.trim().to_owned();
        let mut tx = cx.read().await.map_err(store)?;
        viewer::viewer(cx, &mut tx)
            .await?
            .store_only("register sellers")?;
        let same = repo::sellers_with_tax_id(&mut tx, &model::normalized_tax_id(&tax_id)).await?;
        if rules.tax_id_unique
            && let Some(existing) = same.first()
        {
            return Err(duplicate(
                &model::mask(&tax_id, rules.visible_trailing),
                existing,
            ));
        }
        let known = acom_catalog::service::for_partner::categories(&mut tx).await?;
        if !known.is_empty()
            && let Some(unknown) = input
                .business_category
                .iter()
                .find(|c| !known.iter().any(|k| k.id == **c))
        {
            return Err(
                ToolError::invalid_input(format!("The store has no category {unknown}."))
                    .detail(json!({"field": "acom:business_category", "reason": "enum"})),
            );
        }
        let seller_id = free_id(&mut tx, &input.legal_name).await?;
        tx.commit().await.map_err(store)?;

        let mut checks = vec![Check {
            rule: "tax_id_unique".into(),
            passed: true,
            detail: Some(if rules.tax_id_unique {
                "No other seller uses this tax id".into()
            } else if same.is_empty() {
                "No other seller uses this tax id (the store allows sharing one)".into()
            } else {
                format!(
                    "{} other seller(s) use this tax id; the store allows it",
                    same.len()
                )
            }),
            fix_tool: None,
        }];
        checks.extend(document_checks(
            &prohibited,
            &missing,
            input.documents.len(),
        ));
        let shown_tax_id = if rules.masks("tax_id") {
            model::mask(&tax_id, rules.visible_trailing)
        } else {
            tax_id.clone()
        };
        let plan = OnboardPlan {
            seller_id,
            legal_name: input.legal_name.trim().to_owned(),
            tax_id,
            seller_type: input.seller_type.as_str().to_owned(),
            address: json!(input.address),
            contact_point: json!(input.contact_point),
            business_category: input.business_category.clone(),
            documents: json!(input.documents),
            prohibited,
            missing,
        };
        Ok(Planned {
            changes: vec![Change {
                op: ChangeOp::Create,
                entity: "seller".into(),
                id: String::new(),
                from: Value::Null,
                to: json!({"legal_name": plan.legal_name, "tax_id": shown_tax_id,
                           "acom:seller_status": SellerStatus::UnderReview.as_str(),
                           "acom:seller_type": plan.seller_type,
                           "acom:business_category": plan.business_category}),
                note: Some(
                    "Business registry lookup is a stub (no registry provider is connected): only the tax id's form and the store's own sellers were checked."
                        .into(),
                ),
            }],
            checks,
            basis: Vec::<Basis>::new(),
            approval: ApprovalNeed::Needed {
                reason: "Adds a new seller".into(),
            },
            plan,
        })
    }

    async fn commit(&self, tx: &mut TxCx, plan: &OnboardPlan) -> Result<Committed, ToolError> {
        if !plan.prohibited.is_empty() {
            return Err(ToolError::new(
                PROHIBITED_CATEGORY,
                format!(
                    "The store does not take sellers for {}; leave it out and apply again.",
                    plan.prohibited.join(", ")
                ),
            )
            .detail(json!({"rule": "onboarding.prohibited_categories", "items": plan.prohibited}))
            .next("onboard_seller"));
        }
        if !plan.missing.is_empty() {
            return Err(ToolError::new(
                MISSING_DOCUMENT,
                format!(
                    "Attach the {} and apply again.",
                    plan.missing.join(" and ").replace('_', " ")
                ),
            )
            .detail(json!({"rule": "onboarding.required_documents", "items": plan.missing}))
            .next("onboard_seller"));
        }
        let rules = Rules::load(&tx.policy)?;
        if rules.tax_id_unique
            && let Some(existing) =
                repo::sellers_with_tax_id(tx, &model::normalized_tax_id(&plan.tax_id))
                    .await?
                    .first()
        {
            return Err(duplicate(
                &model::mask(&plan.tax_id, rules.visible_trailing),
                existing,
            ));
        }
        let id = if repo::seller_exists(tx, &plan.seller_id).await? {
            free_id(tx, &plan.legal_name).await?
        } else {
            plan.seller_id.clone()
        };
        let now = tx.clock.now();
        repo::insert_seller(
            tx,
            &repo::NewSeller {
                id: &id,
                legal_name: &plan.legal_name,
                tax_id: &plan.tax_id,
                seller_status: SellerStatus::UnderReview.as_str(),
                seller_type: &plan.seller_type,
                address: &plan.address,
                contact_point: &plan.contact_point,
                business_category: &plan.business_category,
                documents: &plan.documents,
                applied_at: now,
            },
        )
        .await?;
        repo::insert_event(
            tx,
            &repo::NewEvent {
                seller_id: &id,
                action_type: ActionType::CREATE.0,
                action_status: "completed",
                instrument: Self::META.tool,
                from_state: None,
                to_state: SellerStatus::UnderReview.as_str(),
                result: json!({"reason": "Application received; review started",
                               "registry_check": "stub: form and duplicates only"}),
                start_time: now,
                end_time: Some(now),
            },
        )
        .await?;
        Ok(Committed {
            id: id.clone(),
            produced: Map::from_iter([("seller_id".to_owned(), json!(id))]),
        })
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &OnboardPlan,
        done: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(store)?;
        let s = repo::seller(&mut tx, &done.id).await?;
        tx.commit().await.map_err(store)?;
        let mismatches = match s {
            None => vec![format!("seller {} is missing", done.id)],
            Some(s) if s.legal_name != plan.legal_name => {
                vec![format!("seller {} has another legal name", s.id)]
            }
            Some(s) if s.seller_status != SellerStatus::UnderReview.as_str() => {
                vec![format!("seller {} is {}", s.id, s.seller_status)]
            }
            Some(_) => Vec::new(),
        };
        Ok(Verification {
            matched: mismatches.is_empty(),
            mismatches,
        })
    }

    async fn undo(
        &self,
        tx: &mut TxCx,
        _plan: &OnboardPlan,
        done: &Committed,
    ) -> Result<(), ToolError> {
        if repo::delete_application(tx, &done.id).await? {
            Ok(())
        } else {
            Err(ToolError::new(
                ErrorCode::INVALID_STATE,
                format!(
                    "{} already has a contract, a grade or products; terminate it instead.",
                    done.id
                ),
            )
            .detail(json!({"entity": "seller", "id": done.id}))
            .next("set_seller_status"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::tests::rules;
    use crate::tests::{input, offline};

    fn application() -> Value {
        json!({
            "legal_name": "Harbor Goods LLC", "tax_id": "512-73-4410",
            "acom:seller_type": "marketplace",
            "address": {"street_address": "75 Wharf St", "address_locality": "Providence",
                        "address_region": "RI", "postal_code": "02903", "address_country": "US"},
            "contact_point": [{"contact_type": "settlement", "name": "Nadia Fenwick"}],
            "acom:business_category": ["cat_bags"],
            "acom:documents": [{"document_type": "business_registration", "upload_id": "upl_h1"}]
        })
    }

    #[test]
    fn a_missing_payout_proof_fails_the_document_check() {
        let i: OnboardSellerInput = input(application());
        let (prohibited, missing) = screen(&rules(), &i).unwrap();
        assert!(prohibited.is_empty());
        assert_eq!(missing, ["payout_account_proof"]);
        let checks = document_checks(&prohibited, &missing, 1);
        assert!(checks[0].passed);
        assert!(!checks[1].passed);
        assert_eq!(checks[1].fix_tool.as_deref(), Some("onboard_seller"));
    }

    #[tokio::test]
    async fn a_short_tax_id_is_refused_before_anything_is_read() {
        let mut a = application();
        a["tax_id"] = json!("512-73");
        let e = OnboardSeller
            .preview(&offline("adm_sj"), input(a))
            .await
            .unwrap_err();
        assert_eq!(e.code, "invalid_tax_id");
        assert_eq!(e.next_tool.as_deref(), Some("onboard_seller"));
    }
}
