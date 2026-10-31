//! The `reply` resource (docs/api/community.md#reply): what its operations share. A reply is a
//! schema.org `Comment` on a post, or on another reply up to `reply.max_depth`. Replies read as a
//! thread: each reply followed by the replies to it, oldest first.

pub mod create_reply;
pub mod delete_reply;
pub mod list_replies;
pub mod update_reply;

use std::collections::BTreeSet;

use acom_contract::{ErrorCode, ToolError};
use acom_core::StoreTime;
use serde_json::{Value, json};

use crate::Viewer;
use crate::model::shown_author;
use crate::repo::ReplyRow;

/// The spec's `not_found` for a reply, pointing at `list_replies`.
pub(crate) fn reply_not_found(id: &str) -> ToolError {
    ToolError::new(ErrorCode::NOT_FOUND, format!("There is no reply {id} you can see."))
        .detail(json!({"entity": "reply", "id": id}))
        .next("list_replies")
}

/// Whether `viewer` may read a reply's text on a post written by `post_author`: the store reads
/// every reply; others read published replies, secret ones only when they wrote the reply or
/// the post, and their own replies while held for review.
pub(crate) fn readable(viewer: &Viewer, post_author: &str, r: &ReplyRow) -> bool {
    let own = viewer.subject == r.author_id;
    if viewer.operator {
        return true;
    }
    match r.moderation_status.as_str() {
        "published" => !r.is_secret || own || viewer.subject == post_author,
        "held_for_review" => own,
        _ => false,
    }
}

/// The thread a viewer sees: readable replies, plus the place (no text) of an unreadable reply
/// that readable replies answer, so the thread keeps its shape. Each entry says whether its text
/// shows. Order: each reply, then the replies to it, oldest first.
pub(crate) fn thread<'a>(
    viewer: &Viewer,
    post_author: &str,
    replies: &'a [ReplyRow],
) -> Vec<(&'a ReplyRow, bool)> {
    let text: BTreeSet<&str> = replies
        .iter()
        .filter(|r| readable(viewer, post_author, r))
        .map(|r| r.id.as_str())
        .collect();
    let mut shown = text.clone();
    loop {
        let parents: Vec<&str> = replies
            .iter()
            .filter(|r| shown.contains(r.id.as_str()))
            .filter_map(|r| r.parent_item_id.as_deref())
            .filter(|p| !shown.contains(p))
            .collect();
        if parents.is_empty() {
            break;
        }
        shown.extend(parents);
    }
    let mut ordered = replies.iter().collect::<Vec<_>>();
    ordered.sort_by(|a, b| (a.date_created, &a.id).cmp(&(b.date_created, &b.id)));
    let mut out = Vec::new();
    fn walk<'a>(
        parent: Option<&str>,
        all: &[&'a ReplyRow],
        shown: &BTreeSet<&str>,
        text: &BTreeSet<&str>,
        out: &mut Vec<(&'a ReplyRow, bool)>,
    ) {
        for r in all
            .iter()
            .filter(|r| r.parent_item_id.as_deref() == parent && shown.contains(r.id.as_str()))
        {
            out.push((r, text.contains(r.id.as_str())));
            walk(Some(&r.id), all, shown, text, out);
        }
    }
    walk(None, &ordered, &shown, &text, &mut out);
    out
}

/// A reply as a list item; `text` is null when only its place shows.
pub(crate) fn reply_item(r: &ReplyRow, text: bool, time: &StoreTime) -> Value {
    json!({
        "id": r.id,
        "text": if text { json!(r.text) } else { Value::Null },
        "author": shown_author(&r.author_id, r.author_name.as_deref()),
        "date_created": time.format(r.date_created),
        "parent_item": r.parent_item_id,
        "acom:is_secret": r.is_secret,
        "acom:moderation_status": r.moderation_status,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};

    fn reply(id: &str, parent: Option<&str>, author: &str, status: &str, minute: u32) -> ReplyRow {
        ReplyRow {
            id: id.into(),
            post_id: "post_8812".into(),
            parent_item_id: parent.map(str::to_owned),
            reply_depth: if parent.is_some() { 2 } else { 1 },
            author_id: author.into(),
            author_name: None,
            text: format!("text of {id}"),
            is_secret: false,
            moderation_status: status.into(),
            moderation_reason: None,
            date_created: Utc
                .with_ymd_and_hms(2026, 11, 5, 20, minute, 0)
                .single()
                .unwrap_or_default(),
            date_modified: None,
            version: 1,
        }
    }

    fn ids(t: &[(&ReplyRow, bool)]) -> Vec<(String, bool)> {
        t.iter().map(|(r, x)| (r.id.clone(), *x)).collect()
    }

    #[test]
    fn a_thread_keeps_each_reply_under_its_parent() {
        let replies = vec![
            reply("rpl_1", None, "cus_grace", "published", 1),
            reply("rpl_3", None, "adm_sj", "published", 3),
            reply("rpl_2", Some("rpl_1"), "cus_jamie", "published", 2),
        ];
        let v = Viewer {
            subject: "cus_owen".into(),
            operator: false,
        };
        assert_eq!(
            ids(&thread(&v, "cus_jamie", &replies)),
            [
                ("rpl_1".to_owned(), true),
                ("rpl_2".to_owned(), true),
                ("rpl_3".to_owned(), true)
            ]
        );
    }

    #[test]
    fn hidden_replies_leave_their_place_only_when_answered() {
        let mut secret = reply("rpl_4", None, "cus_elena", "published", 4);
        secret.is_secret = true;
        let replies = vec![
            reply("rpl_1", None, "cus_grace", "deleted", 1),
            reply("rpl_2", Some("rpl_1"), "cus_jamie", "published", 2),
            reply("rpl_3", None, "cus_noor", "blinded", 3),
            secret,
            reply("rpl_5", None, "cus_theo", "held_for_review", 5),
        ];
        let owen = Viewer {
            subject: "cus_owen".into(),
            operator: false,
        };
        assert_eq!(
            ids(&thread(&owen, "cus_jamie", &replies)),
            [("rpl_1".to_owned(), false), ("rpl_2".to_owned(), true)]
        );
        // The post's writer reads secret replies on it; the reply's writer reads their held one.
        let jamie = Viewer {
            subject: "cus_jamie".into(),
            operator: false,
        };
        assert!(thread(&jamie, "cus_jamie", &replies).iter().any(|(r, t)| r.id == "rpl_4" && *t));
        let theo = Viewer {
            subject: "cus_theo".into(),
            operator: false,
        };
        assert!(thread(&theo, "cus_jamie", &replies).iter().any(|(r, t)| r.id == "rpl_5" && *t));
        let store = Viewer {
            subject: "adm_sj".into(),
            operator: true,
        };
        assert_eq!(thread(&store, "cus_jamie", &replies).len(), 5);
    }
}
