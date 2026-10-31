//! The `community` domain: shoppers' boards, posts and replies, and their moderation.
//! Specified in `docs/api/community.md`; rules in `policies/community.yml`.
//!
//! Layout (docs/backend/README.md §2.2): one folder per resource of the spec (`post`, `reply`,
//! `board`), one file per operation named after its tool; what a resource's operations share is
//! in its `mod.rs`. Domain-wide code: `rules` (the community policy values), `model` (pure
//! moderation logic: who may read and write, the filter, spam and depth checks, masked names),
//! `repo` (SQL) and `writer` (who is writing, from the customer domain's service).
//!
//! Shoppers write and change their own posts and replies with a preview and no approval; those
//! writes are outside change sets (`undo: null`). The store's blinding, deleting and board
//! settings are operator writes: the runtime puts them in a change set, which undoes them.

pub mod model;
pub mod rules;

mod repo;
mod writer;

mod board;
mod post;
mod reply;

use acom_auth::Principal;
use acom_contract::{ApprovalNeed, ErrorClass, ErrorCode, RegistryBuilder, ToolError};
use acom_core::enums::ActorKind;
use acom_store::{Cx, StoreError};
use serde_json::{Value, json};

/// No board with that id in this store.
pub const BOARD_NOT_FOUND: ErrorCode = ErrorCode::new("board_not_found", ErrorClass::NotFound);
/// No post with that id that the caller can see.
pub const POST_NOT_FOUND: ErrorCode = ErrorCode::new("post_not_found", ErrorClass::NotFound);
/// A filter value that cannot be used.
pub const INVALID_FILTER: ErrorCode = ErrorCode::new("invalid_filter", ErrorClass::InvalidInput);
/// A secret post: only its writer and the store may read it.
pub const SECRET_POST: ErrorCode = ErrorCode::new("secret_post", ErrorClass::Forbidden);
/// The text uses a word the store blocks.
pub const PROFANITY_BLOCKED: ErrorCode =
    ErrorCode::new("profanity_blocked", ErrorClass::PolicyViolation);
/// The board does not let this caller write posts.
pub const WRITE_NOT_ALLOWED: ErrorCode = ErrorCode::new("write_not_allowed", ErrorClass::Forbidden);
/// The board takes no attachments, or not this many.
pub const ATTACHMENT_NOT_ALLOWED: ErrorCode =
    ErrorCode::new("attachment_not_allowed", ErrorClass::PolicyViolation);
/// Only the writer may change or delete it.
pub const NOT_AUTHOR: ErrorCode = ErrorCode::new("not_author", ErrorClass::Forbidden);
/// An operator blinding or deleting must say why.
pub const REASON_REQUIRED: ErrorCode = ErrorCode::new("reason_required", ErrorClass::InvalidInput);
/// The post (or reply) is blinded or deleted: nothing more can be written on it.
pub const POST_LOCKED: ErrorCode = ErrorCode::new("post_locked", ErrorClass::Locked);
/// A reply deeper than `reply.max_depth`.
pub const DEPTH_EXCEEDED: ErrorCode = ErrorCode::new("depth_exceeded", ErrorClass::LimitExceeded);
/// Another board of the store already has that name.
pub const DUPLICATE_BOARD_NAME: ErrorCode =
    ErrorCode::new("duplicate_board_name", ErrorClass::AlreadyExists);

/// Adds every operation of this domain to the registry.
pub fn register(b: &mut RegistryBuilder<Cx>) {
    // post
    b.read(post::list_posts::ListPosts)
        .read(post::get_post::GetPost)
        .write(post::create_post::CreatePost)
        .write(post::update_post::UpdatePost)
        .write(post::delete_post::DeletePost)
        .write(post::report_post::ReportPost);
    // reply
    b.read(reply::list_replies::ListReplies)
        .write(reply::create_reply::CreateReply)
        .write(reply::update_reply::UpdateReply)
        .write(reply::delete_reply::DeleteReply);
    // board
    b.read(board::list_boards::ListBoards)
        .write(board::configure_board::ConfigureBoard);
}

/// Whether the caller works for the store: an operator, our agent acting for one, or the system
/// (the change-set runner). They read secret, held and blinded posts and moderate them.
pub fn is_operator(p: &Principal) -> bool {
    match p.kind {
        ActorKind::Operator | ActorKind::System => true,
        ActorKind::AgentOnBehalf => p.subject.starts_with("adm_"),
        ActorKind::Shopper | ActorKind::ExternalAgent => false,
    }
}

/// Who is reading: the caller's id and whether they work for the store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Viewer {
    pub subject: String,
    pub operator: bool,
}

impl Viewer {
    pub fn of(p: &Principal) -> Self {
        Self {
            subject: p.subject.clone(),
            operator: is_operator(p),
        }
    }
}

/// `board_not_found`, pointing at `list_boards`.
pub(crate) fn board_not_found(id: &str) -> ToolError {
    ToolError::new(
        BOARD_NOT_FOUND,
        format!("There is no board {id} in this store."),
    )
    .detail(json!({"entity": "board", "id": id}))
    .next("list_boards")
}

/// `post_not_found`, pointing at `list_posts`.
pub(crate) fn post_not_found(id: &str) -> ToolError {
    ToolError::new(
        POST_NOT_FOUND,
        format!("There is no post {id} you can see."),
    )
    .detail(json!({"entity": "post", "id": id}))
    .next("list_posts")
}

/// `not_author`: someone other than the writer tries to change or delete it.
pub(crate) fn not_author(entity: &str, id: &str, next: &'static str) -> ToolError {
    ToolError::new(
        NOT_AUTHOR,
        format!("Only the one who wrote {id} can change or delete this {entity}."),
    )
    .detail(json!({"reason": "owner", "entity": entity, "id": id}))
    .next(next)
}

/// `post_locked`: the post or reply is blinded or deleted.
pub(crate) fn locked(entity: &str, id: &str, status: &str) -> ToolError {
    ToolError::new(
        POST_LOCKED,
        format!("{id} is {status}; nothing more can be written on this {entity}."),
    )
    .detail(json!({"entity": entity, "id": id, "status": status}))
    .next("get_post")
}

/// `profanity_blocked`: the text uses a word the store does not allow.
pub(crate) fn profanity_blocked(word: &str, next: &'static str) -> ToolError {
    ToolError::new(
        PROFANITY_BLOCKED,
        format!(
            "The text uses a word this store does not allow ({word}). Change it and try again."
        ),
    )
    .detail(json!({"rule": "moderation.profanity_filter", "word": word}))
    .next(next)
}

/// The spam rule refused the text: too many links (`invalid_input`) or too many posts and
/// replies in the last hour (`rate_limited`).
pub(crate) fn spam_refused(spam: &model::SpamFinding, next: &'static str) -> ToolError {
    if spam.too_many_links {
        ToolError::invalid_input(format!(
            "The text has {} links; this store allows {}.",
            spam.links, spam.max_links
        ))
        .detail(json!({"rule": "spam", "field": "text", "reason": "range",
                       "limit": spam.max_links, "actual": spam.links}))
        .next(next)
    } else {
        ToolError::new(
            ErrorCode::RATE_LIMITED,
            format!(
                "That would be {} posts and replies in an hour; this store allows {}.",
                spam.this_hour, spam.max_per_hour
            ),
        )
        .detail(json!({"rule": "spam", "retry_after_seconds": 3600,
                       "limit": spam.max_per_hour, "actual": spam.this_hour}))
    }
}

/// A write only the store may run: refused for shoppers.
pub(crate) fn operators_only(cx: &Cx, what: &str) -> Result<(), ToolError> {
    if is_operator(&cx.principal) {
        Ok(())
    } else {
        Err(
            ToolError::new(ErrorCode::FORBIDDEN, format!("Only the store can {what}."))
                .detail(json!({"reason": "role"})),
        )
    }
}

/// Whether `approval.yml` asks for a person on this write (`thresholds.*` whose `applies_to`
/// names the tool). Community writes have no threshold by default and run without one.
pub(crate) fn policy_approval(cx: &Cx, tool: &str, facts: &Value) -> ApprovalNeed {
    let a = cx.policy.approval_for(tool, facts);
    match a.reason {
        Some(reason) if a.needed => ApprovalNeed::Needed { reason },
        _ if a.needed => ApprovalNeed::Needed {
            reason: format!("The store asks a person to approve {tool}"),
        },
        _ => ApprovalNeed::NotNeeded,
    }
}

/// A text input with the spaces around it removed; `None` when nothing is left.
pub(crate) fn trimmed(s: Option<&str>) -> Option<String> {
    s.map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

/// A required text input, trimmed; `invalid_input` when empty.
pub(crate) fn required_text(field: &str, s: &str) -> Result<String, ToolError> {
    trimmed(Some(s)).ok_or_else(|| {
        ToolError::invalid_input(format!("`{field}` is empty."))
            .detail(json!({"field": field, "reason": "missing"}))
    })
}

pub(crate) fn db(e: sqlx::Error) -> ToolError {
    ToolError::from(StoreError::from(e))
}

/// Keeps only the named top-level fields of an object (all of them when `fields` is empty).
pub fn project(v: Value, fields: &[String]) -> Value {
    match v {
        Value::Object(map) if !fields.is_empty() => Value::Object(
            map.into_iter()
                .filter(|(k, _)| fields.iter().any(|f| f == k))
                .collect(),
        ),
        other => other,
    }
}

/// A list envelope `{items, total, next_cursor?}`.
pub(crate) fn list(items: Vec<Value>, total: i64, page: &acom_store::Page) -> Value {
    let returned = items.len();
    let mut out = json!({"items": items, "total": total});
    if let Some(c) = page.next_cursor(returned, total) {
        out["next_cursor"] = json!(c);
    }
    out
}

#[cfg(test)]
pub(crate) mod testing {
    use acom_auth::{Principal, Surface, scopes};
    use acom_core::enums::ActorKind;
    use acom_core::{FixedClock, StoreTime, TenantId};
    use acom_policy::{FILES, PolicySet};
    use acom_store::{Cx, Db};
    use std::sync::Arc;

    /// The default policies in `policies/`.
    pub fn policies() -> PolicySet {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../policies");
        let texts: Vec<(&str, String)> = FILES
            .iter()
            .map(|f| {
                (
                    *f,
                    std::fs::read_to_string(format!("{dir}/{f}.yml")).unwrap_or_default(),
                )
            })
            .collect();
        PolicySet::from_defaults(texts.iter().map(|(f, t)| (*f, t.as_str())))
            .unwrap_or_else(|e| panic!("policies: {e}"))
    }

    /// A context whose database is never reached: for checks that fail before any query.
    /// Store time is Fri Nov 6 2026, 11:00 (the demo clock).
    pub fn cx(kind: ActorKind, subject: &str) -> Cx {
        let tenant = TenantId::parse("store_northwind").unwrap_or_else(|e| panic!("{e}"));
        let time = StoreTime::new(chrono_tz::America::New_York);
        let now = time
            .parse("2026-11-06T11:00")
            .unwrap_or_else(|e| panic!("{e}"));
        let operator = matches!(kind, ActorKind::Operator);
        Cx {
            principal: Arc::new(Principal {
                subject: subject.into(),
                kind,
                tenant: tenant.clone(),
                surface: if operator { Surface::Bo } else { Surface::Fo },
                scopes: if operator {
                    scopes::admin()
                } else {
                    scopes::shopper()
                },
                name: None,
                session_id: None,
            }),
            tenant,
            request_id: "req_test".into(),
            clock: Arc::new(FixedClock::at(now)),
            time,
            currency: acom_core::Currency::parse("USD").unwrap_or_else(|e| panic!("{e}")),
            policy: Arc::new(policies()),
            db: Db::connect_lazy("postgres://nobody@127.0.0.1:1/none", 1)
                .unwrap_or_else(|e| panic!("{e}")),
        }
    }

    /// Jamie, a signed-in shopper.
    pub fn shopper() -> Cx {
        cx(ActorKind::Shopper, "cus_jamie")
    }

    /// A guest without an account.
    pub fn guest() -> Cx {
        cx(ActorKind::Shopper, "ses_guest")
    }

    /// SJ, the demo store's operator.
    pub fn operator() -> Cx {
        cx(ActorKind::Operator, "adm_sj")
    }

    /// The same context on the real database (`DATABASE_URL`), for the integration tests
    /// (`cargo test -- --ignored`).
    pub async fn on_db(mut cx: Cx) -> Cx {
        let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| panic!("DATABASE_URL"));
        cx.db = Db::connect(&url, 2).await.unwrap_or_else(|e| panic!("{e}"));
        cx
    }

    /// Another person, as the same kind of caller.
    pub fn as_subject(cx: &Cx, subject: &str) -> Cx {
        let mut p = (*cx.principal).clone();
        p.subject = subject.into();
        Cx {
            principal: Arc::new(p),
            ..cx.clone()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operators_and_their_agents_moderate_but_shoppers_do_not() {
        let tenant =
            acom_core::TenantId::parse("store_northwind").unwrap_or_else(|e| panic!("{e}"));
        let mut p = Principal::system(tenant, "sys_runner");
        assert!(is_operator(&p));
        p.kind = ActorKind::AgentOnBehalf;
        p.subject = "adm_sj".into();
        assert!(is_operator(&p));
        p.subject = "cus_jamie".into();
        assert!(!is_operator(&p));
        p.kind = ActorKind::Shopper;
        assert!(!is_operator(&p));
    }

    #[test]
    fn keeps_only_the_named_fields() {
        let v = json!({"id": "post_8812", "headline": "Runs a bit small", "comment_count": 3});
        let kept = project(v.clone(), &["id".into(), "headline".into()]);
        assert_eq!(
            kept,
            json!({"id": "post_8812", "headline": "Runs a bit small"})
        );
        assert_eq!(project(v.clone(), &[]), v);
    }

    #[test]
    fn the_registry_takes_every_operation_with_closed_schemas() {
        let mut b = RegistryBuilder::default();
        register(&mut b);
        let r = b.build().unwrap_or_else(|e| panic!("{e:#?}"));
        assert_eq!(r.len(), 12);
        for op in r.iter() {
            assert!(
                op.meta().address.starts_with("community."),
                "{}",
                op.meta().tool
            );
        }
    }
}
