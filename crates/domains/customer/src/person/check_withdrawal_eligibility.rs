//! `check_withdrawal_eligibility`: before a member leaves, what blocks it, what leaving does, and
//! what is kept and for how long. The shopper agent answers "what happens if I leave?" with it,
//! and the withdrawal screen shows the same answer before the member presses the person-only
//! `POST /v1/customer/me/withdrawal`. Operators can read it for a member (masked) but never
//! withdraw anyone.

use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, ReadOp, ToolError, fields,
};
use acom_store::Cx;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{account_or_not_found, data_handling, eligibility};
use crate::access::{customer_for, policy_number, policy_words};
use crate::verification;
use crate::{ALREADY_WITHDRAWN, CUSTOMER_NOT_FOUND, NOT_SIGNED_IN};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CheckWithdrawalEligibilityInput {
    /// The member. `me` (the signed-in shopper) when left out; only operators name another.
    #[schemars(regex(pattern = r"^(me|cus_[a-z0-9_]+)$"), example = &"me")]
    pub customer_id: Option<String>,
    /// Top-level fields to return. All when left out.
    #[schemars(example = ["eligible", "blockers"])]
    pub fields: Option<Vec<String>>,
}

pub struct CheckWithdrawalEligibility;

impl Operation for CheckWithdrawalEligibility {
    const META: OpMeta = OpMeta {
        address: "customer.person.check_withdrawal_eligibility",
        tool: "check_withdrawal_eligibility",
        group: Group::Engage,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/customer/people/me/withdrawal-eligibility",
        summary: "Before a member leaves the store: what blocks it (open orders, claims, inquiries, remedies), what happens on leaving (coupons, subscriptions, tier), and what data is kept for how long. Reads only; only the member can withdraw.",
        consumes: &[],
        produces: &[],
        checks: &[],
        errors: &[NOT_SIGNED_IN, ALREADY_WITHDRAWN, CUSTOMER_NOT_FOUND],
        audience: Audience::Both,
        action_type: ActionType::ASSESS,
        person_scope: "",
    };
    type Input = CheckWithdrawalEligibilityInput;
    type Output = Value;
}

impl ReadOp<Cx> for CheckWithdrawalEligibility {
    async fn read(
        &self,
        cx: &Cx,
        input: CheckWithdrawalEligibilityInput,
    ) -> Result<Value, ToolError> {
        let wanted = input.customer_id.unwrap_or_else(|| "me".into());
        let customer_id = customer_for(cx, &wanted)?;
        let rejoin_days = policy_number(cx, "customer.withdrawal.rejoin.block_days")?;
        let cooling_off = policy_number(cx, "customer.withdrawal.cooling_off_days")?;
        let reuse = crate::access::policy(cx, "customer.withdrawal.identifier_reuse.login_id")
            .cloned()
            .unwrap_or(Value::Null);
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let a = account_or_not_found(&mut tx, &customer_id).await?;
        if a.member_state == "withdrawn" {
            return Err(ToolError::new(
                ALREADY_WITHDRAWN,
                format!("{} already left the store.", a.id),
            )
            .detail(json!({"entity": "person", "from": "withdrawn"})));
        }
        let e = eligibility(&mut tx, &a.id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let mut reauth = verification::allowed(&cx.policy, "withdrawal_reauth");
        if a.password_hash.is_none() {
            reauth.retain(|m| m != "password");
        }
        let out = json!({
            "customer_id": a.id,
            "eligible": e.blockers.is_empty(),
            "checked_at": cx.time.format(cx.clock.now()),
            "blockers": e.blockers,
            "warnings": e.warnings,
            "on_withdrawal": e.on_withdrawal,
            "not_checked": e.unchecked,
            "data_handling": data_handling(&cx.policy),
            "rejoin_after_days": rejoin_days,
            "login_id_reusable": reuse,
            "cooling_off_days": cooling_off,
            "reasons": policy_words(cx, "customer.withdrawal.reasons"),
            "reauthentication_methods": reauth,
        });
        fields::keep(out, &input.fields.unwrap_or_default())
    }
}

#[cfg(test)]
mod tests {
    use acom_contract::schema::{closed_schema_for, violations};

    use super::*;

    #[test]
    fn input_is_closed_and_it_only_reads() {
        let s = closed_schema_for::<CheckWithdrawalEligibilityInput>();
        assert_eq!(violations(&s), Vec::<String>::new(), "{s:#}");
        assert_eq!(CheckWithdrawalEligibility::META.kind, Kind::Read);
    }
}
