//! `list_channels` (`integration.channel.list_channels`): the outside channels the store is
//! connected to and how they are doing. Secrets never come back, only whether the channel can
//! check signed events.

use acom_contract::{
    ActionType, Audience, ErrorClass, ErrorCode, Group, Kind, Method, OpMeta, Operation, ReadOp,
    Slot, ToolError,
};
use acom_core::enums::{ChannelType, ConnectionStatus};
use acom_store::{Cx, Page};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

use super::channel_json;
use crate::{list, repo};

/// A filter or a page the tool cannot read.
pub const INVALID_FILTER: ErrorCode = ErrorCode::new("invalid_filter", ErrorClass::InvalidInput);

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListChannelsInput {
    /// Only channels of this kind. marketplace: a marketplace selling the store's products;
    /// carrier: a parcel carrier; payment: a payment provider; fulfillment: an outside
    /// warehouse.
    #[serde(rename = "acom:channel_type", alias = "channel_type")]
    #[schemars(example = &"carrier")]
    pub channel_type: Option<ChannelType>,
    /// Only channels in this state. connected: reachable; degraded: answering slowly or in
    /// part; disconnected: not reachable.
    #[serde(rename = "acom:connection_status", alias = "connection_status")]
    #[schemars(example = &"connected")]
    pub connection_status: Option<ConnectionStatus>,
    /// Channels per page, 1–100 (default 20).
    #[schemars(range(min = 1, max = 100), example = 20)]
    pub limit: Option<u32>,
    /// `next_cursor` of the previous page.
    #[schemars(example = &"o20")]
    pub cursor: Option<String>,
    /// Top-level fields to return (`id` always comes back); all of them when left out.
    #[schemars(example = ["name", "acom:connection_status"])]
    pub fields: Option<Vec<String>>,
}

/// List the store's outside channels.
#[derive(Debug)]
pub struct ListChannels;

impl Operation for ListChannels {
    const META: OpMeta = OpMeta {
        address: "integration.channel.list_channels",
        tool: "list_channels",
        group: Group::Operate,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/integration/channels",
        summary: "List the outside channels the store is connected to (marketplaces, carriers, payment providers, outside warehouses) with their connection status and last sync. Secrets are never shown.",
        consumes: &[],
        produces: &[Slot::many("channel_id")],
        checks: &[],
        errors: &[INVALID_FILTER],
        audience: Audience::Operator,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = ListChannelsInput;
    type Output = Value;
}

impl ReadOp<Cx> for ListChannels {
    async fn read(&self, cx: &Cx, input: ListChannelsInput) -> Result<Value, ToolError> {
        let page = Page::new(input.limit, input.cursor.as_deref()).map_err(|e| ToolError {
            code: INVALID_FILTER.code.into(),
            next_tool: Some("list_channels".into()),
            ..e
        })?;
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let (rows, total) = repo::channels(
            &mut tx,
            input.channel_type.map(ChannelType::as_str),
            input.connection_status.map(ConnectionStatus::as_str),
            page.limit,
            page.offset,
        )
        .await?;
        tx.commit().await.map_err(ToolError::from)?;
        let items: Vec<Value> = rows.iter().map(|c| channel_json(c, &cx.time)).collect();
        let items =
            acom_contract::fields::keep_each(items, input.fields.as_deref().unwrap_or_default())?;
        Ok(list(items, total, &page))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use acom_contract::schema::{closed_schema_for, violations};
    use serde_json::json;

    #[test]
    fn the_input_is_closed_and_takes_the_spec_names() {
        let s = closed_schema_for::<ListChannelsInput>();
        assert!(violations(&s).is_empty(), "{s:#}");
        assert_eq!(
            s["properties"]["acom:channel_type"]["enum"],
            json!(["marketplace", "carrier", "payment", "fulfillment"])
        );
        let i: ListChannelsInput =
            serde_json::from_value(json!({"channel_type": "carrier", "limit": 5})).unwrap();
        assert_eq!(i.channel_type, Some(ChannelType::Carrier));
        assert!(serde_json::from_value::<ListChannelsInput>(json!({"type": "carrier"})).is_err());
    }

    #[tokio::test]
    async fn a_bad_cursor_is_an_invalid_filter() {
        let e = ListChannels
            .read(
                &crate::testing::offline("adm_sj"),
                serde_json::from_value(json!({"cursor": "page2"})).unwrap(),
            )
            .await
            .unwrap_err();
        assert_eq!(
            (e.code.as_ref(), e.next_tool.as_deref()),
            ("invalid_filter", Some("list_channels"))
        );
    }

    #[tokio::test]
    #[ignore = "integration: needs the demo database (DATABASE_URL)"]
    async fn lists_the_demo_channels_without_secrets() {
        let cx = crate::testing::db("adm_sj").await;
        let all = ListChannels
            .read(&cx, serde_json::from_value(json!({})).unwrap())
            .await
            .unwrap();
        assert_eq!(all["total"], 3, "{all:#}");
        let text = all.to_string();
        assert!(
            !text.contains("hash") && !text.contains("credential"),
            "{text}"
        );
        let carriers = ListChannels
            .read(
                &cx,
                serde_json::from_value(json!({"acom:channel_type": "carrier", "fields": ["name"]}))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            carriers["items"],
            json!([{"id": "chn_carrier_ups", "name": "UPS"}])
        );
    }
}
