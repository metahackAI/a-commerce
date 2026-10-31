//! `list_popups` (docs/api/merchandising.md#popup).

use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot, ToolError,
};
use acom_core::enums::PopupType;
use acom_store::{Cx, Page};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{popup_json, popups};
use crate::display_slot::{StorePage, View, operators_only, view};
use crate::parse_time;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListPopupsInput {
    /// Only popups of this kind: image, notice, event or consent.
    #[schemars(example = &"notice")]
    pub popup_type: Option<PopupType>,
    /// Only popups that open on this page: main, category, product or search.
    #[schemars(example = &"main")]
    pub page: Option<StorePage>,
    /// Only popups open at this moment (store time without an offset). Operators only; a
    /// shopper always gets what is open now.
    #[schemars(example = &"2026-12-24T10:00")]
    pub as_of: Option<String>,
    /// Page size, 1–100. Default 20.
    #[schemars(range(min = 1, max = 100), example = 20)]
    pub limit: Option<u32>,
    /// `next_cursor` from the previous page.
    #[schemars(example = &"o20")]
    pub cursor: Option<String>,
}

/// The store's popups.
#[derive(Debug)]
pub struct ListPopups;

impl Operation for ListPopups {
    const META: OpMeta = OpMeta {
        address: "merchandising.popup.list",
        tool: "list_popups",
        group: Group::Offer,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/merchandising/popups",
        summary: "Store popups with their kind, pages, priority, do-not-show-again days, period and whether they are open now. A shopper gets only the popups open now.",
        consumes: &[],
        produces: &[Slot::many("popup_id")],
        checks: &[],
        errors: &[],
        audience: Audience::Both,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = ListPopupsInput;
    type Output = Value;
}

impl ReadOp<Cx> for ListPopups {
    async fn read(&self, cx: &Cx, input: ListPopupsInput) -> Result<Value, ToolError> {
        let page = Page::new(input.limit, input.cursor.as_deref())?;
        let now = cx.clock.now();
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        // Shoppers see what opens for them now; the schedule is for operators.
        let open_at = match (view(cx, &mut tx).await?, &input.as_of) {
            (View::Shopper(_), Some(_)) => return Err(operators_only("as_of")),
            (View::Shopper(_), None) => Some(now),
            (View::Operator, Some(s)) => Some(parse_time(&cx.time, "as_of", s)?),
            (View::Operator, None) => None,
        };
        let (total, rows) = popups(
            &mut tx,
            input.popup_type.map(PopupType::as_str),
            input.page.map(StorePage::word),
            open_at,
            page,
        )
        .await?;
        tx.commit().await.map_err(ToolError::from)?;
        let items: Vec<Value> = rows
            .iter()
            .map(|p| popup_json(cx, p, open_at.unwrap_or(now)))
            .collect();
        let mut out = json!({"items": items, "total": total});
        if let Some(c) = page.next_cursor(rows.len(), total) {
            out["next_cursor"] = json!(c);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn takes_the_spec_input_with_a_closed_schema() {
        let i: ListPopupsInput = serde_json::from_value(json!({
            "popup_type": "notice", "page": "main", "as_of": "2026-12-24T10:00", "limit": 20}))
        .unwrap();
        assert_eq!(i.popup_type, Some(PopupType::Notice));
        assert!(
            serde_json::from_value::<ListPopupsInput>(json!({"popup_type": "banner"})).is_err()
        );
        let schema = acom_contract::schema::closed_schema_for::<ListPopupsInput>();
        assert!(
            acom_contract::schema::violations(&schema).is_empty(),
            "{schema:#}"
        );
        assert_eq!(
            schema["properties"]["popup_type"]["enum"],
            json!(["image", "notice", "event", "consent"])
        );
    }
}
