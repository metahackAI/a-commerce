//! `configure_board` (docs/api/community.md#board): the store creates a board, or changes one
//! when `board_id` is given. Settings left out take the policy's defaults on a new board
//! (`board.write_permission.default`, `board.allow_secret`, `attachment.allowed_by_default`,
//! `moderation.default_mode`) and stay as they are on an existing one. Board names are unique in
//! the store (`board.name_unique`). Undone with its change set: a new board is removed while
//! nobody has posted on it, a changed one gets its settings back.

use acom_contract::{
    ActionType, Audience, Basis, Change, ChangeOp, Check, Committed, ErrorCode, Group, Kind,
    Method, OpMeta, Operation, Planned, Slot, ToolError, UndoSupport, Verification, WriteOp,
};
use acom_core::enums::{BoardType, ModerationMode, WritePermission};
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::settings;
use crate::repo::{self, BoardRow};
use crate::rules::BoardRules;
use crate::{
    BOARD_NOT_FOUND, DUPLICATE_BOARD_NAME, board_not_found, operators_only, policy_approval,
    required_text,
};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ConfigureBoardInput {
    /// The board to change (`list_boards`); leave out to create a board.
    #[schemars(regex(pattern = r"^brd_[a-z0-9_]+$"), example = &"brd_reviews")]
    pub board_id: Option<String>,
    /// The board's name, unique in the store.
    #[schemars(length(min = 1, max = 100), example = &"Product reviews")]
    pub name: String,
    /// What the board is for: free (open talk); review (product reviews); question (questions to
    /// the store); event (store news and events).
    #[serde(rename = "acom:board_type", alias = "board_type")]
    #[schemars(example = &"review")]
    pub board_type: BoardType,
    /// Who may write posts: everyone (guests too); member (signed-in members); staff (the store
    /// only). A new board takes the store's default when left out.
    #[serde(rename = "acom:write_permission", alias = "write_permission")]
    #[schemars(example = &"member")]
    pub write_permission: Option<WritePermission>,
    /// Whether posts may be secret (only the writer and the store read them). A new board takes
    /// the store's default when left out.
    #[serde(rename = "acom:allow_secret", alias = "allow_secret")]
    #[schemars(example = false)]
    pub allow_secret: Option<bool>,
    /// Whether posts may carry uploaded images. A new board takes the store's default when left
    /// out.
    #[serde(rename = "acom:allow_attachment", alias = "allow_attachment")]
    #[schemars(example = true)]
    pub allow_attachment: Option<bool>,
    /// pre_review: new posts wait until the store looks; post_review: they show at once and
    /// reports put them in the queue; none: no queue, reports are only recorded. A new board
    /// takes the store's default when left out.
    #[serde(rename = "acom:moderation_mode", alias = "moderation_mode")]
    #[schemars(example = &"post_review")]
    pub moderation_mode: Option<ModerationMode>,
}

/// What `configure_board` runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigureBoardPlan {
    /// The board as it was; `None` when this creates it.
    pub before: Option<BoardRow>,
    pub after: BoardRow,
    /// Another board with the same name, when names must be unique.
    pub duplicate_of: Option<String>,
}

/// Create a board or change its settings.
#[derive(Debug)]
pub struct ConfigureBoard;

impl Operation for ConfigureBoard {
    const META: OpMeta = OpMeta {
        address: "community.board.configure_board",
        tool: "configure_board",
        group: Group::Engage,
        kind: Kind::Write,
        method: Method::Post,
        path: "/v1/community/boards",
        summary: "The store creates a community board, or changes one by its board_id: name, kind, who may write, secret posts, attachments and moderation (review before or after posts show). Unset settings take the store's defaults.",
        consumes: &[],
        produces: &[Slot::one("board_id")],
        checks: &["board.name_unique"],
        errors: &[DUPLICATE_BOARD_NAME, BOARD_NOT_FOUND],
        audience: Audience::Operator,
        action_type: ActionType::UPDATE,
        person_scope: "",
    };
    type Input = ConfigureBoardInput;
    type Output = Value;
}

/// `invalid_input` for a setting the store's policy does not offer.
fn not_offered(field: &str, value: &str, allowed: &[String]) -> ToolError {
    ToolError::invalid_input(format!(
        "This store does not offer {value} for `{field}`; it offers {}.",
        allowed.join(", ")
    ))
    .detail(json!({"field": field, "reason": "enum", "allowed": allowed}))
}

/// The `board.name_unique` check.
pub(crate) fn name_check(name: &str, unique: bool, other: Option<&str>) -> Check {
    let clash = unique && other.is_some();
    Check {
        rule: "board.name_unique".into(),
        passed: !clash,
        detail: Some(match other {
            Some(id) if unique => format!("Board {id} already uses the name \"{name}\""),
            _ => "No other board uses this name".into(),
        }),
        fix_tool: clash.then(|| "list_boards".into()),
    }
}

fn duplicate(name: &str, existing: &str) -> ToolError {
    ToolError::new(
        DUPLICATE_BOARD_NAME,
        format!("Board {existing} already uses the name \"{name}\"; pick another name."),
    )
    .detail(json!({"entity": "board", "field": "name", "existing_id": existing}))
    .next("list_boards")
}

impl WriteOp<Cx> for ConfigureBoard {
    type Plan = ConfigureBoardPlan;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(
        &self,
        cx: &Cx,
        input: ConfigureBoardInput,
    ) -> Result<Planned<ConfigureBoardPlan>, ToolError> {
        operators_only(cx, "set up community boards")?;
        let name = required_text("name", &input.name)?;
        let rules = BoardRules::from_policy(&cx.policy)?;
        let board_type = input.board_type.as_str();
        if !rules.types.iter().any(|t| t == board_type) {
            return Err(not_offered("acom:board_type", board_type, &rules.types));
        }
        if let Some(w) = input.write_permission
            && !rules.write_permissions.iter().any(|p| p == w.as_str())
        {
            return Err(not_offered(
                "acom:write_permission",
                w.as_str(),
                &rules.write_permissions,
            ));
        }
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let before = match &input.board_id {
            Some(id) => Some(
                repo::board(&mut tx, id)
                    .await?
                    .ok_or_else(|| board_not_found(id))?,
            ),
            None => None,
        };
        let other = repo::board_named(&mut tx, &name, input.board_id.as_deref()).await?;
        tx.commit().await.map_err(ToolError::from)?;

        let after = BoardRow {
            id: before.as_ref().map_or_else(
                || format!("brd_{}", acom_core::id::new_suffix()),
                |b| b.id.clone(),
            ),
            name: name.clone(),
            board_type: board_type.into(),
            write_permission: input.write_permission.map_or_else(
                || {
                    before.as_ref().map_or_else(
                        || rules.default_write_permission.clone(),
                        |b| b.write_permission.clone(),
                    )
                },
                |w| w.as_str().to_owned(),
            ),
            allow_secret: input
                .allow_secret
                .or(before.as_ref().map(|b| b.allow_secret))
                .unwrap_or(rules.allow_secret),
            allow_attachment: input
                .allow_attachment
                .or(before.as_ref().map(|b| b.allow_attachment))
                .unwrap_or(rules.attachment_by_default),
            moderation_mode: input.moderation_mode.map_or_else(
                || {
                    before.as_ref().map_or_else(
                        || rules.default_moderation_mode.clone(),
                        |b| b.moderation_mode.clone(),
                    )
                },
                |m| m.as_str().to_owned(),
            ),
            version: before.as_ref().map_or(1, |b| b.version),
        };
        let changes = match &before {
            None => vec![Change {
                op: ChangeOp::Create,
                entity: "board".into(),
                id: String::new(),
                from: Value::Null,
                to: settings(&after),
                note: None,
            }],
            Some(b) => {
                let (was, now) = (settings(b), settings(&after));
                now.as_object()
                    .into_iter()
                    .flatten()
                    .filter(|(k, v)| was.get(k.as_str()) != Some(v))
                    .map(|(k, v)| Change {
                        op: ChangeOp::Update,
                        entity: "board".into(),
                        id: b.id.clone(),
                        from: was.get(k.as_str()).cloned().unwrap_or(Value::Null),
                        to: v.clone(),
                        note: Some(k.clone()),
                    })
                    .collect()
            }
        };
        Ok(Planned {
            changes,
            checks: vec![name_check(&name, rules.name_unique, other.as_deref())],
            basis: before
                .iter()
                .map(|b| Basis {
                    entity: "community.board".into(),
                    id: b.id.clone(),
                    version: i64::from(b.version),
                })
                .collect(),
            approval: policy_approval(
                cx,
                Self::META.tool,
                &json!({"new_board": before.is_none()}),
            ),
            plan: ConfigureBoardPlan {
                before,
                after,
                duplicate_of: other.filter(|_| rules.name_unique),
            },
        })
    }

    async fn commit(
        &self,
        tx: &mut TxCx,
        plan: &ConfigureBoardPlan,
    ) -> Result<Committed, ToolError> {
        if let Some(other) = &plan.duplicate_of {
            return Err(duplicate(&plan.after.name, other));
        }
        let rules = BoardRules::from_policy(&tx.policy)?;
        if rules.name_unique
            && let Some(other) =
                repo::board_named(tx, &plan.after.name, Some(&plan.after.id)).await?
        {
            return Err(duplicate(&plan.after.name, &other));
        }
        match &plan.before {
            None => repo::insert_board(tx, &plan.after).await?,
            Some(_) => {
                if repo::update_board(tx, &plan.after).await? == 0 {
                    return Err(board_not_found(&plan.after.id));
                }
            }
        }
        let mut produced = Map::new();
        produced.insert("board_id".into(), json!(plan.after.id));
        Ok(Committed {
            id: plan.after.id.clone(),
            produced,
        })
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &ConfigureBoardPlan,
        _done: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let row = repo::board(&mut tx, &plan.after.id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let matched = row.is_some_and(|b| settings(&b) == settings(&plan.after));
        Ok(Verification {
            matched,
            mismatches: if matched {
                Vec::new()
            } else {
                vec![format!(
                    "{} does not read back with its settings",
                    plan.after.id
                )]
            },
        })
    }

    async fn undo(
        &self,
        tx: &mut TxCx,
        plan: &ConfigureBoardPlan,
        _done: &Committed,
    ) -> Result<(), ToolError> {
        match &plan.before {
            None => {
                if repo::delete_empty_board(tx, &plan.after.id).await? == 0
                    && repo::board(tx, &plan.after.id).await?.is_some()
                {
                    let posts = repo::posts_on(tx, &plan.after.id).await?;
                    return Err(ToolError::new(
                        ErrorCode::INVALID_STATE,
                        format!(
                            "{} has {posts} posts now; it stays. Change its settings instead.",
                            plan.after.id
                        ),
                    )
                    .next("configure_board"));
                }
                Ok(())
            }
            Some(before) => repo::update_board(tx, before).await.map(|_| ()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing;

    fn input(board: Option<&str>, name: &str) -> ConfigureBoardInput {
        ConfigureBoardInput {
            board_id: board.map(str::to_owned),
            name: name.into(),
            board_type: BoardType::Free,
            write_permission: None,
            allow_secret: None,
            allow_attachment: None,
            moderation_mode: None,
        }
    }

    #[test]
    fn a_name_in_use_fails_the_check() {
        assert!(name_check("Style talk", true, None).passed);
        let c = name_check("Style talk", true, Some("brd_style"));
        assert!(!c.passed);
        assert_eq!(c.fix_tool.as_deref(), Some("list_boards"));
        assert!(name_check("Style talk", false, Some("brd_style")).passed);
        let schema = acom_contract::schema::closed_schema_for::<ConfigureBoardInput>();
        assert!(
            acom_contract::schema::violations(&schema).is_empty(),
            "{schema:#}"
        );
        assert_eq!(
            schema["properties"]["acom:moderation_mode"]["enum"],
            json!(["pre_review", "post_review", "none"])
        );
    }

    #[tokio::test]
    async fn shoppers_cannot_set_up_boards() {
        let e = ConfigureBoard
            .preview(&testing::shopper(), input(None, "Trail talk"))
            .await
            .unwrap_err();
        assert_eq!(e.code, "forbidden");
    }

    #[tokio::test]
    #[ignore = "integration: needs a PostgreSQL database"]
    async fn the_store_creates_a_board_with_the_defaults_and_undoes_it() {
        let store = testing::on_db(testing::operator()).await;
        let planned = ConfigureBoard
            .preview(&store, input(None, "Trail talk"))
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(planned.changes[0].op, ChangeOp::Create);
        assert_eq!(
            planned.changes[0].to,
            json!({"name": "Trail talk", "acom:board_type": "free",
                   "acom:write_permission": "member", "acom:allow_secret": true,
                   "acom:allow_attachment": true, "acom:moderation_mode": "post_review"})
        );
        assert!(planned.checks[0].passed);
        let mut tx = store.begin().await.unwrap_or_else(|e| panic!("{e}"));
        let done = ConfigureBoard
            .commit(&mut tx, &planned.plan)
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(repo::board(&mut tx, &done.id).await.unwrap_or_else(|e| panic!("{e}")).is_some());
        ConfigureBoard
            .undo(&mut tx, &planned.plan, &done)
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(repo::board(&mut tx, &done.id).await.unwrap_or_else(|e| panic!("{e}")).is_none());
        tx.tx.rollback().await.unwrap_or_else(|e| panic!("{e}"));

        // Renaming a board to a name in use fails the check.
        let clash = ConfigureBoard
            .preview(&store, input(Some("brd_questions"), "style TALK"))
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(!clash.checks[0].passed);
        // Changing one setting shows only that setting.
        let one = ConfigureBoard
            .preview(
                &store,
                ConfigureBoardInput {
                    board_type: BoardType::Question,
                    moderation_mode: Some(ModerationMode::PreReview),
                    ..input(Some("brd_questions"), "Ask the store")
                },
            )
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(one.changes.len(), 1, "{:?}", one.changes);
        assert_eq!(one.changes[0].note.as_deref(), Some("acom:moderation_mode"));
        let e = ConfigureBoard
            .preview(&store, input(Some("brd_nowhere"), "Nowhere"))
            .await
            .unwrap_err();
        assert_eq!(
            (e.code.as_ref(), e.next_tool.as_deref()),
            ("board_not_found", Some("list_boards"))
        );
    }
}
