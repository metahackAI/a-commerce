//! The community rules the operations apply, read from the store's policies
//! (`policies/community.yml` plus overrides). Nothing here is a constant: a missing key is an
//! error, so a broken policy file shows up at the first call instead of as a silent default.

use acom_contract::{ErrorCode, ToolError};
use acom_policy::PolicySet;
use serde_json::Value;

fn missing(path: &str) -> ToolError {
    ToolError::new(
        ErrorCode::INTERNAL,
        format!("The store policy `{path}` is missing or malformed."),
    )
}

fn lookup<'a>(p: &'a PolicySet, path: &str) -> Result<&'a Value, ToolError> {
    p.lookup(path).ok_or_else(|| missing(path))
}

fn flag(p: &PolicySet, path: &str) -> Result<bool, ToolError> {
    lookup(p, path)?.as_bool().ok_or_else(|| missing(path))
}

fn word(p: &PolicySet, path: &str) -> Result<String, ToolError> {
    lookup(p, path)?
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| missing(path))
}

fn words(p: &PolicySet, path: &str) -> Result<Vec<String>, ToolError> {
    lookup(p, path)?
        .as_array()
        .map(|l| {
            l.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .ok_or_else(|| missing(path))
}

fn count(p: &PolicySet, path: &str) -> Result<u64, ToolError> {
    lookup(p, path)?.as_u64().ok_or_else(|| missing(path))
}

fn small(p: &PolicySet, path: &str) -> Result<i64, ToolError> {
    i64::try_from(count(p, path)?).map_err(|_| missing(path))
}

/// `board`, with the defaults a new board takes from `moderation.default_mode` and
/// `attachment.allowed_by_default`.
#[derive(Debug, Clone, PartialEq)]
pub struct BoardRules {
    /// `board.types`: the kinds of board this store has.
    pub types: Vec<String>,
    /// `board.write_permission.values`.
    pub write_permissions: Vec<String>,
    /// `board.write_permission.default`.
    pub default_write_permission: String,
    /// `board.allow_secret`: whether a new board takes secret posts.
    pub allow_secret: bool,
    /// `board.name_unique`: one board per name in the store.
    pub name_unique: bool,
    /// `moderation.default_mode`.
    pub default_moderation_mode: String,
    /// `attachment.allowed_by_default`.
    pub attachment_by_default: bool,
}

impl BoardRules {
    pub fn from_policy(p: &PolicySet) -> Result<Self, ToolError> {
        Ok(Self {
            types: words(p, "community.board.types")?,
            write_permissions: words(p, "community.board.write_permission.values")?,
            default_write_permission: word(p, "community.board.write_permission.default")?,
            allow_secret: flag(p, "community.board.allow_secret")?,
            name_unique: flag(p, "community.board.name_unique")?,
            default_moderation_mode: word(p, "community.moderation.default_mode")?,
            attachment_by_default: flag(p, "community.attachment.allowed_by_default")?,
        })
    }
}

/// `moderation.profanity_filter`: the words the store blocks and what a match does.
#[derive(Debug, Clone, PartialEq)]
pub struct FilterRules {
    pub enabled: bool,
    /// Lowercase words, compared word by word.
    pub blocked_words: Vec<String>,
    /// `on_match: block` refuses the text; `hold_for_review` saves it held for review.
    pub blocks: bool,
}

impl FilterRules {
    pub fn from_policy(p: &PolicySet) -> Result<Self, ToolError> {
        Ok(Self {
            enabled: flag(p, "community.moderation.profanity_filter.enabled")?,
            blocked_words: p
                .lookup("community.moderation.profanity_filter.blocked_words")
                .and_then(Value::as_array)
                .map(|l| {
                    l.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_lowercase)
                        .collect()
                })
                .unwrap_or_default(),
            blocks: word(p, "community.moderation.profanity_filter.on_match")? == "block",
        })
    }
}

/// `spam`: links, writing rate and repeats, for posts and replies alike.
#[derive(Debug, Clone, PartialEq)]
pub struct SpamRules {
    /// The same writer with the same text within this many minutes gets the first result back.
    pub duplicate_window_minutes: i64,
    pub max_links: usize,
    /// Posts and replies together, per writer.
    pub max_posts_per_hour: i64,
    /// `on_match: block` refuses; `hold_for_review` saves it held for review.
    pub blocks: bool,
}

impl SpamRules {
    pub fn from_policy(p: &PolicySet) -> Result<Self, ToolError> {
        Ok(Self {
            duplicate_window_minutes: small(p, "community.spam.duplicate_window_minutes")?,
            max_links: usize::try_from(count(p, "community.spam.max_links")?)
                .map_err(|_| missing("community.spam.max_links"))?,
            max_posts_per_hour: small(p, "community.spam.max_posts_per_hour")?,
            blocks: word(p, "community.spam.on_match")? == "block",
        })
    }
}

/// `attachment`: what a post on a board that takes attachments may carry.
#[derive(Debug, Clone, PartialEq)]
pub struct AttachmentRules {
    pub media_types: Vec<String>,
    pub max_files: usize,
    pub max_file_mb: u64,
}

impl AttachmentRules {
    pub fn from_policy(p: &PolicySet) -> Result<Self, ToolError> {
        Ok(Self {
            media_types: words(p, "community.attachment.media_types")?,
            max_files: usize::try_from(count(p, "community.attachment.max_files")?)
                .map_err(|_| missing("community.attachment.max_files"))?,
            max_file_mb: count(p, "community.attachment.max_file_mb")?,
        })
    }
}

/// `moderation.report_reasons`, `moderation.report_threshold`, `moderation.blind.on_threshold`.
#[derive(Debug, Clone, PartialEq)]
pub struct ReportRules {
    pub reasons: Vec<String>,
    /// Different reporters it takes to hold (or blind) a post.
    pub threshold: i64,
    /// `on_threshold: auto_blind` blinds at once; `hold_for_review` puts it in the queue.
    pub auto_blind: bool,
}

impl ReportRules {
    pub fn from_policy(p: &PolicySet) -> Result<Self, ToolError> {
        Ok(Self {
            reasons: words(p, "community.moderation.report_reasons")?,
            threshold: small(p, "community.moderation.report_threshold")?,
            auto_blind: word(p, "community.moderation.blind.on_threshold")? == "auto_blind",
        })
    }
}

/// `moderation.blind`: what blinding and deleting need and keep.
#[derive(Debug, Clone, PartialEq)]
pub struct BlindRules {
    pub operator_reason_required: bool,
    /// A deleted post with other people's replies keeps its place, its text hidden.
    pub keep_replies_on_delete: bool,
}

impl BlindRules {
    pub fn from_policy(p: &PolicySet) -> Result<Self, ToolError> {
        Ok(Self {
            operator_reason_required: flag(
                p,
                "community.moderation.blind.operator_reason_required",
            )?,
            keep_replies_on_delete: flag(p, "community.moderation.blind.keep_replies_on_delete")?,
        })
    }
}

/// `moderation.author_masking`: the name a shopper's post or reply shows (`Jamie` → `Ja***`).
#[derive(Debug, Clone, PartialEq)]
pub struct AuthorMasking {
    pub visible_chars: usize,
    pub mask_char: char,
}

impl AuthorMasking {
    pub fn from_policy(p: &PolicySet) -> Result<Self, ToolError> {
        let path = "community.moderation.author_masking.visible_chars";
        let visible_chars = usize::try_from(count(p, path)?).map_err(|_| missing(path))?;
        let path = "community.moderation.author_masking.mask_char";
        let mask_char = word(p, path)?.chars().next().ok_or_else(|| missing(path))?;
        Ok(Self {
            visible_chars,
            mask_char,
        })
    }

    /// The name to show: the first `visible_chars` characters and the rest masked. A name that
    /// short keeps its first character only, so some of it is always masked.
    pub fn shown(&self, name: &str) -> String {
        let chars: Vec<char> = name.trim().chars().collect();
        if chars.is_empty() {
            return String::new();
        }
        let keep = self.visible_chars.min(chars.len().saturating_sub(1)).max(1);
        let mut out: String = chars[..keep].iter().collect();
        out.extend(std::iter::repeat_n(self.mask_char, chars.len() - keep));
        out
    }
}

/// `reply.max_depth`: post → reply (1) → reply to a reply (2) …
#[derive(Debug, Clone, PartialEq)]
pub struct ReplyRules {
    pub max_depth: i32,
}

impl ReplyRules {
    pub fn from_policy(p: &PolicySet) -> Result<Self, ToolError> {
        let path = "community.reply.max_depth";
        Ok(Self {
            max_depth: i32::try_from(count(p, path)?).map_err(|_| missing(path))?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::policies;

    #[test]
    fn reads_the_default_community_policy() {
        let p = policies();
        let board = BoardRules::from_policy(&p).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(board.types, ["free", "review", "question", "event"]);
        assert_eq!(board.default_write_permission, "member");
        assert_eq!(board.default_moderation_mode, "post_review");
        assert!(board.attachment_by_default && board.allow_secret && board.name_unique);
        let filter = FilterRules::from_policy(&p).unwrap_or_else(|e| panic!("{e}"));
        assert!(filter.enabled && filter.blocks && filter.blocked_words.is_empty());
        let spam = SpamRules::from_policy(&p).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            (
                spam.duplicate_window_minutes,
                spam.max_links,
                spam.max_posts_per_hour
            ),
            (5, 2, 10)
        );
        let reports = ReportRules::from_policy(&p).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!((reports.threshold, reports.auto_blind), (5, false));
        assert_eq!(reports.reasons.len(), 7);
        let blind = BlindRules::from_policy(&p).unwrap_or_else(|e| panic!("{e}"));
        assert!(blind.operator_reason_required && blind.keep_replies_on_delete);
        let files = AttachmentRules::from_policy(&p).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!((files.max_files, files.max_file_mb), (5, 10));
        let replies = ReplyRules::from_policy(&p).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(replies.max_depth, 2);
    }

    #[test]
    fn masks_names_as_the_policy_says() {
        let m = AuthorMasking::from_policy(&policies()).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(m.shown("Jamie"), "Ja***");
        assert_eq!(m.shown("Owen"), "Ow**");
        assert_eq!(m.shown("Al"), "A*");
        assert_eq!(m.shown("  "), "");
    }

    #[test]
    fn a_store_override_changes_the_rules() {
        let p = policies()
            .with_overrides([(
                "community",
                "moderation:\n  profanity_filter:\n    blocked_words: [Scam]\n    on_match: hold_for_review\n",
            )])
            .unwrap_or_else(|e| panic!("{e}"));
        let filter = FilterRules::from_policy(&p).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(filter.blocked_words, ["scam"]);
        assert!(!filter.blocks);
    }
}
