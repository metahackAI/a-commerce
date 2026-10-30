//! Who is looking at prices: an operator sees costs and may ask for any tier at any time; a
//! shopper always sees their own tier's prices now (`pricing.cost_visibility`,
//! `search.yml query.member_tier_filter_at_query_time`).

use acom_contract::ToolError;
use acom_store::{Cx, TxCx};
use chrono::{DateTime, Utc};
use serde_json::json;

use crate::{TIER_NOT_FOUND, is_operator, neighbors, operator_only, parse_time};

/// The person prices are computed for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Viewer {
    pub operator: bool,
    /// The member tier whose prices apply; `None` is everyone.
    pub tier: Option<String>,
    /// The moment prices are computed for.
    pub at: DateTime<Utc>,
}

impl Viewer {
    /// Refuses the operator-only inputs from anyone else, before anything is read.
    pub fn check_inputs(cx: &Cx, tier: Option<&str>, as_of: Option<&str>) -> Result<(), ToolError> {
        if is_operator(&cx.principal) {
            return Ok(());
        }
        if tier.is_some() {
            return Err(operator_only("valid_for_member_tier"));
        }
        if as_of.is_some() {
            return Err(operator_only("as_of"));
        }
        Ok(())
    }

    /// Reads the operator-only inputs (`valid_for_member_tier`, `as_of`) and refuses them from
    /// anyone else. A shopper gets their own tier from the customer domain.
    pub async fn resolve(
        cx: &Cx,
        tx: &mut TxCx,
        tier: Option<&str>,
        as_of: Option<&str>,
    ) -> Result<Self, ToolError> {
        let operator = is_operator(&cx.principal);
        if !operator {
            if tier.is_some() {
                return Err(operator_only("valid_for_member_tier"));
            }
            if as_of.is_some() {
                return Err(operator_only("as_of"));
            }
            let tier = match cx.principal.customer_id() {
                Some(c) => neighbors::customer_tier(tx, c).await?,
                None => None,
            };
            return Ok(Self {
                operator,
                tier,
                at: cx.clock.now(),
            });
        }
        let at = parse_time(cx, "as_of", as_of)?.unwrap_or_else(|| cx.clock.now());
        let tier = match tier {
            None | Some("everyone") => None,
            Some(t) => {
                if !neighbors::tier_exists(tx, t).await? {
                    return Err(ToolError::new(
                        TIER_NOT_FOUND,
                        format!("There is no member tier `{t}`."),
                    )
                    .detail(json!({"entity": "tier", "id": t}))
                    .next("list_tiers"));
                }
                Some(t.to_owned())
            }
        };
        Ok(Self { operator, tier, at })
    }
}
