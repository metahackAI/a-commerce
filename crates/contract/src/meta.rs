//! Static description of an operation: its names, kind, links, checks, errors, and scopes.

use serde::Serialize;

use crate::error::ErrorCode;

/// The four parts of the business (docs/api/README.md §1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Group {
    Offer,
    Transact,
    Engage,
    Operate,
}

/// R, W, W$ in the spec.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// No side effects.
    Read,
    /// Preview first; runs without a person when the approval policy allows it.
    Write,
    /// Moves money or reaches customers: always waits for a person's approval.
    WriteApproval,
}

impl Kind {
    pub fn spec_mark(self) -> &'static str {
        match self {
            Kind::Read => "R",
            Kind::Write => "W",
            Kind::WriteApproval => "W$",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Method {
    Get,
    Post,
    Put,
    Patch,
    Delete,
}

/// Who the operation is for. Decides which agents' tool searches can see it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Audience {
    Operator,
    Shopper,
    Both,
    /// Called by the system or by other services, never shown to a model.
    Internal,
}

/// A typed value that flows between tools: what one produces, the next consumes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Slot {
    /// Slot name, for example `product_id` or `promotion_id`.
    pub name: &'static str,
    /// `product_id[]` when true.
    pub many: bool,
}

impl Slot {
    pub const fn one(name: &'static str) -> Self {
        Self { name, many: false }
    }

    pub const fn many(name: &'static str) -> Self {
        Self { name, many: true }
    }
}

/// The schema.org action an operation performs (docs/architecture.md §5.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ActionType(pub &'static str);

impl ActionType {
    pub const SEARCH: Self = Self("SearchAction");
    pub const ASSESS: Self = Self("AssessAction");
    pub const CREATE: Self = Self("CreateAction");
    pub const UPDATE: Self = Self("UpdateAction");
    pub const DELETE: Self = Self("DeleteAction");
    pub const SCHEDULE: Self = Self("ScheduleAction");
    pub const ADD: Self = Self("AddAction");
    pub const SEND: Self = Self("SendAction");
    pub const ORDER: Self = Self("OrderAction");
    pub const PAY: Self = Self("PayAction");
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct OpMeta {
    /// `domain.resource.operation`, for example `promotion.offer.set_sale_prices`.
    pub address: &'static str,
    /// What the model sees, `verb_noun`.
    pub tool: &'static str,
    pub group: Group,
    pub kind: Kind,
    pub method: Method,
    /// REST path under the service root, for example `/v1/promotion/sale-prices`.
    pub path: &'static str,
    /// One line a model can choose the tool from.
    pub summary: &'static str,
    pub consumes: &'static [Slot],
    pub produces: &'static [Slot],
    /// Policy check keys run during preview.
    pub checks: &'static [&'static str],
    pub errors: &'static [ErrorCode],
    pub audience: Audience,
    pub action_type: ActionType,
}

impl OpMeta {
    pub fn domain(&self) -> &'static str {
        self.address.split('.').next().unwrap_or(self.address)
    }

    pub fn resource(&self) -> &'static str {
        self.address.split('.').nth(1).unwrap_or("")
    }
}
