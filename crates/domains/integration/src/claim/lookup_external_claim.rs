//! `lookup_external_claim` (`integration.claim.lookup_external_claim`): an outside channel's
//! cancel, return and exchange requests and the store claim each maps to. The channel is not
//! connected in this store, so the answer is the copy it last sent, and says so; nothing is
//! asked of the channel and nothing changes.

use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot, ToolError,
};
use acom_core::enums::ClaimType;
use acom_store::{Cx, Page};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

use crate::channel::registered;
use crate::model::{copy_json, holds_orders, lookup_note};
use crate::repo::{self, CopyFilter, CopyKind};
use crate::{CHANNEL_NOT_REGISTERED, EXTERNAL_LOOKUP_FAILED, check_id, list, parse_time};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LookupExternalClaimInput {
    /// The marketplace or outside warehouse (`list_channels`).
    #[schemars(regex(pattern = r"^chn_[a-z0-9_]+$"), example = &"chn_marketplace_a")]
    pub channel_id: String,
    /// One request, by the channel's own number.
    #[schemars(length(min = 1, max = 120), example = &"MK-R-2210")]
    pub external_claim_ref: Option<String>,
    /// Only this kind. cancel: the order is called off; return: items come back for money;
    /// exchange: items come back for others.
    #[schemars(example = &"return")]
    pub claim_type: Option<ClaimType>,
    /// Requests made at or after this time (store time when no offset).
    #[schemars(example = &"2026-11-01T00:00")]
    pub requested_from: Option<String>,
    /// Requests made at or before this time (store time when no offset).
    #[schemars(example = &"2026-11-06T23:59")]
    pub requested_through: Option<String>,
    /// Requests per page, 1–100 (default 20).
    #[schemars(range(min = 1, max = 100), example = 20)]
    pub limit: Option<u32>,
    /// `next_cursor` of the previous page.
    #[schemars(example = &"o20")]
    pub cursor: Option<String>,
}

/// Look up an outside channel's cancel, return and exchange requests.
#[derive(Debug)]
pub struct LookupExternalClaim;

impl Operation for LookupExternalClaim {
    const META: OpMeta = OpMeta {
        address: "integration.claim.lookup_external_claim",
        tool: "lookup_external_claim",
        group: Group::Operate,
        kind: Kind::Read,
        method: Method::Post,
        path: "/v1/integration/claims/lookup",
        summary: "Read an outside channel's cancel, return and exchange requests and the store claim each maps to. Reads the copy the channel last sent; changes nothing.",
        consumes: &[Slot::one("channel_id")],
        produces: &[Slot::many("external_claim_ref")],
        checks: &[],
        errors: &[CHANNEL_NOT_REGISTERED, EXTERNAL_LOOKUP_FAILED],
        audience: Audience::Operator,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = LookupExternalClaimInput;
    type Output = Value;
}

impl ReadOp<Cx> for LookupExternalClaim {
    async fn read(&self, cx: &Cx, input: LookupExternalClaimInput) -> Result<Value, ToolError> {
        check_id("channel_id", "chn", &input.channel_id)?;
        let page = Page::new(input.limit, input.cursor.as_deref())?;
        let filter = CopyFilter {
            reference: input.external_claim_ref.map(|r| r.trim().to_owned()),
            from: input
                .requested_from
                .as_deref()
                .map(|t| parse_time(cx, "requested_from", t))
                .transpose()?,
            through: input
                .requested_through
                .as_deref()
                .map(|t| parse_time(cx, "requested_through", t))
                .transpose()?,
            external_status: None,
            claim_type: input.claim_type.map(|t| t.as_str().to_owned()),
        };
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let channel = registered(&mut tx, &input.channel_id, CHANNEL_NOT_REGISTERED).await?;
        holds_orders(&channel, "list_channels")?;
        let (rows, total) = repo::copies(
            &mut tx,
            CopyKind::Claim,
            &channel.id,
            &filter,
            page.limit,
            page.offset,
        )
        .await?;
        tx.commit().await.map_err(ToolError::from)?;
        let items = rows
            .iter()
            .map(|r| copy_json(&channel, r, CopyKind::Claim, &cx.time))
            .collect();
        let mut out = list(items, total, &page);
        out["acom:source"] = lookup_note(&channel, "requests", &cx.time);
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use acom_contract::schema::{closed_schema_for, violations};
    use serde_json::json;

    #[test]
    fn the_input_is_closed() {
        let s = closed_schema_for::<LookupExternalClaimInput>();
        assert!(violations(&s).is_empty(), "{s:#}");
        assert_eq!(
            s["properties"]["claim_type"]["enum"],
            json!(["cancel", "return", "exchange"])
        );
    }

    #[tokio::test]
    #[ignore = "integration: needs the demo database (DATABASE_URL)"]
    async fn reads_the_marketplace_returns() {
        let cx = crate::testing::db("adm_sj").await;
        let out = LookupExternalClaim
            .read(
                &cx,
                serde_json::from_value(json!({"channel_id": "chn_marketplace_a",
                                              "claim_type": "return"}))
                .unwrap(),
            )
            .await
            .unwrap();
        assert!(out["total"].as_i64().unwrap() >= 1, "{out:#}");
        let r = out["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["external_claim_ref"] == "MK-R-2210")
            .unwrap();
        assert_eq!(r["acom:claim_status"], "requested");
        assert_eq!(r["claim_id"], Value::Null);
        assert_eq!(out["acom:source"]["connected"], false);
    }
}
