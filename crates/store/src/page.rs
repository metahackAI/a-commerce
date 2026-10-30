//! Paging for list reads (docs/api/conventions.md §6): `limit` (default 20, at most 100) and an
//! opaque `cursor` taken from the previous response's `next_cursor`.

use acom_contract::ToolError;
use serde_json::json;

/// One page of a list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Page {
    pub limit: i64,
    pub offset: i64,
}

impl Page {
    pub const DEFAULT_LIMIT: u32 = 20;
    pub const MAX_LIMIT: u32 = 100;

    /// Reads `limit` and `cursor` from an input. A cursor not made by [`Page::next_cursor`] is
    /// refused with `invalid_filter`.
    pub fn new(limit: Option<u32>, cursor: Option<&str>) -> Result<Self, ToolError> {
        let limit = limit.unwrap_or(Self::DEFAULT_LIMIT);
        if !(1..=Self::MAX_LIMIT).contains(&limit) {
            return Err(ToolError::invalid_input(format!(
                "`limit` must be 1–{}.",
                Self::MAX_LIMIT
            ))
            .detail(json!({"field": "limit", "reason": "range"})));
        }
        let offset = match cursor {
            None => 0,
            Some(c) => c
                .strip_prefix("o")
                .and_then(|n| n.parse::<i64>().ok())
                .filter(|n| *n >= 0)
                .ok_or_else(|| {
                    ToolError::invalid_input(
                        "`cursor` must be a `next_cursor` from an earlier page.",
                    )
                    .detail(json!({"field": "cursor", "reason": "format"}))
                })?,
        };
        Ok(Self {
            limit: i64::from(limit),
            offset,
        })
    }

    /// The cursor for the page after this one, when there is one.
    pub fn next_cursor(&self, returned: usize, total: i64) -> Option<String> {
        let next = self.offset + i64::try_from(returned).unwrap_or(0);
        (returned > 0 && next < total).then(|| format!("o{next}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pages_forward_and_refuses_bad_input() {
        let p = Page::new(None, None).unwrap();
        assert_eq!((p.limit, p.offset), (20, 0));
        assert_eq!(p.next_cursor(20, 45).as_deref(), Some("o20"));
        let q = Page::new(Some(20), Some("o40")).unwrap();
        assert_eq!(q.next_cursor(5, 45), None);
        assert!(Page::new(Some(0), None).is_err());
        assert!(Page::new(None, Some("abc")).is_err());
    }
}
