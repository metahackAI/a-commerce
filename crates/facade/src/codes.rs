//! The code tables behind the screens.
//!
//! Callers see only the codes. What each code means lives here, as it would in a code table
//! only the screen team knows; no schema or description carries it. Input codes are enums
//! without variant docs, so their schema is a bare `enum` list.

use serde::Deserialize;
use serde_json::Value;

/// A code enum with its back-end word for each code, both ways.
macro_rules! codes {
    ($name:ident { $($variant:ident = $code:literal => $word:literal),+ $(,)? }) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, schemars::JsonSchema)]
        pub enum $name {
            $( #[serde(rename = $code)] $variant ),+
        }

        impl $name {
            /// The back end's word.
            #[allow(dead_code)]
            pub fn word(self) -> &'static str {
                match self { $( Self::$variant => $word ),+ }
            }

            /// The code itself.
            #[allow(dead_code)]
            pub fn as_code(self) -> &'static str {
                match self { $( Self::$variant => $code ),+ }
            }

            /// The code for a back-end word, `"99"` when there is none.
            #[allow(dead_code)]
            pub fn code(word: &str) -> &'static str {
                match word { $( $word => $code, )+ _ => "99" }
            }

            /// The code for a back-end word read from an output.
            #[allow(dead_code)]
            pub fn of(v: &Value) -> Value {
                v.as_str().map_or(Value::Null, |w| Value::from(Self::code(w)))
            }
        }
    };
}

codes!(SellStatCd {
    S10 = "10" => "in_stock",
    S20 = "20" => "out_of_stock",
    S30 = "30" => "pre_order",
    S40 = "40" => "back_order",
    S90 = "90" => "discontinued",
});

codes!(PrmtStatCd {
    P00 = "00" => "draft",
    P10 = "10" => "scheduled",
    P20 = "20" => "running",
    P30 = "30" => "paused",
    P90 = "90" => "ended",
});

codes!(MbrStatCd {
    M10 = "10" => "active",
    M20 = "20" => "dormant",
    M30 = "30" => "blocked",
    M80 = "80" => "pending_withdrawal",
    M90 = "90" => "withdrawn",
});

codes!(ClmStatCd {
    R10 = "R10" => "requested",
    R11 = "R11" => "charge_due",
    R15 = "R15" => "photos_requested",
    R20 = "R20" => "approved",
    R30 = "R30" => "pickup_instructed",
    R40 = "R40" => "picked_up",
    R50 = "R50" => "inspected",
    R60 = "R60" => "reship_requested",
    R90 = "R90" => "completed",
    R91 = "R91" => "declined",
    R92 = "R92" => "withdrawn",
    R93 = "R93" => "converted",
});

codes!(ClmRsnCd {
    C01 = "01" => "defect",
    C02 = "02" => "damaged_in_transit",
    C03 = "03" => "not_as_described",
    C04 = "04" => "wrong_item_sent",
    C05 = "05" => "wrong_option_sent",
    C06 = "06" => "missing_parts",
    C11 = "11" => "wrong_size",
    C12 = "12" => "different_option_wanted",
    C13 = "13" => "changed_mind",
});

codes!(RspbCd {
    S = "S" => "store_fault",
    L = "L" => "seller_fault",
    C = "C" => "customer_fault",
    D = "D" => "carrier_fault",
    N = "N" => "none",
});

codes!(RtnDlvCstCd {
    F1 = "1" => "free_return",
    F2 = "2" => "return_fees_customer_responsibility",
    F3 = "3" => "return_shipping_fees",
});

codes!(MsgTpCd {
    M01 = "M01" => "ask_for_photos",
    M02 = "M02" => "decline",
    M03 = "M03" => "refund_done",
    M99 = "M99" => "custom",
});

codes!(SndChnlCd {
    C1 = "1" => "email",
    C2 = "2" => "sms",
    C3 = "3" => "push",
    C4 = "4" => "in_app",
});

codes!(CpnAplyTpCd {
    A01 = "01" => "automatic_at_checkout",
    A02 = "02" => "code_entry",
    A03 = "03" => "download",
});

codes!(DupAplyCd {
    N = "N" => "no_tier_coupons",
    Y = "Y" => "with_tier_coupons",
    X = "X" => "no_other_coupons",
});

codes!(PayMthdCd {
    P10 = "10" => "credit_card",
    P20 = "20" => "bank_transfer",
    P30 = "30" => "virtual_account",
    P40 = "40" => "loyalty_points",
    P50 = "50" => "store_credit",
});

codes!(CmprBaseCd {
    B1 = "1" => "same_weekday_last_4_weeks",
    B2 = "2" => "previous_period",
});

codes!(DispPageCd {
    D01 = "01" => "main",
    D02 = "02" => "category",
    D03 = "03" => "product",
    D04 = "04" => "search",
});

codes!(CornerTpCd {
    H = "H" => "hero",
    B = "B" => "banner_board",
    G = "G" => "product_grid",
    S = "S" => "brand_strip",
    C = "C" => "collection_feed",
});

codes!(DispStatCd {
    D00 = "00" => "draft",
    D10 = "10" => "scheduled",
    D20 = "20" => "active",
    D90 = "90" => "ended",
});

codes!(RfndMthdCd {
    R01 = "01" => "original_payment",
    R02 = "02" => "bank_account",
    R03 = "03" => "store_credit",
});

codes!(AplyTgtCd {
    C = "C" => "category",
    P = "P" => "products",
});

codes!(RowStat {
    I = "I" => "insert",
    U = "U" => "update",
    D = "D" => "delete",
});

/// The promotion type code is a pair of back-end facts: the discount layer and whether the
/// promotion is for one tier. `T05` is everything else.
pub fn prmt_tp(layer: &str, tier: Option<&str>) -> &'static str {
    let for_tier = tier.is_some_and(|t| t != "everyone");
    match (layer, for_tier) {
        ("item_price", false) => "T01",
        ("order_discount", _) => "T02",
        ("item_price", true) => "T03",
        ("add_on", _) => "T04",
        _ => "T05",
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, schemars::JsonSchema)]
pub enum PrmtTpCd {
    #[serde(rename = "T01")]
    T01,
    #[serde(rename = "T02")]
    T02,
    #[serde(rename = "T03")]
    T03,
    #[serde(rename = "T04")]
    T04,
    #[serde(rename = "T05")]
    T05,
}

/// The store's tiers as grade codes: `G00` is every shopper, then `G01`, `G02`… by rank.
#[derive(Debug, Clone, Default)]
pub struct Grades {
    /// (code, tier id, name, members)
    pub rows: Vec<(String, String, String, Value)>,
}

impl Grades {
    pub fn from_tiers(out: &Value) -> Self {
        let rows = out["items"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|t| {
                let id = t["id"].as_str()?;
                let rank = t["rank"].as_i64().unwrap_or(0);
                Some((
                    format!("G{rank:02}"),
                    id.to_owned(),
                    t["name"].as_str().unwrap_or(id).to_owned(),
                    t["member_count"].clone(),
                ))
            })
            .collect();
        Self { rows }
    }

    /// The code of a tier id (`everyone` and no tier are `G00`).
    pub fn code(&self, tier: Option<&str>) -> String {
        match tier {
            None | Some("everyone") => "G00".to_owned(),
            Some(t) => self
                .rows
                .iter()
                .find(|r| r.1 == t)
                .map_or_else(|| "G99".to_owned(), |r| r.0.clone()),
        }
    }

    /// The tier id of a code.
    pub fn tier(&self, code: &str) -> Option<String> {
        if code == "G00" {
            return Some("everyone".to_owned());
        }
        self.rows.iter().find(|r| r.0 == code).map(|r| r.1.clone())
    }
}
