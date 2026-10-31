//! `get_seller`: one seller's legal details, contacts, contract summary, grade and products.

use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot, ToolError,
};
use acom_store::Cx;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{profile_wire, tax_id_shown};
use crate::rules::Rules;
use crate::viewer;
use crate::{SELLER_NOT_FOUND, activity, contract, repo, select, seller_not_found, store};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetSellerInput {
    /// The seller, from `search_sellers`.
    #[schemars(regex(pattern = r"^slr_[a-z0-9_]+$"), example = &"slr_fieldstone")]
    pub seller_id: String,
    /// Top-level fields to return, for example `["acom:contract_summary", "acom:grade_level"]`.
    #[schemars(example = ["legal_name", "acom:seller_status", "acom:contract_summary"])]
    pub fields: Option<Vec<String>>,
}

pub struct GetSeller;

impl Operation for GetSeller {
    const META: OpMeta = OpMeta {
        address: "partner.seller.get_seller",
        tool: "get_seller",
        group: Group::Operate,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/partner/sellers/{seller_id}",
        summary: "One seller: legal name, tax id (masked for operators), address, contacts, status, type, the contract in force and the grade now, and the products it sells.",
        consumes: &[Slot::one("seller_id")],
        produces: &[],
        checks: &[],
        errors: &[SELLER_NOT_FOUND],
        audience: Audience::Operator,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = GetSellerInput;
    type Output = Value;
}

impl ReadOp<Cx> for GetSeller {
    async fn read(&self, cx: &Cx, input: GetSellerInput) -> Result<Value, ToolError> {
        viewer::staff(cx)?;
        let rules = Rules::load(&cx.policy)?;
        let now = cx.clock.now();
        let mut tx = cx.read().await.map_err(store)?;
        let viewer = viewer::viewer(cx, &mut tx).await?;
        viewer.may_see(&input.seller_id)?;
        let s = repo::seller(&mut tx, &input.seller_id)
            .await?
            .ok_or_else(|| seller_not_found(&input.seller_id))?;
        let summary = match repo::contract_of(&mut tx, &s.id).await? {
            Some((contract_id, _)) => {
                let versions = repo::versions(&mut tx, &contract_id).await?;
                contract::current(&versions, now).map(|v| {
                    json!({"contract_id": contract_id, "version": v.version_number,
                           "valid_from": cx.time.format(v.valid_from),
                           "valid_through": cx.time.format(v.valid_through),
                           "acom:contract_status": crate::model::contract_status(
                               v.valid_from, v.valid_through, now)})
                })
            }
            None => None,
        };
        let grade = repo::grade_at(&mut tx, &s.id, now).await?;
        let products = activity::products(&mut tx, &s.id).await?;
        tx.commit().await.map_err(store)?;
        let mut out = json!({
            "id": s.id,
            "@type": "Organization",
            "legal_name": s.legal_name,
            "tax_id": tax_id_shown(&s, &viewer, &rules),
            "acom:seller_status": s.seller_status,
            "acom:seller_type": s.seller_type,
            "acom:contract_summary": summary,
            "acom:grade_level": grade.map(|g| g.grade_level),
            "acom:applied_at": cx.time.format(s.applied_at),
            "acom:products": products.iter().map(|p| json!({
                "product_id": p.product_id, "name": p.name,
                "approval_status": p.approval_status})).collect::<Vec<_>>(),
        });
        if let (Some(o), Value::Object(p)) = (out.as_object_mut(), profile_wire(&s)) {
            o.extend(p);
        }
        Ok(select(out, &input.fields.unwrap_or_default()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{input, offline};

    #[tokio::test]
    async fn outside_agents_cannot_read_a_seller() {
        let mut cx = offline("adm_sj");
        cx.principal = std::sync::Arc::new(acom_auth::Principal {
            kind: acom_core::enums::ActorKind::ExternalAgent,
            subject: "key_31".into(),
            ..(*cx.principal).clone()
        });
        let e = GetSeller
            .read(&cx, input(json!({"seller_id": "slr_fieldstone"})))
            .await
            .unwrap_err();
        assert_eq!(e.code, "forbidden");
    }
}
