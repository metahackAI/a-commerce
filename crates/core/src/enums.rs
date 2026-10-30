//! Word enumerations shared by the API, the database, and policies.
//!
//! The lists come from `docs/api/enums.md`. Each enum has a name (`order_status`) that
//! database column comments cite as `values: order_status`, and `cargo xtask check-schema`
//! checks that every such column's `CHECK (... IN (...))` list equals the list here.

use std::fmt;
use std::str::FromStr;

/// A value that is not in the enumeration.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("`{value}` is not a valid {name} (expected one of: {expected})")]
pub struct UnknownWord {
    pub name: &'static str,
    pub value: String,
    pub expected: String,
}

/// Defines a word enum: snake_case serde, `as_str`, `FromStr`, `ALL`, and a closed JSON Schema.
macro_rules! word_enum {
    (
        $(#[$meta:meta])*
        $name:ident = $list:literal {
            $( $(#[$vmeta:meta])* $variant:ident = $word:literal ),+ $(,)?
        }
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub enum $name {
            $( $(#[$vmeta])* $variant ),+
        }

        impl $name {
            /// Name of this list, as cited in database comments (`values: …`).
            pub const LIST: &'static str = $list;
            pub const ALL: &'static [$name] = &[ $( $name::$variant ),+ ];
            pub const WORDS: &'static [&'static str] = &[ $( $word ),+ ];

            pub const fn as_str(self) -> &'static str {
                match self { $( $name::$variant => $word ),+ }
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl FromStr for $name {
            type Err = UnknownWord;
            fn from_str(s: &str) -> Result<Self, Self::Err> {
                match s {
                    $( $word => Ok($name::$variant), )+
                    _ => Err(UnknownWord {
                        name: $list,
                        value: s.to_owned(),
                        expected: Self::WORDS.join(", "),
                    }),
                }
            }
        }

        impl serde::Serialize for $name {
            fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                s.serialize_str(self.as_str())
            }
        }

        impl<'de> serde::Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                let s = <String as serde::Deserialize>::deserialize(d)?;
                s.parse().map_err(serde::de::Error::custom)
            }
        }

        impl schemars::JsonSchema for $name {
            fn schema_name() -> std::borrow::Cow<'static, str> {
                stringify!($name).into()
            }

            fn inline_schema() -> bool {
                true
            }

            fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
                schemars::json_schema!({ "type": "string", "enum": Self::WORDS })
            }
        }
    };
}

word_enum! {
    /// schema.org `OrderStatus`, for orders and order items.
    OrderStatus = "order_status" {
        PaymentDue = "payment_due", Processing = "processing", InTransit = "in_transit",
        Delivered = "delivered", Returned = "returned", Cancelled = "cancelled", Problem = "problem",
    }
}

word_enum! {
    /// acom:fulfillmentStage, the warehouse step of an order item or delivery.
    FulfillmentStage = "fulfillment_stage" {
        AwaitingStock = "awaiting_stock", ShipInstructed = "ship_instructed", Packed = "packed", Shipped = "shipped",
    }
}

word_enum! {
    /// acom:deliveryState of a ParcelDelivery.
    DeliveryState = "delivery_state" {
        AwaitingStock = "awaiting_stock", ShipInstructed = "ship_instructed", Packed = "packed",
        LabelCreated = "label_created", PickedUp = "picked_up", InTransit = "in_transit",
        Delivered = "delivered", DeliveryFailed = "delivery_failed", Cancelled = "cancelled",
    }
}

word_enum! {
    /// schema.org `PaymentStatusType`.
    PaymentStatus = "payment_status" {
        PaymentDue = "payment_due", PaymentComplete = "payment_complete", PaymentDeclined = "payment_declined",
        PaymentPastDue = "payment_past_due", PaymentAutomaticallyApplied = "payment_automatically_applied",
    }
}

word_enum! {
    /// acom:claimStatus, shared by cancel, return, and exchange claims.
    ClaimStatus = "claim_status" {
        ChargeDue = "charge_due", Requested = "requested", PhotosRequested = "photos_requested",
        Approved = "approved", PickupInstructed = "pickup_instructed", PickedUp = "picked_up",
        Inspected = "inspected", ReshipRequested = "reship_requested", Completed = "completed",
        Declined = "declined", Withdrawn = "withdrawn", Converted = "converted",
    }
}

word_enum! {
    /// acom:claimType.
    ClaimType = "claim_type" { Cancel = "cancel", Return = "return", Exchange = "exchange" }
}

word_enum! {
    /// acom:claimPath, who decides a claim.
    ClaimPath = "claim_path" { SelfService = "self_service", StoreReview = "store_review", NotAllowed = "not_allowed" }
}

word_enum! {
    /// acom:claimReason for returns and exchanges.
    ClaimReason = "claim_reason" {
        Defect = "defect", DamagedInTransit = "damaged_in_transit", NotAsDescribed = "not_as_described",
        WrongItemSent = "wrong_item_sent", WrongOptionSent = "wrong_option_sent", MissingParts = "missing_parts",
        WrongSize = "wrong_size", DifferentOptionWanted = "different_option_wanted", ChangedMind = "changed_mind",
    }
}

word_enum! {
    /// acom:cancelReason.
    CancelReason = "cancel_reason" {
        ChangedMind = "changed_mind", WrongOptionOrdered = "wrong_option_ordered", FoundLowerPrice = "found_lower_price",
        DiscountChanged = "discount_changed", PaymentMethodChange = "payment_method_change",
        DuplicateOrder = "duplicate_order", PaymentNotCompleted = "payment_not_completed",
        ShippingDelay = "shipping_delay", OutOfStock = "out_of_stock", FreeGiftUnavailable = "free_gift_unavailable",
        ProductInfoInaccurate = "product_info_inaccurate", SellerCancelled = "seller_cancelled",
        StoreCancelled = "store_cancelled",
    }
}

word_enum! {
    /// acom:liability.
    Liability = "liability" {
        StoreFault = "store_fault", SellerFault = "seller_fault", CustomerFault = "customer_fault",
        CarrierFault = "carrier_fault", None = "none",
    }
}

word_enum! {
    /// acom:inspectionCondition of a returned item.
    InspectionCondition = "inspection_condition" { AsNew = "as_new", Damaged = "damaged", MissingParts = "missing_parts" }
}

word_enum! {
    /// acom:refundStatus.
    RefundStatus = "refund_status" { Requested = "requested", Completed = "completed", Failed = "failed" }
}

word_enum! {
    /// acom:refundMethod.
    RefundMethod = "refund_method" {
        OriginalPayment = "original_payment", BankAccount = "bank_account", StoreCredit = "store_credit",
    }
}

word_enum! {
    /// schema.org `RefundTypeEnumeration`.
    RefundType = "refund_type" {
        FullRefund = "full_refund", ExchangeRefund = "exchange_refund", StoreCreditRefund = "store_credit_refund",
    }
}

word_enum! {
    /// schema.org `ActionStatusType`.
    ActionStatus = "action_status" {
        Potential = "potential", Active = "active", Completed = "completed", Failed = "failed",
    }
}

word_enum! {
    /// acom:changeSetState.
    ChangeSetState = "change_set_state" {
        Drafting = "drafting", WaitingForApproval = "waiting_for_approval", Running = "running",
        Verifying = "verifying", Live = "live", Failed = "failed", Rejected = "rejected", Undone = "undone",
    }
}

word_enum! {
    /// acom:previewState.
    PreviewState = "preview_state" {
        Open = "open", Committed = "committed", Expired = "expired", Superseded = "superseded",
    }
}

word_enum! {
    /// acom:approvalStatus of a product.
    ApprovalStatus = "approval_status" {
        Draft = "draft", PendingReview = "pending_review", Active = "active", Rejected = "rejected",
        Discontinued = "discontinued",
    }
}

word_enum! {
    /// schema.org `ItemAvailability`.
    Availability = "availability" {
        InStock = "in_stock", OutOfStock = "out_of_stock", PreOrder = "pre_order", BackOrder = "back_order",
        Discontinued = "discontinued",
    }
}

word_enum! {
    /// schema.org `PriceTypeEnumeration`.
    PriceType = "price_type" { ListPrice = "list_price", SalePrice = "sale_price" }
}

word_enum! {
    /// acom:saleType of a product.
    SaleType = "sale_type" {
        Always = "always", PreOrder = "pre_order", Subscription = "subscription", GiftCard = "gift_card",
    }
}

word_enum! {
    /// acom:promotionStatus.
    PromotionStatus = "promotion_status" {
        Draft = "draft", Scheduled = "scheduled", Running = "running", Paused = "paused", Ended = "ended",
    }
}

word_enum! {
    /// acom:couponStatus of an issued (member) coupon.
    CouponStatus = "coupon_status" {
        Issued = "issued", Available = "available", Used = "used", Restored = "restored",
        Expired = "expired", Revoked = "revoked",
    }
}

word_enum! {
    /// acom:couponApplication, how a coupon reaches an order.
    CouponApplication = "coupon_application" {
        AutomaticAtCheckout = "automatic_at_checkout", CodeEntry = "code_entry", Download = "download",
    }
}

word_enum! {
    /// acom:discountLayer, where a discount applies.
    DiscountLayer = "discount_layer" { ItemPrice = "item_price", OrderDiscount = "order_discount", AddOn = "add_on" }
}

word_enum! {
    /// acom:displayStatus of a display booking or curated page.
    DisplayStatus = "display_status" { Draft = "draft", Scheduled = "scheduled", Active = "active", Ended = "ended" }
}

word_enum! {
    /// acom:slotType of a display slot.
    SlotType = "slot_type" {
        Hero = "hero", BannerBoard = "banner_board", ProductGrid = "product_grid", BrandStrip = "brand_strip",
        CollectionFeed = "collection_feed",
    }
}

word_enum! {
    /// acom:termSet, which set a category belongs to.
    TermSet = "term_set" { Standard = "standard", Display = "display" }
}

word_enum! {
    /// acom:memberState of a person.
    MemberState = "member_state" {
        Active = "active", Dormant = "dormant", Blocked = "blocked", PendingWithdrawal = "pending_withdrawal",
        Withdrawn = "withdrawn",
    }
}

word_enum! {
    /// acom:memberType.
    MemberType = "member_type" { Individual = "individual", Business = "business" }
}

word_enum! {
    /// acom:messageChannel.
    MessageChannel = "message_channel" { Email = "email", Sms = "sms", Push = "push", InApp = "in_app" }
}

word_enum! {
    /// acom:messageKind of a customer message.
    MessageKind = "message_kind" {
        AskForPhotos = "ask_for_photos", Decline = "decline", RefundDone = "refund_done", Custom = "custom",
    }
}

word_enum! {
    /// acom:salesChannel.
    SalesChannel = "sales_channel" { Web = "web", MobileWeb = "mobile_web", App = "app", Agent = "agent" }
}

word_enum! {
    /// acom:paymentMethod.
    PaymentMethod = "payment_method" {
        CreditCard = "credit_card", BankTransfer = "bank_transfer", VirtualAccount = "virtual_account",
        LoyaltyPoints = "loyalty_points", StoreCredit = "store_credit",
    }
}

word_enum! {
    /// schema.org `ReturnFeesEnumeration`.
    ReturnFees = "return_fees" {
        FreeReturn = "free_return", ReturnFeesCustomerResponsibility = "return_fees_customer_responsibility",
        ReturnShippingFees = "return_shipping_fees",
    }
}

word_enum! {
    /// acom:taskState of an operator task.
    TaskState = "task_state" { NeedsYou = "needs_you", Working = "working", Done = "done", Dismissed = "dismissed" }
}

word_enum! {
    /// acom:taskOrigin, how a task started.
    TaskOrigin = "task_origin" { YouAsked = "you_asked", AgentNoticed = "agent_noticed", Automation = "automation" }
}

word_enum! {
    /// acom:addressUsage.
    AddressUsage = "address_usage" { Shipping = "shipping", Billing = "billing", Return = "return" }
}

word_enum! {
    /// acom:preferenceKey, what a remembered shopper preference is about.
    PreferenceKey = "preference_key" {
        Size = "size", FitNote = "fit_note", FabricPreference = "fabric_preference",
        ComparesSpecs = "compares_specs", LayoutPreference = "layout_preference",
        TravelPriority = "travel_priority", Other = "other",
    }
}

word_enum! {
    /// acom:actorKind, who performed an action (`agent` in an action record).
    ActorKind = "actor_kind" {
        Operator = "operator", Shopper = "shopper", AgentOnBehalf = "agent_on_behalf",
        ExternalAgent = "external_agent", System = "system",
    }
}

word_enum! {
    /// acom:adminStatus of an operator account.
    AdminStatus = "admin_status" { Active = "active", Suspended = "suspended", Locked = "locked" }
}

word_enum! {
    /// schema:creativeWorkStatus of a review: whether shoppers see it.
    ReviewStatus = "review_status" { Visible = "visible", Hidden = "hidden", Reported = "reported" }
}

word_enum! {
    /// acom:fitVote, how a reviewer says the size fits.
    FitVote = "fit_vote" { TrueToSize = "true_to_size", RunsSmall = "runs_small", RunsLarge = "runs_large" }
}

word_enum! {
    /// acom:measurementName, which body or garment measurement a value is.
    MeasurementName = "measurement_name" {
        Chest = "chest", Shoulder = "shoulder", Sleeve = "sleeve", Length = "length", Waist = "waist",
        Hip = "hip", Inseam = "inseam", Rise = "rise", FootLength = "foot_length",
    }
}

word_enum! {
    /// acom:couponStacking, which other benefits a coupon combines with.
    CouponStacking = "coupon_stacking" {
        NoTierCoupons = "no_tier_coupons", WithTierCoupons = "with_tier_coupons", NoOtherCoupons = "no_other_coupons",
    }
}

word_enum! {
    /// acom:chatSurface, where a conversation happens.
    ChatSurface = "chat_surface" { Bo = "bo", Fo = "fo" }
}

word_enum! {
    /// acom:stepKind of an agent step.
    StepKind = "step_kind" {
        Route = "route", SearchTools = "search_tools", Plan = "plan", CallTool = "call_tool", Check = "check",
    }
}

/// Every word list, by name. `cargo xtask check-schema` compares database `CHECK` lists with these.
pub const LISTS: &[(&str, &[&str])] = &[
    (OrderStatus::LIST, OrderStatus::WORDS),
    (FulfillmentStage::LIST, FulfillmentStage::WORDS),
    (DeliveryState::LIST, DeliveryState::WORDS),
    (PaymentStatus::LIST, PaymentStatus::WORDS),
    (ClaimStatus::LIST, ClaimStatus::WORDS),
    (ClaimType::LIST, ClaimType::WORDS),
    (ClaimPath::LIST, ClaimPath::WORDS),
    (ClaimReason::LIST, ClaimReason::WORDS),
    (CancelReason::LIST, CancelReason::WORDS),
    (Liability::LIST, Liability::WORDS),
    (InspectionCondition::LIST, InspectionCondition::WORDS),
    (RefundStatus::LIST, RefundStatus::WORDS),
    (RefundMethod::LIST, RefundMethod::WORDS),
    (RefundType::LIST, RefundType::WORDS),
    (ActionStatus::LIST, ActionStatus::WORDS),
    (ChangeSetState::LIST, ChangeSetState::WORDS),
    (PreviewState::LIST, PreviewState::WORDS),
    (ApprovalStatus::LIST, ApprovalStatus::WORDS),
    (Availability::LIST, Availability::WORDS),
    (PriceType::LIST, PriceType::WORDS),
    (SaleType::LIST, SaleType::WORDS),
    (PromotionStatus::LIST, PromotionStatus::WORDS),
    (CouponStatus::LIST, CouponStatus::WORDS),
    (CouponApplication::LIST, CouponApplication::WORDS),
    (DiscountLayer::LIST, DiscountLayer::WORDS),
    (DisplayStatus::LIST, DisplayStatus::WORDS),
    (SlotType::LIST, SlotType::WORDS),
    (TermSet::LIST, TermSet::WORDS),
    (MemberState::LIST, MemberState::WORDS),
    (MemberType::LIST, MemberType::WORDS),
    (MessageChannel::LIST, MessageChannel::WORDS),
    (MessageKind::LIST, MessageKind::WORDS),
    (SalesChannel::LIST, SalesChannel::WORDS),
    (PaymentMethod::LIST, PaymentMethod::WORDS),
    (ReturnFees::LIST, ReturnFees::WORDS),
    (TaskState::LIST, TaskState::WORDS),
    (TaskOrigin::LIST, TaskOrigin::WORDS),
    (AddressUsage::LIST, AddressUsage::WORDS),
    (PreferenceKey::LIST, PreferenceKey::WORDS),
    (ActorKind::LIST, ActorKind::WORDS),
    (AdminStatus::LIST, AdminStatus::WORDS),
    (ReviewStatus::LIST, ReviewStatus::WORDS),
    (FitVote::LIST, FitVote::WORDS),
    (MeasurementName::LIST, MeasurementName::WORDS),
    (CouponStacking::LIST, CouponStacking::WORDS),
    (ChatSurface::LIST, ChatSurface::WORDS),
    (StepKind::LIST, StepKind::WORDS),
];

/// The words of one list, by name.
pub fn words(list: &str) -> Option<&'static [&'static str]> {
    LISTS
        .iter()
        .find(|(name, _)| *name == list)
        .map(|(_, words)| *words)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_words() {
        for status in OrderStatus::ALL {
            assert_eq!(status.as_str().parse::<OrderStatus>().unwrap(), *status);
        }
        assert_eq!(
            serde_json::to_string(&ClaimStatus::PhotosRequested).unwrap(),
            "\"photos_requested\""
        );
        let parsed: ChangeSetState = serde_json::from_str("\"waiting_for_approval\"").unwrap();
        assert_eq!(parsed, ChangeSetState::WaitingForApproval);
    }

    #[test]
    fn rejects_codes_and_old_spellings() {
        let e = "paid".parse::<PaymentStatus>().unwrap_err();
        assert!(e.expected.contains("payment_complete"));
        assert!("waiting".parse::<ClaimStatus>().is_err());
    }

    #[test]
    fn lists_are_unique_and_snake_case() {
        let mut names: Vec<&str> = LISTS.iter().map(|(n, _)| *n).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), LISTS.len());
        for (_, words) in LISTS {
            for w in *words {
                assert!(
                    w.bytes().all(|b| b.is_ascii_lowercase() || b == b'_'),
                    "{w}"
                );
            }
        }
        assert_eq!(words("term_set"), Some(&["standard", "display"][..]));
    }
}
