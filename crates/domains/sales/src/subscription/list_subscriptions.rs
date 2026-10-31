//! `list_subscriptions` (`sales.subscription.list`): standing orders, newest first. A shopper
//! sees their own; an operator sees every one, or one customer's.

use acom_contract::{
    ActionType, Audience, ErrorCode, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot,
    ToolError,
};
use acom_core::enums::SubscriptionStatus;
use acom_store::{Cx, Page};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::subscription_json;
use crate::owner::{self, Viewer};
use crate::repo;
use crate::{INVALID_FILTER, NOT_SIGNED_IN, not_signed_in};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListSubscriptionsInput {
    /// Whose: a customer id (operators), or `me`. A shopper always gets their own.
    #[schemars(regex(pattern = r"^(cus_[a-z0-9_]+|me)$"), example = &"me")]
    pub customer_id: Option<String>,
    /// Only subscriptions in these states. All when left out.
    #[schemars(example = ["active", "paused"])]
    pub status: Option<Vec<SubscriptionStatus>>,
    /// Page size, 1–100. Default 20.
    #[schemars(range(min = 1, max = 100), example = 20)]
    pub limit: Option<u32>,
    /// `next_cursor` from the previous page.
    #[schemars(example = &"o20")]
    pub cursor: Option<String>,
}

pub(crate) struct ListSubscriptions;

impl Operation for ListSubscriptions {
    const META: OpMeta = OpMeta {
        address: "sales.subscription.list",
        tool: "list_subscriptions",
        group: Group::Transact,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/sales/subscriptions",
        summary: "Standing orders, newest first: what each brings, how often, the next charge date and amount, where it goes and its state. Shoppers see their own, operators everyone's or one customer's.",
        consumes: &[],
        produces: &[Slot::many("subscription_id")],
        checks: &[],
        errors: &[INVALID_FILTER, NOT_SIGNED_IN],
        audience: Audience::Both,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = ListSubscriptionsInput;
    type Output = Value;
}

impl ReadOp<Cx> for ListSubscriptions {
    async fn read(&self, cx: &Cx, input: ListSubscriptionsInput) -> Result<Value, ToolError> {
        let asked = input.customer_id.as_deref();
        let customer_id = match owner::viewer(&cx.principal) {
            Viewer::Shopper(me) => match asked {
                None | Some("me") => Some(me),
                Some(c) if c == me => Some(me),
                Some(_) => {
                    return Err(ToolError::new(
                        ErrorCode::FORBIDDEN,
                        "You can see your own subscriptions only.",
                    )
                    .detail(json!({"reason": "owner"}))
                    .next("list_subscriptions"));
                }
            },
            Viewer::Staff => match asked {
                Some("me") => {
                    return Err(ToolError::new(
                        INVALID_FILTER,
                        "`me` is for shoppers; give the customer's id, or leave it out for everyone.",
                    )
                    .detail(json!({"field": "customer_id", "reason": "format"}))
                    .next("search_members"));
                }
                other => other.map(str::to_owned),
            },
            Viewer::Guest => return Err(not_signed_in()),
        };
        let page = Page::new(input.limit, input.cursor.as_deref())?;
        let statuses: Vec<String> = input
            .status
            .unwrap_or_default()
            .iter()
            .map(|s| s.as_str().to_owned())
            .collect();
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let (rows, total) = repo::subscriptions(
            &mut tx,
            customer_id.as_deref(),
            &statuses,
            page.limit,
            page.offset,
        )
        .await?;
        tx.commit().await.map_err(ToolError::from)?;
        let items: Vec<Value> = rows
            .iter()
            .map(|r| subscription_json(r, &cx.time))
            .collect::<Result<_, _>>()?;
        let mut v = json!({"items": items, "total": total});
        if let Some(c) = page.next_cursor(rows.len(), total) {
            v["next_cursor"] = json!(c);
        }
        Ok(v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use acom_contract::schema::{closed_schema_for, violations};

    #[test]
    fn input_schema_is_closed_with_states_as_words() {
        let s = closed_schema_for::<ListSubscriptionsInput>();
        assert!(violations(&s).is_empty(), "{s:#}");
        assert_eq!(
            s["properties"]["status"]["items"]["enum"],
            json!(["active", "paused", "cancelled"])
        );
    }

    #[tokio::test]
    async fn shoppers_list_their_own_only() {
        let cx = crate::testdb::offline("cus_jamie");
        let e = ListSubscriptions
            .read(
                &cx,
                ListSubscriptionsInput {
                    customer_id: Some("cus_grace".into()),
                    status: None,
                    limit: None,
                    cursor: None,
                },
            )
            .await
            .unwrap_err();
        assert_eq!(e.code, "forbidden");
    }

    /// Integration (`DATABASE_URL`, demo seed): Jamie's monthly merino tee, and everyone's for
    /// an operator.
    #[tokio::test]
    #[ignore = "integration: needs a PostgreSQL database"]
    async fn lists_jamies_and_everyones() {
        let jamie = crate::testdb::cx("cus_jamie").await;
        let mine = ListSubscriptions
            .read(
                &jamie,
                ListSubscriptionsInput {
                    customer_id: Some("me".into()),
                    status: None,
                    limit: None,
                    cursor: None,
                },
            )
            .await
            .unwrap();
        assert_eq!(mine["total"], 1);
        let s = &mine["items"][0];
        assert_eq!(s["id"], "sub_3301");
        assert_eq!(s["billing_cycle"], "every_month");
        assert_eq!(s["next_charge_date"], "2026-12-01");
        assert_eq!(s["payment"]["label"], "Card ending 4821");
        let sj = crate::testdb::cx("adm_sj").await;
        let paused = ListSubscriptions
            .read(
                &sj,
                ListSubscriptionsInput {
                    customer_id: None,
                    status: Some(vec![SubscriptionStatus::Paused]),
                    limit: None,
                    cursor: None,
                },
            )
            .await
            .unwrap();
        assert!(
            paused["items"]
                .as_array()
                .unwrap()
                .iter()
                .any(|s| s["id"] == "sub_3302")
        );
    }
}
