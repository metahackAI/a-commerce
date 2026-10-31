//! Pure moderation logic, no I/O: the word filter, the spam and attachment rules, who may write
//! on a board, reply depth, the report threshold, who may read a post, and the name a writer
//! shows. The operations feed it what they read; the unit tests below hold it to the policy.

use acom_contract::Check;

use crate::Viewer;
use crate::rules::{AttachmentRules, FilterRules, ReplyRules, ReportRules, SpamRules};

/// The first word of `blocked` the text uses, compared word by word and ignoring case.
pub fn blocked_word(text: &str, blocked: &[String]) -> Option<String> {
    if blocked.is_empty() {
        return None;
    }
    let words: Vec<String> = text
        .split(|c: char| !c.is_alphanumeric() && c != '\'')
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect();
    blocked.iter().find(|b| words.contains(b)).cloned()
}

/// The links a text carries: every `http://`, `https://` and bare `www.` address.
pub fn link_count(text: &str) -> usize {
    let lower = text.to_lowercase();
    lower
        .split_whitespace()
        .filter(|w| {
            let w = w.trim_start_matches(|c: char| !c.is_alphanumeric());
            w.starts_with("http://") || w.starts_with("https://") || w.starts_with("www.")
        })
        .count()
}

/// What the word filter makes of a text: the word it found, if the filter is on.
pub fn filter_finds(rules: &FilterRules, text: &str) -> Option<String> {
    if rules.enabled {
        blocked_word(text, &rules.blocked_words)
    } else {
        None
    }
}

/// The `moderation.profanity_filter` check. A blocked word fails it when the store blocks such
/// text, and passes with a note when the store holds it for review instead.
pub fn profanity_check(rules: &FilterRules, found: Option<&str>, fix_tool: &str) -> Check {
    let refused = found.is_some() && rules.blocks;
    Check {
        rule: "moderation.profanity_filter".into(),
        passed: !refused,
        detail: Some(match found {
            _ if !rules.enabled => "The store does not filter words".into(),
            None => "No blocked words".into(),
            Some(w) if rules.blocks => format!("Uses a blocked word ({w})"),
            Some(_) => "Uses a blocked word; it will be held for review".into(),
        }),
        fix_tool: refused.then(|| fix_tool.into()),
    }
}

/// What the spam rule counted for one write.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SpamFinding {
    pub links: usize,
    pub max_links: usize,
    /// Posts and replies by the writer in the last hour, this one included.
    pub this_hour: i64,
    pub max_per_hour: i64,
    pub too_many_links: bool,
    pub too_fast: bool,
}

impl SpamFinding {
    pub fn new(rules: &SpamRules, links: usize, this_hour: i64) -> Self {
        Self {
            links,
            max_links: rules.max_links,
            this_hour,
            max_per_hour: rules.max_posts_per_hour,
            too_many_links: links > rules.max_links,
            too_fast: this_hour > rules.max_posts_per_hour,
        }
    }

    pub fn matched(&self) -> bool {
        self.too_many_links || self.too_fast
    }
}

fn plural(n: impl Into<i64>, one: &str, many: &str) -> String {
    let n = n.into();
    format!("{n} {}", if n == 1 { one } else { many })
}

/// The `spam` check: links per text and writes per hour. Over a limit it fails when the store
/// blocks spam, and passes with a note when the store holds it for review.
pub fn spam_check(rules: &SpamRules, f: &SpamFinding, fix_tool: &str) -> Check {
    let refused = f.matched() && rules.blocks;
    let links = plural(i64::try_from(f.links).unwrap_or(i64::MAX), "link", "links");
    let detail = if f.too_many_links {
        format!("{links}; the store allows {}", f.max_links)
    } else if f.too_fast {
        format!(
            "{} this hour; the store allows {}",
            plural(f.this_hour, "post", "posts"),
            f.max_per_hour
        )
    } else if f.links == 0 && f.this_hour <= 1 {
        "No links".into()
    } else {
        format!(
            "{links}, {} this hour",
            plural(f.this_hour, "post", "posts")
        )
    };
    Check {
        rule: "spam".into(),
        passed: !refused,
        detail: Some(if f.matched() && !rules.blocks {
            format!("{detail}; it will be held for review")
        } else {
            detail
        }),
        fix_tool: (refused && f.too_many_links).then(|| fix_tool.into()),
    }
}

/// The `attachment` check: the board takes attachments, and no more than `max_files`. File types
/// and sizes are checked when a file is uploaded; the detail repeats them.
pub fn attachment_check(rules: &AttachmentRules, board_allows: bool, files: usize) -> Check {
    let (passed, detail) = if files == 0 {
        (true, "No attachments".to_owned())
    } else if !board_allows {
        (false, "This board takes no attachments".to_owned())
    } else if files > rules.max_files {
        (
            false,
            format!("{files} attachments; the store allows {}", rules.max_files),
        )
    } else {
        (
            true,
            format!(
                "{} of up to {} ({}, up to {} MB each)",
                plural(
                    i64::try_from(files).unwrap_or(i64::MAX),
                    "attachment",
                    "attachments"
                ),
                rules.max_files,
                rules.media_types.join(", "),
                rules.max_file_mb
            ),
        )
    };
    Check {
        rule: "attachment".into(),
        passed,
        detail: Some(detail),
        fix_tool: (!passed).then(|| "create_post".into()),
    }
}

/// Who is writing a post or a reply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriterKind {
    /// An operator of the store (or the change-set runner).
    Staff,
    /// A signed-in shopper.
    Customer,
    /// A guest, by their browser session.
    Guest,
}

/// Whether a writer may write where `permission` (`everyone`, `member`, `staff`) applies, and
/// why. Shoppers need an active account; the store may always write.
pub fn may_write(permission: &str, kind: WriterKind, member_state: Option<&str>) -> (bool, String) {
    match (permission, kind) {
        (_, WriterKind::Staff) => (true, "The store can write here".into()),
        ("staff", _) => (false, "Only the store writes here".into()),
        (_, WriterKind::Customer) => match member_state {
            Some("active") => (
                true,
                if permission == "everyone" {
                    "Everyone can write here".into()
                } else {
                    "Members can write here".into()
                },
            ),
            Some(state) => (
                false,
                format!(
                    "The account is {}; it cannot write here",
                    state.replace('_', " ")
                ),
            ),
            None => (false, "No member account was found".into()),
        },
        ("everyone", WriterKind::Guest) => (true, "Everyone can write here".into()),
        (_, WriterKind::Guest) => (false, "Sign in as a member to write here".into()),
    }
}

/// The `board.write_permission` check.
pub fn write_permission_check(
    permission: &str,
    kind: WriterKind,
    member_state: Option<&str>,
) -> Check {
    let (passed, detail) = may_write(permission, kind, member_state);
    Check {
        rule: "board.write_permission".into(),
        passed,
        detail: Some(detail),
        fix_tool: (!passed).then(|| "list_boards".into()),
    }
}

/// Who may reply on a board: everyone where everyone may post; members and the store elsewhere
/// (a board only the store posts on still takes members' replies).
pub fn reply_permission(write_permission: &str) -> &'static str {
    if write_permission == "everyone" {
        "everyone"
    } else {
        "member"
    }
}

/// The `reply.max_depth` check: a reply to the post is depth 1, a reply to that reply 2, ….
pub fn depth_check(rules: &ReplyRules, depth: i32) -> Check {
    let passed = depth <= rules.max_depth;
    Check {
        rule: "reply.max_depth".into(),
        passed,
        detail: Some(if passed {
            format!("Depth {depth} of {}", rules.max_depth)
        } else {
            format!(
                "Depth {depth}; replies go {} deep at most. Reply higher up the thread",
                rules.max_depth
            )
        }),
        fix_tool: (!passed).then(|| "create_reply".into()),
    }
}

/// The `moderation.report_threshold` check for one more report on a post, and the state the
/// post moves to when this report reaches the threshold. A board with no moderation queue
/// (`none`) records reports without holding the post.
pub fn threshold_check(
    rules: &ReportRules,
    moderation_mode: &str,
    status: &str,
    reporters_after: i64,
) -> (Check, Option<&'static str>) {
    let action = if rules.auto_blind {
        "blinds it"
    } else {
        "holds it for review"
    };
    let (detail, to) = if moderation_mode == "none" {
        (
            format!(
                "{} of {}; this board keeps no moderation queue, so the report is recorded only",
                plural(reporters_after, "report", "reports"),
                rules.threshold
            ),
            None,
        )
    } else if status != "published" {
        (
            format!(
                "{} of {}; the post is already {}",
                plural(reporters_after, "report", "reports"),
                rules.threshold,
                status.replace('_', " ")
            ),
            None,
        )
    } else if reporters_after >= rules.threshold {
        (
            format!(
                "{reporters_after} of {} reports; this report {action}",
                rules.threshold
            ),
            Some(if rules.auto_blind {
                "blinded"
            } else {
                "held_for_review"
            }),
        )
    } else {
        let left = rules.threshold - reporters_after;
        (
            format!(
                "{reporters_after} of {} reports; {} more {action}",
                rules.threshold,
                if left == 1 {
                    "one".to_owned()
                } else {
                    left.to_string()
                }
            ),
            None,
        )
    };
    (
        Check {
            rule: "moderation.report_threshold".into(),
            passed: true,
            detail: Some(detail),
            fix_tool: None,
        },
        to,
    )
}

/// The state a new post or reply starts in, and why it is held: a filter that holds instead of
/// blocking, or a board that reviews posts before they show (`pre_review`).
pub fn new_status(
    moderation_mode: &str,
    held_by: Option<&'static str>,
) -> (&'static str, Option<&'static str>) {
    match held_by {
        Some(why) => ("held_for_review", Some(why)),
        None if moderation_mode == "pre_review" => ("held_for_review", Some("pre_review")),
        None => ("published", None),
    }
}

/// The name shown with a post or reply: the masked name kept when it was written, the store for
/// operators, and "A guest" for guests.
pub fn shown_author(author_id: &str, author_name: Option<&str>) -> String {
    match author_name.map(str::trim).filter(|n| !n.is_empty()) {
        Some(n) => n.to_owned(),
        None if author_id.starts_with("adm_") => "The store".into(),
        None if author_id.starts_with("ses_") => "A guest".into(),
        None => "A shopper".into(),
    }
}

/// What a viewer may see of one post.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PostAccess {
    /// Everything.
    Full,
    /// That it exists and its state, not its text (blinded or deleted).
    TextHidden,
    /// A secret post of someone else: `secret_post`.
    Secret,
    /// Not yet public (held for review) and not the viewer's: `post_not_found`.
    Hidden,
}

/// What `viewer` may see of a post by `author_id` in `status`. The store sees every post; the
/// writer sees their own; others see published posts that are not secret, and only the place of
/// a blinded or deleted one.
pub fn post_access(viewer: &Viewer, author_id: &str, status: &str, is_secret: bool) -> PostAccess {
    let own = viewer.subject == author_id;
    if viewer.operator {
        PostAccess::Full
    } else if is_secret && !own {
        PostAccess::Secret
    } else if status == "held_for_review" && !own {
        PostAccess::Hidden
    } else if matches!(status, "blinded" | "deleted") {
        PostAccess::TextHidden
    } else {
        PostAccess::Full
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::{AttachmentRules, FilterRules, ReplyRules, ReportRules, SpamRules};
    use crate::testing::policies;

    fn viewer(subject: &str, operator: bool) -> Viewer {
        Viewer {
            subject: subject.into(),
            operator,
        }
    }

    #[test]
    fn blocked_words_match_whole_words_only() {
        let blocked = vec!["scam".to_owned()];
        assert_eq!(
            blocked_word("This is a SCAM, honestly", &blocked).as_deref(),
            Some("scam")
        );
        assert_eq!(blocked_word("Scampi recipe", &blocked), None);
        assert_eq!(blocked_word("anything", &[]), None);
    }

    #[test]
    fn the_filter_blocks_or_holds_as_the_store_says() {
        let mut rules = FilterRules::from_policy(&policies()).unwrap_or_else(|e| panic!("{e}"));
        rules.blocked_words = vec!["scam".into()];
        let found = filter_finds(&rules, "what a scam");
        let c = profanity_check(&rules, found.as_deref(), "create_post");
        assert!(!c.passed);
        assert_eq!(c.fix_tool.as_deref(), Some("create_post"));
        rules.blocks = false;
        let c = profanity_check(&rules, found.as_deref(), "create_post");
        assert!(c.passed && c.detail.as_deref().is_some_and(|d| d.contains("held")));
        rules.enabled = false;
        assert_eq!(filter_finds(&rules, "what a scam"), None);
    }

    #[test]
    fn spam_counts_links_and_the_writing_rate() {
        let rules = SpamRules::from_policy(&policies()).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            link_count("see https://a.example and (www.b.example) or http://c"),
            3
        );
        assert_eq!(link_count("no links here, just www"), 0);
        let ok = SpamFinding::new(&rules, 1, 2);
        let c = spam_check(&rules, &ok, "create_post");
        assert_eq!(c.detail.as_deref(), Some("1 link, 2 posts this hour"));
        assert!(c.passed);
        let quiet = spam_check(&rules, &SpamFinding::new(&rules, 0, 1), "create_reply");
        assert_eq!(quiet.detail.as_deref(), Some("No links"));
        let links = SpamFinding::new(&rules, 3, 1);
        let c = spam_check(&rules, &links, "create_post");
        assert!(!c.passed && links.too_many_links);
        assert_eq!(c.fix_tool.as_deref(), Some("create_post"));
        let fast = SpamFinding::new(&rules, 0, 11);
        let c = spam_check(&rules, &fast, "create_post");
        assert!(!c.passed && fast.too_fast);
        assert_eq!(c.fix_tool, None);
    }

    #[test]
    fn attachments_need_a_board_that_takes_them() {
        let rules = AttachmentRules::from_policy(&policies()).unwrap_or_else(|e| panic!("{e}"));
        assert!(attachment_check(&rules, false, 0).passed);
        assert!(!attachment_check(&rules, false, 1).passed);
        assert!(attachment_check(&rules, true, 5).passed);
        let c = attachment_check(&rules, true, 6);
        assert!(!c.passed);
        assert_eq!(c.fix_tool.as_deref(), Some("create_post"));
    }

    #[test]
    fn who_may_write_follows_the_board() {
        use WriterKind::*;
        assert!(may_write("member", Customer, Some("active")).0);
        assert!(!may_write("member", Customer, Some("blocked")).0);
        assert!(!may_write("member", Guest, None).0);
        assert!(may_write("everyone", Guest, None).0);
        assert!(!may_write("staff", Customer, Some("active")).0);
        assert!(may_write("staff", Staff, None).0);
        let c = write_permission_check("staff", Customer, Some("active"));
        assert_eq!(c.fix_tool.as_deref(), Some("list_boards"));
        assert_eq!(reply_permission("staff"), "member");
        assert_eq!(reply_permission("everyone"), "everyone");
    }

    #[test]
    fn replies_go_as_deep_as_the_policy_allows() {
        let rules = ReplyRules::from_policy(&policies()).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            depth_check(&rules, 1).detail.as_deref(),
            Some("Depth 1 of 2")
        );
        assert!(depth_check(&rules, 2).passed);
        assert!(!depth_check(&rules, 3).passed);
    }

    #[test]
    fn the_fifth_reporter_holds_a_post_for_review() {
        let mut rules = ReportRules::from_policy(&policies()).unwrap_or_else(|e| panic!("{e}"));
        let (c, to) = threshold_check(&rules, "post_review", "published", 4);
        assert_eq!(
            c.detail.as_deref(),
            Some("4 of 5 reports; one more holds it for review")
        );
        assert_eq!(to, None);
        let (_, to) = threshold_check(&rules, "post_review", "published", 5);
        assert_eq!(to, Some("held_for_review"));
        rules.auto_blind = true;
        let (_, to) = threshold_check(&rules, "post_review", "published", 5);
        assert_eq!(to, Some("blinded"));
        let (_, to) = threshold_check(&rules, "none", "published", 9);
        assert_eq!(to, None);
        let (_, to) = threshold_check(&rules, "post_review", "held_for_review", 9);
        assert_eq!(to, None);
    }

    #[test]
    fn new_writes_start_published_unless_held() {
        assert_eq!(new_status("post_review", None), ("published", None));
        assert_eq!(
            new_status("pre_review", None),
            ("held_for_review", Some("pre_review"))
        );
        assert_eq!(
            new_status("none", Some("spam")),
            ("held_for_review", Some("spam"))
        );
    }

    #[test]
    fn names_show_masked_or_as_the_store() {
        assert_eq!(shown_author("cus_jamie", Some("Ja***")), "Ja***");
        assert_eq!(shown_author("adm_sj", None), "The store");
        assert_eq!(shown_author("ses_01abc", None), "A guest");
    }

    #[test]
    fn secret_and_held_posts_reach_their_writer_and_the_store_only() {
        let jamie = viewer("cus_jamie", false);
        let grace = viewer("cus_grace", false);
        let sj = viewer("adm_sj", true);
        assert_eq!(
            post_access(&grace, "cus_jamie", "published", true),
            PostAccess::Secret
        );
        assert_eq!(
            post_access(&jamie, "cus_jamie", "published", true),
            PostAccess::Full
        );
        assert_eq!(
            post_access(&sj, "cus_jamie", "held_for_review", true),
            PostAccess::Full
        );
        assert_eq!(
            post_access(&grace, "cus_jamie", "held_for_review", false),
            PostAccess::Hidden
        );
        assert_eq!(
            post_access(&jamie, "cus_jamie", "held_for_review", false),
            PostAccess::Full
        );
        assert_eq!(
            post_access(&grace, "cus_jamie", "blinded", false),
            PostAccess::TextHidden
        );
        assert_eq!(
            post_access(&jamie, "cus_jamie", "deleted", false),
            PostAccess::TextHidden
        );
    }
}
