//! `list_wishlist`, `add_to_wishlist`, `remove_from_wishlist`: products a shopper keeps for
//! later (`customer.wishlist_item`). Current prices and availability come from catalog through
//! `service::providers::product_cards`.

use acom_contract::{Basis, Committed, ToolError};
use acom_store::TxCx;
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, json};

use crate::person::customer_not_found;
use crate::repo;

mod add_to_wishlist;
mod list_wishlist;
mod remove_from_wishlist;

pub use add_to_wishlist::*;
pub use list_wishlist::*;
pub use remove_from_wishlist::*;

fn produced(product_id: &str) -> Committed {
    let mut produced = Map::new();
    produced.insert("product_id".into(), json!(product_id));
    Committed {
        id: product_id.to_owned(),
        produced,
    }
}

/// Reads the person (refusing unknown customers) for the basis.
async fn person_basis(tx: &mut TxCx, customer_id: &str) -> Result<Basis, ToolError> {
    let p = repo::person(tx, customer_id)
        .await?
        .ok_or_else(|| customer_not_found(customer_id))?;
    Ok(Basis {
        entity: "customer.person".into(),
        id: p.id,
        version: i64::from(p.version),
    })
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WishlistItemInput {
    /// The member; a shopper uses `me`.
    #[schemars(regex(pattern = r"^(me|cus_[a-z0-9_]+)$"), example = &"me")]
    pub customer_id: String,
    /// The product (`search_products`, `get_product`).
    #[schemars(regex(pattern = r"^prd_[a-z0-9_]+$"), example = &"prd_field_jacket")]
    pub product_id: String,
}

/// One wishlist row to add or take away.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WishlistPlan {
    pub customer_id: String,
    pub product_id: String,
    /// When it was (or will be) added; kept so undo puts it back as it was.
    pub added_at: DateTime<Utc>,
    /// `false` when there is nothing to do (already kept, or already gone).
    pub changes_anything: bool,
}

#[cfg(test)]
mod tests {
    use acom_contract::WriteOp;

    use super::*;

    /// Integration: needs a seeded database (`DATABASE_URL`). Run with `-- --ignored`.
    #[tokio::test]
    #[ignore = "integration: needs a PostgreSQL database"]
    async fn keeps_and_drops_a_product() {
        let cx = crate::testdb::cx("cus_jamie").await;
        let input = || WishlistItemInput {
            customer_id: "me".into(),
            product_id: "prd_field_jacket".into(),
        };
        let add = AddToWishlist.preview(&cx, input()).await.unwrap();
        assert_eq!(add.changes.len(), 1);
        let mut tx = cx.begin().await.unwrap();
        AddToWishlist.commit(&mut tx, &add.plan).await.unwrap();
        AddToWishlist.commit(&mut tx, &add.plan).await.unwrap();
        let (total, rows) = repo::wishlist(&mut tx, "cus_jamie", 20, 0).await.unwrap();
        assert_eq!(total, 1);
        assert_eq!(rows[0].product_group_id, "prd_field_jacket");
        let remove = WishlistPlan {
            changes_anything: true,
            ..add.plan
        };
        RemoveFromWishlist.commit(&mut tx, &remove).await.unwrap();
        assert!(
            repo::wishlist_item(&mut tx, "cus_jamie", "prd_field_jacket")
                .await
                .unwrap()
                .is_none()
        );
    }
}
