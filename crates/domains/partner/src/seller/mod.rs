//! `partner.seller`: the businesses that sell in the store (schema.org `Organization`), from
//! application through review and activation to suspension or termination. What the seller
//! operations share: the address, contact and document inputs, and a seller as the tools
//! return it (sensitive fields masked for store operators).

pub mod get_seller;
pub mod onboard_seller;
pub mod search_sellers;
pub mod set_seller_status;
pub mod update_seller;

pub use get_seller::GetSeller;
pub use onboard_seller::OnboardSeller;
pub use search_sellers::SearchSellers;
pub use set_seller_status::SetSellerStatus;
pub use update_seller::UpdateSeller;

use acom_core::enums::SellerDocumentType;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::model;
use crate::repo::SellerRow;
use crate::rules::Rules;
use crate::viewer::Viewer;

/// Who a contact point is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ContactType {
    /// Payouts, invoices and the settlement statement.
    Settlement,
    /// Shopper questions, claims and returns.
    CustomerService,
    /// Listings, product data and stock.
    Product,
}

/// One person or team the store reaches at the seller (schema.org `ContactPoint`).
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ContactPointInput {
    /// What they handle: settlement (payouts), customer_service (shopper questions and claims) or
    /// product (listings and stock).
    #[schemars(example = &"settlement")]
    pub contact_type: ContactType,
    /// Name of the person or team.
    #[schemars(length(min = 1, max = 100), example = &"Nadia Fenwick")]
    pub name: String,
    /// Email address.
    #[schemars(length(min = 3, max = 200), example = &"accounts@harborgoods.example")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    /// Phone number with its country code.
    #[schemars(length(min = 3, max = 40), example = &"+1-401-555-0163")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub telephone: Option<String>,
}

/// A business address (schema.org `PostalAddress`).
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PostalAddressInput {
    /// Street, number and suite.
    #[schemars(length(min = 1, max = 200), example = &"75 Wharf St")]
    pub street_address: String,
    /// City.
    #[schemars(length(min = 1, max = 100), example = &"Providence")]
    pub address_locality: String,
    /// State or region.
    #[schemars(length(max = 100), example = &"RI")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub address_region: Option<String>,
    /// Postal code.
    #[schemars(length(min = 1, max = 20), example = &"02903")]
    pub postal_code: String,
    /// ISO 3166-1 country code.
    #[schemars(regex(pattern = r"^[A-Z]{2}$"), example = &"US")]
    pub address_country: String,
}

/// A paper attached to an application (schema.org `MediaObject`), uploaded first.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DocumentInput {
    /// What the paper is: business_registration (business registration certificate),
    /// payout_account_proof (bank letter for the payout account) or other.
    #[schemars(example = &"business_registration")]
    pub document_type: SellerDocumentType,
    /// The upload's id, from the chat attachment or `POST /api/uploads`.
    #[schemars(regex(pattern = r"^upl_[a-z0-9_]+$"), example = &"upl_harbor_registration")]
    pub upload_id: String,
    /// The file name as uploaded.
    #[schemars(length(min = 1, max = 200), example = &"harbor-goods-registration.pdf")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// A seller's tax id as this viewer may see it.
pub(crate) fn tax_id_shown(s: &SellerRow, viewer: &Viewer, rules: &Rules) -> String {
    if rules.masks("tax_id") && !viewer.sees_unmasked(&s.id) {
        model::mask(&s.tax_id, rules.visible_trailing)
    } else {
        s.tax_id.clone()
    }
}

/// A payout account reference as this viewer may see it.
pub(crate) fn payout_shown(
    seller_id: &str,
    reference: &str,
    viewer: &Viewer,
    rules: &Rules,
) -> String {
    if rules.masks("payout_account") && !viewer.sees_unmasked(seller_id) {
        model::mask(reference, rules.visible_trailing)
    } else {
        reference.to_owned()
    }
}

/// The short form `search_sellers` lists.
pub(crate) fn seller_item(
    s: &SellerRow,
    grade: Option<&str>,
    viewer: &Viewer,
    rules: &Rules,
) -> Value {
    json!({
        "id": s.id,
        "legal_name": s.legal_name,
        "tax_id": tax_id_shown(s, viewer, rules),
        "acom:seller_status": s.seller_status,
        "acom:seller_type": s.seller_type,
        "acom:grade_level": grade,
    })
}

/// The address, contacts, categories and documents as the profile shows them.
pub(crate) fn profile_wire(s: &SellerRow) -> Value {
    json!({
        "address": s.address,
        "contact_point": s.contact_point,
        "acom:business_category": s.business_category,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::tests::rules;

    fn row() -> SellerRow {
        SellerRow {
            id: "slr_harbor_goods".into(),
            legal_name: "Harbor Goods LLC".into(),
            tax_id: "512-73-4410".into(),
            seller_status: "active".into(),
            seller_type: "marketplace".into(),
            address: json!({}),
            contact_point: json!([]),
            business_category: vec![],
            documents: json!([]),
            organization_id: None,
            applied_at: chrono::DateTime::<chrono::Utc>::UNIX_EPOCH,
            version: 1,
        }
    }

    #[test]
    fn operators_see_the_tax_id_masked_and_the_seller_sees_its_own() {
        let r = rules();
        let s = row();
        let item = seller_item(&s, Some("gold"), &Viewer::Store, &r);
        assert_eq!(item["tax_id"], "***-**-4410");
        assert_eq!(item["acom:grade_level"], "gold");
        assert_eq!(
            tax_id_shown(&s, &Viewer::Seller("slr_harbor_goods".into()), &r),
            "512-73-4410"
        );
        assert_eq!(
            tax_id_shown(&s, &Viewer::Seller("slr_other".into()), &r),
            "***-**-4410"
        );
        assert_eq!(
            payout_shown("slr_harbor_goods", "pa_harbor_4410", &Viewer::Store, &r),
            "**********4410"
        );
    }
}
