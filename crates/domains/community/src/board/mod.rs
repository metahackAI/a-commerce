//! The `board` resource (docs/api/community.md#board): what its operations share. A board is a
//! `CreativeWork` (`acom:Board`) that says what it is for, who may write on it, whether it takes
//! secret posts and attachments, and how it is moderated.

pub mod configure_board;
pub mod list_boards;

use serde_json::{Value, json};

use crate::repo::BoardRow;

/// A board as `list_boards` returns it.
pub(crate) fn board_json(b: &BoardRow) -> Value {
    json!({
        "id": b.id,
        "name": b.name,
        "acom:board_type": b.board_type,
        "acom:write_permission": b.write_permission,
        "acom:allow_secret": b.allow_secret,
        "acom:allow_attachment": b.allow_attachment,
        "acom:moderation_mode": b.moderation_mode,
    })
}

/// A board's settings as a preview shows them (no id, no version).
pub(crate) fn settings(b: &BoardRow) -> Value {
    let mut v = board_json(b);
    if let Some(o) = v.as_object_mut() {
        o.remove("id");
    }
    v
}
