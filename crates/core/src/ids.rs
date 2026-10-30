//! Typed ids for the entities the MVP stores (prefix table: docs/api/conventions.md §9).
//!
//! Each type only accepts its own prefix, so a variant id can never be passed where a product
//! id is expected — the same distinction the database's id CHECKs make.

use crate::define_id;

define_id!(
    /// An operator account (`platform.admin_user`).
    AdminId,
    "adm"
);
define_id!(
    /// A customer (`customer.person`).
    CustomerId,
    "cus"
);
define_id!(
    /// A system actor such as the change-set runner (`sys_runner`).
    SystemActorId,
    "sys"
);
define_id!(
    /// A product as people name it (`catalog.product_group`, schema.org ProductGroup).
    ProductId,
    "prd"
);
define_id!(
    /// A purchasable variant (`catalog.product`, schema.org Product).
    VariantId,
    "var"
);
define_id!(
    /// A price offer for one variant (`catalog.offer`).
    OfferId,
    "off"
);
define_id!(
    /// A brand (`catalog.brand`).
    BrandId,
    "brn"
);
define_id!(
    /// A category (`merchandising.defined_term`).
    CategoryId,
    "cat"
);
define_id!(
    /// A display slot (`merchandising.display_slot`).
    SlotId,
    "slot"
);
define_id!(
    /// Banner content (`merchandising.banner`).
    BannerId,
    "bnr"
);
define_id!(
    /// A display booking (`merchandising.display_booking`).
    BookingId,
    "dsp"
);
define_id!(
    /// A promotion (`promotion.promotion`).
    PromotionId,
    "promo"
);
define_id!(
    /// A member or sale price (`promotion.unit_price_specification`).
    PriceSpecificationId,
    "ups"
);
define_id!(
    /// A coupon definition (`promotion.coupon`).
    CouponId,
    "cpn"
);
define_id!(
    /// A coupon issued to one member (`promotion.member_coupon`).
    MemberCouponId,
    "mcp"
);
define_id!(
    /// A postal address (`customer.postal_address`).
    AddressId,
    "adr"
);
define_id!(
    /// A member program (`customer.member_program`).
    ProgramId,
    "prg"
);
define_id!(
    /// A remembered shopper preference (`customer.preference`).
    MemoryId,
    "mem"
);
define_id!(
    /// A cart (`sales.cart`).
    CartId,
    "cart"
);
define_id!(
    /// A cart line (`sales.cart_line`).
    CartLineId,
    "cln"
);
define_id!(
    /// An order (`sales.sales_order`).
    OrderId,
    "ord"
);
define_id!(
    /// An order item (`sales.order_item`).
    OrderItemId,
    "oln"
);
define_id!(
    /// A payment (`sales.payment`).
    PaymentId,
    "pay"
);
define_id!(
    /// A parcel delivery (`fulfillment.parcel_delivery`).
    DeliveryId,
    "dlv"
);
define_id!(
    /// A cancel, return, or exchange claim (`fulfillment.claim`).
    ClaimId,
    "clm"
);
define_id!(
    /// An inspection (`fulfillment.inspection`).
    InspectionId,
    "ins"
);
define_id!(
    /// A refund (`fulfillment.refund`).
    RefundId,
    "rfd"
);
define_id!(
    /// A message to a customer (`messaging.message`).
    MessageId,
    "msg"
);
define_id!(
    /// A preview (`platform.preview`).
    PreviewId,
    "pv"
);
define_id!(
    /// A change set (`platform.change_set`).
    ChangeSetId,
    "cs"
);
define_id!(
    /// An action log entry (`platform.action_log`).
    ActionId,
    "act"
);
define_id!(
    /// An operator task (`platform.task`).
    TaskId,
    "task"
);
define_id!(
    /// A conversation (`agent.conversation`).
    ConversationId,
    "conv"
);
define_id!(
    /// A turn in a conversation (`agent.turn`).
    TurnId,
    "turn"
);
define_id!(
    /// A request, for tracing across services.
    RequestId,
    "req"
);

/// A member tier id: a word the store chose (`vip`, `gold`). `everyone` is reserved for
/// "no tier restriction" and is not a tier id (docs/api/enums.md §5).
#[derive(
    Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
#[serde(try_from = "String", into = "String")]
pub struct TierId(String);

impl TierId {
    pub const EVERYONE: &'static str = "everyone";

    pub fn parse(s: impl Into<String>) -> Result<Self, crate::IdError> {
        let s = s.into();
        let word = s.bytes().next().is_some_and(|b| b.is_ascii_lowercase())
            && s.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
        if word && s != Self::EVERYONE {
            Ok(Self(s))
        } else {
            Err(crate::IdError::BadSuffix(s))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for TierId {
    type Error = crate::IdError;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        Self::parse(s)
    }
}

impl From<TierId> for String {
    fn from(t: TierId) -> String {
        t.0
    }
}

impl schemars::JsonSchema for TierId {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "TierId".into()
    }

    fn inline_schema() -> bool {
        true
    }

    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "type": "string",
            "pattern": "^[a-z][a-z0-9_]*$",
            "description": "A member tier id the store chose, such as vip. Use everyone for no tier restriction where a field allows it.",
            "examples": ["vip"]
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn product_and_variant_ids_do_not_mix() {
        assert!(ProductId::parse("prd_windbreaker").is_ok());
        assert!(ProductId::parse("var_windbreaker_m_slate").is_err());
        assert!(VariantId::parse("var_windbreaker_m_slate").is_ok());
    }

    #[test]
    fn tier_ids_are_words_but_not_everyone() {
        assert!(TierId::parse("vip").is_ok());
        assert!(TierId::parse("everyone").is_err());
        assert!(TierId::parse("tier_vip").is_ok());
        assert!(TierId::parse("VIP").is_err());
    }
}
