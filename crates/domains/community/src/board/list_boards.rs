//! `list_boards` (docs/api/community.md#board): the store's community boards with what each is
//! for, who may write, and what it allows, with the total.

use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot, ToolError,
};
use acom_core::enums::BoardType;
use acom_store::{Cx, Page};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::board_json;
use crate::rules::BoardRules;
use crate::{INVALID_FILTER, list, repo};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListBoardsInput {
    /// Only boards of this kind: free (open talk); review (product reviews); question
    /// (questions to the store); event (store news and events).
    #[serde(rename = "acom:board_type", alias = "board_type")]
    #[schemars(example = &"review")]
    pub board_type: Option<BoardType>,
    /// Page size, 1–100. Default 20.
    #[schemars(range(min = 1, max = 100), example = 20)]
    pub limit: Option<u32>,
    /// `next_cursor` from the previous page.
    #[schemars(example = &"o20")]
    pub cursor: Option<String>,
}

/// The store's boards.
#[derive(Debug)]
pub struct ListBoards;

impl Operation for ListBoards {
    const META: OpMeta = OpMeta {
        address: "community.board.list_boards",
        tool: "list_boards",
        group: Group::Engage,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/community/boards",
        summary: "The store's community boards with the total: what each is for (free talk, reviews, questions, events), who may write, and whether it takes secret posts and attachments.",
        consumes: &[],
        produces: &[Slot::many("board_id")],
        checks: &["board.types"],
        errors: &[INVALID_FILTER],
        audience: Audience::Both,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = ListBoardsInput;
    type Output = Value;
}

impl ReadOp<Cx> for ListBoards {
    async fn read(&self, cx: &Cx, input: ListBoardsInput) -> Result<Value, ToolError> {
        let page = Page::new(input.limit, input.cursor.as_deref())?;
        let rules = BoardRules::from_policy(&cx.policy)?;
        let board_type = input.board_type.map(BoardType::as_str);
        if let Some(t) = board_type
            && !rules.types.iter().any(|k| k == t)
        {
            return Err(ToolError::new(
                INVALID_FILTER,
                format!(
                    "This store has no {t} boards; its kinds are {}.",
                    rules.types.join(", ")
                ),
            )
            .detail(json!({"field": "acom:board_type", "reason": "enum", "allowed": rules.types}))
            .next("list_boards"));
        }
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let (rows, total) = repo::boards(&mut tx, board_type, page.limit, page.offset).await?;
        tx.commit().await.map_err(ToolError::from)?;
        Ok(list(rows.iter().map(board_json).collect(), total, &page))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing;

    #[tokio::test]
    async fn a_kind_the_store_does_not_have_is_an_invalid_filter() {
        let mut cx = testing::shopper();
        cx.policy = std::sync::Arc::new(
            cx.policy
                .with_overrides([("community", "board:\n  types: [free, review]\n")])
                .unwrap_or_else(|e| panic!("{e}")),
        );
        let e = ListBoards
            .read(
                &cx,
                ListBoardsInput {
                    board_type: Some(BoardType::Event),
                    limit: None,
                    cursor: None,
                },
            )
            .await
            .unwrap_err();
        assert_eq!(
            (e.code.as_ref(), e.next_tool.as_deref()),
            ("invalid_filter", Some("list_boards"))
        );
        let schema = acom_contract::schema::closed_schema_for::<ListBoardsInput>();
        assert!(
            acom_contract::schema::violations(&schema).is_empty(),
            "{schema:#}"
        );
        assert_eq!(
            schema["properties"]["acom:board_type"]["enum"],
            json!(["free", "review", "question", "event"])
        );
    }

    #[tokio::test]
    #[ignore = "integration: needs a PostgreSQL database"]
    async fn the_demo_store_has_four_boards() {
        let cx = testing::on_db(testing::shopper()).await;
        let all = ListBoards
            .read(
                &cx,
                ListBoardsInput {
                    board_type: None,
                    limit: None,
                    cursor: None,
                },
            )
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(all["total"], json!(4));
        let reviews = ListBoards
            .read(
                &cx,
                ListBoardsInput {
                    board_type: Some(BoardType::Review),
                    limit: None,
                    cursor: None,
                },
            )
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(reviews["total"], json!(1));
        assert_eq!(reviews["items"][0]["id"], "brd_reviews");
        assert_eq!(reviews["items"][0]["acom:write_permission"], "member");
    }
}
