//! `list_wishlist`, `add_to_wishlist`, `remove_from_wishlist`: products a shopper keeps for
//! later (`customer.wishlist_item`). Current prices and availability come from catalog through
//! `service::providers::product_cards`.

use acom_contract::{
    ActionType, Audience, Basis, Change, ChangeOp, Committed, Group, Kind, Method, OpMeta,
    Operation, Planned, ReadOp, Slot, ToolError, UndoSupport, Verification, WriteOp,
};
use acom_store::{Cx, Page, TxCx};
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::access::{approval, customer_for};
use crate::person::customer_not_found;
use crate::service::providers;
use crate::{CUSTOMER_NOT_FOUND, INVALID_FILTER, PRODUCT_NOT_FOUND, repo};

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

// ── list_wishlist ───────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListWishlistInput {
    /// The member; a shopper uses `me`.
    #[schemars(regex(pattern = r"^(me|cus_[a-z0-9_]+)$"), example = &"me")]
    pub customer_id: String,
    /// Only products in this category. Categories are not available here yet; the call is
    /// refused.
    #[schemars(regex(pattern = r"^cat_[a-z0-9_]+$"), example = &"cat_summer_outerwear")]
    pub category_id: Option<String>,
    /// Page size, 1–100. Default 20.
    #[schemars(range(min = 1, max = 100), example = 20)]
    pub limit: Option<u32>,
    /// `next_cursor` from the previous page.
    #[schemars(example = &"o20")]
    pub cursor: Option<String>,
}

pub struct ListWishlist;

impl Operation for ListWishlist {
    const META: OpMeta = OpMeta {
        address: "customer.wishlist.list",
        tool: "list_wishlist",
        group: Group::Engage,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/customer/people/{customer_id}/wishlist",
        summary: "The products a member keeps for later, newest first, with today's price and availability.",
        consumes: &[Slot::one("customer_id")],
        produces: &[Slot::many("product_id")],
        checks: &[],
        errors: &[CUSTOMER_NOT_FOUND, INVALID_FILTER],
        audience: Audience::Both,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = ListWishlistInput;
    type Output = Value;
}

impl ReadOp<Cx> for ListWishlist {
    async fn read(&self, cx: &Cx, input: ListWishlistInput) -> Result<Value, ToolError> {
        let customer_id = customer_for(cx, &input.customer_id)?;
        if input.category_id.is_some() {
            return Err(ToolError::new(
                INVALID_FILTER,
                "The wishlist cannot be filtered by category yet; list it without `category_id`.",
            )
            .detail(json!({"field": "category_id", "reason": "unsupported"}))
            .next("list_wishlist"));
        }
        let page = Page::new(input.limit, input.cursor.as_deref())?;
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        if repo::person(&mut tx, &customer_id).await?.is_none() {
            return Err(customer_not_found(&customer_id));
        }
        let (total, rows) = repo::wishlist(&mut tx, &customer_id, page.limit, page.offset).await?;
        let ids: Vec<String> = rows.iter().map(|r| r.product_group_id.clone()).collect();
        let cards = match providers::product_cards() {
            Some(read) => Some(read(&mut tx, &ids).await?),
            None => None,
        };
        tx.commit().await.map_err(ToolError::from)?;
        let items: Vec<Value> = rows
            .iter()
            .map(|r| {
                let mut item = json!({"product_id": r.product_group_id,
                                      "added_at": cx.time.format(r.created_at)});
                let card = cards
                    .as_ref()
                    .and_then(|c| c.iter().find(|c| c.product_id == r.product_group_id));
                if let Some(c) = card {
                    item["name"] = json!(c.name);
                    item["price"] = json!(c.price)["value"].clone();
                    item["price_currency"] = json!(c.price.currency);
                    item["availability"] = json!(c.availability);
                }
                item
            })
            .collect();
        let mut out = json!({"items": items, "total": total});
        if let Some(c) = page.next_cursor(rows.len(), total) {
            out["next_cursor"] = json!(c);
        }
        Ok(out)
    }
}

// ── add_to_wishlist ─────────────────────────────────────────────────────────────────────

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

pub struct AddToWishlist;

impl Operation for AddToWishlist {
    const META: OpMeta = OpMeta {
        address: "customer.wishlist.add",
        tool: "add_to_wishlist",
        group: Group::Engage,
        kind: Kind::Write,
        method: Method::Post,
        path: "/v1/customer/people/{customer_id}/wishlist",
        summary: "Keep a product on the shopper's wishlist. Already there is fine.",
        consumes: &[Slot::one("product_id")],
        produces: &[Slot::one("product_id")],
        checks: &[],
        errors: &[CUSTOMER_NOT_FOUND, PRODUCT_NOT_FOUND],
        audience: Audience::Both,
        action_type: ActionType::ADD,
        person_scope: "",
    };
    type Input = WishlistItemInput;
    type Output = Value;
}

impl WriteOp<Cx> for AddToWishlist {
    type Plan = WishlistPlan;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(
        &self,
        cx: &Cx,
        input: WishlistItemInput,
    ) -> Result<Planned<WishlistPlan>, ToolError> {
        let customer_id = customer_for(cx, &input.customer_id)?;
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let basis = person_basis(&mut tx, &customer_id).await?;
        // Without the catalog reader the foreign key still refuses unknown products at commit.
        if let Some(read) = providers::product_cards() {
            let found = read(&mut tx, std::slice::from_ref(&input.product_id)).await?;
            if found.is_empty() {
                return Err(ToolError::new(
                    PRODUCT_NOT_FOUND,
                    format!("There is no product {}.", input.product_id),
                )
                .detail(json!({"entity": "product", "id": input.product_id}))
                .next("search_products"));
            }
        }
        let existing = repo::wishlist_item(&mut tx, &customer_id, &input.product_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let changes = match &existing {
            Some(_) => vec![],
            None => vec![Change {
                op: ChangeOp::Create,
                entity: "wishlist_item".into(),
                id: String::new(),
                from: Value::Null,
                to: json!({"product_id": input.product_id}),
                note: None,
            }],
        };
        let facts = json!({"customer_id": customer_id});
        Ok(Planned {
            plan: WishlistPlan {
                customer_id,
                product_id: input.product_id,
                added_at: existing
                    .as_ref()
                    .map_or_else(|| cx.clock.now(), |e| e.created_at),
                changes_anything: existing.is_none(),
            },
            changes,
            checks: vec![],
            basis: vec![basis],
            approval: approval(cx, Self::META.tool, &facts),
        })
    }

    async fn commit(&self, tx: &mut TxCx, plan: &WishlistPlan) -> Result<Committed, ToolError> {
        if plan.changes_anything {
            repo::insert_wishlist_item(tx, &plan.customer_id, &plan.product_id, plan.added_at)
                .await?;
        }
        Ok(produced(&plan.product_id))
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &WishlistPlan,
        _done: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let row = repo::wishlist_item(&mut tx, &plan.customer_id, &plan.product_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let mismatches = match row {
            Some(_) => vec![],
            None => vec![format!("{} is not on the wishlist", plan.product_id)],
        };
        Ok(Verification {
            matched: mismatches.is_empty(),
            mismatches,
        })
    }

    async fn undo(
        &self,
        tx: &mut TxCx,
        plan: &WishlistPlan,
        _done: &Committed,
    ) -> Result<(), ToolError> {
        if plan.changes_anything {
            repo::delete_wishlist_item(tx, &plan.customer_id, &plan.product_id).await?;
        }
        Ok(())
    }
}

// ── remove_from_wishlist ────────────────────────────────────────────────────────────────

pub struct RemoveFromWishlist;

impl Operation for RemoveFromWishlist {
    const META: OpMeta = OpMeta {
        address: "customer.wishlist.remove",
        tool: "remove_from_wishlist",
        group: Group::Engage,
        kind: Kind::Write,
        method: Method::Delete,
        path: "/v1/customer/people/{customer_id}/wishlist/{product_id}",
        summary: "Take a product off the shopper's wishlist. Not there is fine.",
        consumes: &[Slot::one("product_id")],
        produces: &[Slot::one("product_id")],
        checks: &[],
        errors: &[CUSTOMER_NOT_FOUND],
        audience: Audience::Both,
        action_type: ActionType::DELETE,
        person_scope: "",
    };
    type Input = WishlistItemInput;
    type Output = Value;
}

impl WriteOp<Cx> for RemoveFromWishlist {
    type Plan = WishlistPlan;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(
        &self,
        cx: &Cx,
        input: WishlistItemInput,
    ) -> Result<Planned<WishlistPlan>, ToolError> {
        let customer_id = customer_for(cx, &input.customer_id)?;
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let basis = person_basis(&mut tx, &customer_id).await?;
        let existing = repo::wishlist_item(&mut tx, &customer_id, &input.product_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let changes = match &existing {
            Some(_) => vec![Change {
                op: ChangeOp::Delete,
                entity: "wishlist_item".into(),
                id: input.product_id.clone(),
                from: json!({"product_id": input.product_id}),
                to: Value::Null,
                note: None,
            }],
            None => vec![],
        };
        let facts = json!({"customer_id": customer_id});
        Ok(Planned {
            plan: WishlistPlan {
                customer_id,
                product_id: input.product_id,
                added_at: existing
                    .as_ref()
                    .map_or_else(|| cx.clock.now(), |e| e.created_at),
                changes_anything: existing.is_some(),
            },
            changes,
            checks: vec![],
            basis: vec![basis],
            approval: approval(cx, Self::META.tool, &facts),
        })
    }

    async fn commit(&self, tx: &mut TxCx, plan: &WishlistPlan) -> Result<Committed, ToolError> {
        if plan.changes_anything {
            repo::delete_wishlist_item(tx, &plan.customer_id, &plan.product_id).await?;
        }
        Ok(produced(&plan.product_id))
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &WishlistPlan,
        _done: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let row = repo::wishlist_item(&mut tx, &plan.customer_id, &plan.product_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let mismatches = match row {
            Some(_) => vec![format!("{} is still on the wishlist", plan.product_id)],
            None => vec![],
        };
        Ok(Verification {
            matched: mismatches.is_empty(),
            mismatches,
        })
    }

    async fn undo(
        &self,
        tx: &mut TxCx,
        plan: &WishlistPlan,
        _done: &Committed,
    ) -> Result<(), ToolError> {
        if plan.changes_anything {
            repo::insert_wishlist_item(tx, &plan.customer_id, &plan.product_id, plan.added_at)
                .await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use acom_contract::schema::{closed_schema_for, violations};

    use super::*;

    #[test]
    fn inputs_are_closed_and_described() {
        for s in [
            closed_schema_for::<ListWishlistInput>(),
            closed_schema_for::<WishlistItemInput>(),
        ] {
            assert_eq!(violations(&s), Vec::<String>::new(), "{s:#}");
        }
        assert_eq!(
            RemoveFromWishlist::META.path,
            "/v1/customer/people/{customer_id}/wishlist/{product_id}"
        );
    }

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
