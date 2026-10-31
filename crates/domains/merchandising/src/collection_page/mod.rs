//! The `collection_page` resource (docs/api/merchandising.md#collection_page): curated pages
//! (`CollectionPage`, `col_`) of products in sections. What its operations share.
//!
//! Curated pages have no table yet (`migrations/0006_merchandising.sql` stops at categories,
//! slots, banners and bookings), so the store has none: the list is empty and every id is
//! unknown. The reads keep the spec's inputs and shapes so callers stay the same when the table
//! arrives.

pub mod get_curation;
pub mod list_curations;

use acom_contract::{ErrorClass, ErrorCode};

pub const COLLECTION_PAGE_NOT_FOUND: ErrorCode =
    ErrorCode::new("collection_page_not_found", ErrorClass::NotFound);
