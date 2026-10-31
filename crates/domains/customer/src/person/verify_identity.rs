//! `verify_identity`: prove a person owns an email or phone (a one-time code or link), or that a
//! business is registered (a registry lookup), before sign-up, a contact change or a business
//! check. The first call sends a code; the second call, with the code, finishes the check and
//! hands out a one-time `verification_token`.

use acom_contract::{
    ActionType, Audience, Basis, Change, ChangeOp, Check, Committed, ErrorCode, Group, Kind,
    Method, OpMeta, Operation, Planned, Slot, ToolError, UndoSupport, Verification, WriteOp,
};
use acom_store::{Cx, TxCx};
use chrono::Duration;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::access::{approval, policy_number};
use crate::verification::{
    self, NewCheck, check, mark_verified, not_delivered, record_wrong_code, sends_since, waiting,
};
use crate::{
    CODE_MISMATCH, METHOD_NOT_ENABLED, TOO_MANY_ATTEMPTS, VERIFICATION_EXPIRED, secret, wording,
};

/// What the check is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum VerifyPurpose {
    /// Before `register_member`.
    Signup,
    /// Before `update_member` changes an email or phone: the new one is checked.
    ContactChange,
    /// Before `verify_business`, by a business registry lookup.
    BusinessVerification,
}

impl VerifyPurpose {
    fn word(self) -> &'static str {
        match self {
            Self::Signup => "signup",
            Self::ContactChange => "contact_change",
            Self::BusinessVerification => "business_verification",
        }
    }
}

/// The business a registry lookup checks.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RegistrySubject {
    /// The business registration number.
    #[schemars(length(min = 4, max = 40), example = &"12-3456789")]
    pub tax_id: String,
    /// The registered business name.
    #[schemars(length(min = 1, max = 200), example = &"Harbor Supply Co.")]
    pub legal_name: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VerifyIdentityInput {
    /// signup; contact_change; business_verification.
    #[schemars(example = &"signup")]
    pub purpose: VerifyPurpose,
    /// A method id the store turned on for the purpose (`identity_verification.required_for`),
    /// for example `email_code`, `sms_code`, `business_registry`.
    #[schemars(regex(pattern = r"^[a-z][a-z0-9_]*$"), example = &"email_code")]
    pub method: String,
    /// The email or phone number the code goes to. For code and link methods.
    #[schemars(length(min = 3, max = 200), example = &"alex@example.com")]
    pub destination: Option<String>,
    /// The business to look up. For registry methods.
    pub subject: Option<RegistrySubject>,
    /// The code the person received, on the second call.
    #[schemars(regex(pattern = r"^[0-9]{4,12}$"), example = &"482913")]
    pub code: Option<String>,
}

/// What the run does: send a code, finish a check with the right code, or look up a registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Step {
    SendCode,
    ConfirmCode,
    RegistryLookup,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerifyIdentityPlan {
    pub step: Step,
    /// The check to make (send, registry) or to finish (confirm).
    pub check: NewCheck,
    pub token_ttl_seconds: i64,
}

pub struct VerifyIdentity;

impl Operation for VerifyIdentity {
    const META: OpMeta = OpMeta {
        address: "customer.person.verify_identity",
        tool: "verify_identity",
        group: Group::Engage,
        kind: Kind::Write,
        method: Method::Post,
        path: "/v1/customer/people/verify-identity",
        summary: "Prove a person owns an email or phone (send a one-time code, then call again with it) or that a business is registered, before sign-up, a contact change or a business check. Gives a one-time verification_token.",
        consumes: &[],
        produces: &[Slot::one("verification_token")],
        checks: &[
            "identity_verification.required_for",
            "identity_verification.resend_cooldown_seconds",
        ],
        errors: &[
            METHOD_NOT_ENABLED,
            CODE_MISMATCH,
            VERIFICATION_EXPIRED,
            TOO_MANY_ATTEMPTS,
            ErrorCode::RATE_LIMITED,
        ],
        audience: Audience::Both,
        action_type: ActionType::ASSESS,
        person_scope: "",
    };
    type Input = VerifyIdentityInput;
    type Output = Value;
}

fn missing(field: &str, why: &str) -> ToolError {
    ToolError::invalid_input(why.to_owned())
        .detail(json!({"field": field, "reason": "missing"}))
        .next("verify_identity")
}

/// Whether `d` looks like an address for the method's channel.
fn fits_channel(channel: Option<&str>, d: &str) -> bool {
    match channel {
        Some("email") => wording::looks_like_email(d),
        Some("sms") => wording::looks_like_phone(d),
        _ => true,
    }
}

impl WriteOp<Cx> for VerifyIdentity {
    type Plan = VerifyIdentityPlan;
    /// A sent code cannot be unsent, and a finished check is history.
    const UNDO: UndoSupport = UndoSupport::NotSupported;

    async fn preview(
        &self,
        cx: &Cx,
        input: VerifyIdentityInput,
    ) -> Result<Planned<VerifyIdentityPlan>, ToolError> {
        let purpose = input.purpose.word();
        let m = verification::method(&cx.policy, purpose, &input.method)?;
        let secs = |k: &str| policy_number(cx, &format!("customer.identity_verification.{k}"));
        let token_ttl = secs("token_ttl_seconds")?;
        let required = Check {
            rule: "identity_verification.required_for".into(),
            passed: true,
            detail: Some(format!("{} is allowed for {purpose}", m.id)),
            fix_tool: Some("get_policy".into()),
        };
        let customer_id = cx.principal.customer_id().map(str::to_owned);
        let now = cx.clock.now();

        // A registry lookup finishes at once.
        if !m.sends_code() {
            let subject = input
                .subject
                .ok_or_else(|| missing("subject", "A registry lookup needs `subject`."))?;
            return Ok(Planned {
                changes: vec![Change {
                    op: ChangeOp::Create,
                    entity: "identity_verification".into(),
                    id: String::new(),
                    from: Value::Null,
                    to: json!("verified"),
                    note: Some(
                        "Registry lookup stub: no registry provider is connected, so the details are recorded as given"
                            .into(),
                    ),
                }],
                checks: vec![required],
                basis: vec![],
                approval: approval(cx, Self::META.tool, &json!({"purpose": purpose})),
                plan: VerifyIdentityPlan {
                    step: Step::RegistryLookup,
                    check: NewCheck {
                        id: format!("ivf_{}", acom_core::id::new_suffix()),
                        purpose: purpose.into(),
                        method: m.id,
                        destination: None,
                        customer_id,
                        subject: Some(json!(subject)),
                        ttl_seconds: token_ttl,
                        verified_token_ttl_seconds: Some(token_ttl),
                        code_length: None,
                    },
                    token_ttl_seconds: token_ttl,
                },
            });
        }

        let destination = input
            .destination
            .as_deref()
            .map(secret::normalize)
            .filter(|d| !d.is_empty())
            .ok_or_else(|| missing("destination", "Say where the code goes (`destination`)."))?;
        if !fits_channel(m.channel.as_deref(), &destination) {
            return Err(ToolError::invalid_input(format!(
                "{} sends by {}; `destination` is not one.",
                m.id,
                m.channel.as_deref().unwrap_or("message")
            ))
            .detail(json!({"field": "destination", "reason": "format"}))
            .next("verify_identity"));
        }
        let mut tx = cx.read().await.map_err(ToolError::from)?;

        // Second call: finish the waiting check with the code.
        if let Some(code) = input.code {
            let open = waiting(&mut tx, purpose, &m.id, &destination).await?;
            tx.commit().await.map_err(ToolError::from)?;
            let max = i32::try_from(secs("code.max_attempts")?).unwrap_or(i32::MAX);
            let c = match open {
                Some(c) if c.expires > now => c,
                _ => {
                    return Err(ToolError::new(
                        VERIFICATION_EXPIRED,
                        "No code is waiting for this email or phone; it expired or was never sent. Send a new one.",
                    )
                    .detail(json!({"purpose": purpose, "method": m.id}))
                    .next("verify_identity"));
                }
            };
            if c.attempts >= max {
                return Err(too_many(max));
            }
            let right = c
                .code_hash
                .as_deref()
                .is_some_and(|h| secret::matches(h, code.trim()));
            if !right {
                // The wrong try is counted at once: it is a security record, not a change of
                // the store, and the check must stop working after max_attempts.
                let mut w = cx.begin().await.map_err(ToolError::from)?;
                let attempts = record_wrong_code(&mut w, &c.id, max).await?;
                w.commit().await.map_err(ToolError::from)?;
                if attempts >= max {
                    return Err(too_many(max));
                }
                return Err(ToolError::new(
                    CODE_MISMATCH,
                    "That code is not the one sent. Check it and try again.",
                )
                .detail(json!({"attempts_left": max - attempts}))
                .next("verify_identity"));
            }
            return Ok(Planned {
                changes: vec![Change {
                    op: ChangeOp::Update,
                    entity: "identity_verification".into(),
                    id: c.id.clone(),
                    from: json!("code_sent"),
                    to: json!("verified"),
                    note: None,
                }],
                checks: vec![required],
                basis: vec![Basis {
                    entity: "customer.identity_verification".into(),
                    id: c.id.clone(),
                    version: i64::from(c.version),
                }],
                approval: approval(cx, Self::META.tool, &json!({"purpose": purpose})),
                plan: VerifyIdentityPlan {
                    step: Step::ConfirmCode,
                    check: NewCheck {
                        id: c.id,
                        purpose: purpose.into(),
                        method: m.id,
                        destination: Some(destination),
                        customer_id,
                        subject: None,
                        ttl_seconds: 0,
                        verified_token_ttl_seconds: None,
                        code_length: None,
                    },
                    token_ttl_seconds: token_ttl,
                },
            });
        }

        // First call: send a code, within the resend limits.
        let cooldown = secs("resend_cooldown_seconds")?;
        let per_hour = secs("max_sends_per_destination_per_hour")?;
        let (sent, last) = sends_since(&mut tx, &destination, now - Duration::hours(1)).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let wait = last
            .map(|l| (l + Duration::seconds(cooldown) - now).num_seconds())
            .filter(|s| *s > 0);
        if let Some(retry) = wait {
            return Err(rate_limited(
                retry,
                "A code was just sent; wait before sending another.",
            ));
        }
        if sent >= per_hour {
            let retry = last.map_or(3600, |l| {
                (l + Duration::hours(1) - now).num_seconds().max(1)
            });
            return Err(rate_limited(
                retry,
                "Too many codes went to this email or phone this hour.",
            ));
        }
        let ttl = if m.kind == "magic_link" {
            secs("link_ttl_seconds")?
        } else {
            secs("code.ttl_seconds")?
        };
        let length = usize::try_from(secs("code.length")?).unwrap_or(6);
        let sent_word = if m.kind == "magic_link" {
            "link_sent"
        } else {
            "code_sent"
        };
        Ok(Planned {
            changes: vec![Change {
                op: ChangeOp::Create,
                entity: "identity_verification".into(),
                id: String::new(),
                from: Value::Null,
                to: json!(sent_word),
                note: Some(format!("To {}", verification::masked(&destination))),
            }],
            checks: vec![
                required,
                Check {
                    rule: "identity_verification.resend_cooldown_seconds".into(),
                    passed: true,
                    detail: Some(format!(
                        "{} of {per_hour} this hour; one every {cooldown} seconds",
                        sent + 1
                    )),
                    fix_tool: None,
                },
            ],
            basis: vec![],
            approval: approval(cx, Self::META.tool, &json!({"purpose": purpose})),
            plan: VerifyIdentityPlan {
                step: Step::SendCode,
                check: NewCheck {
                    id: format!("ivf_{}", acom_core::id::new_suffix()),
                    purpose: purpose.into(),
                    method: m.id,
                    destination: Some(destination),
                    customer_id,
                    subject: None,
                    ttl_seconds: ttl,
                    verified_token_ttl_seconds: None,
                    code_length: Some(length),
                },
                token_ttl_seconds: token_ttl,
            },
        })
    }

    async fn commit(
        &self,
        tx: &mut TxCx,
        plan: &VerifyIdentityPlan,
    ) -> Result<Committed, ToolError> {
        let c = &plan.check;
        let mut produced = Map::new();
        match plan.step {
            Step::SendCode => {
                // The code exists only here, on its way to the delivery stub.
                let _code = verification::insert(tx, c).await?;
                let to = c.destination.as_deref().unwrap_or_default();
                let channel = if to.contains('@') { "email" } else { "sms" };
                produced.insert("status".into(), json!("code_sent"));
                produced.insert("delivery".into(), not_delivered(channel, to));
            }
            Step::ConfirmCode => {
                mark_verified(tx, &c.id, Duration::seconds(plan.token_ttl_seconds)).await?;
                produced.insert("verification_token".into(), json!(c.id));
                produced.insert("expires_in".into(), json!(plan.token_ttl_seconds));
            }
            Step::RegistryLookup => {
                verification::insert(tx, c).await?;
                produced.insert("verification_token".into(), json!(c.id));
                produced.insert("expires_in".into(), json!(plan.token_ttl_seconds));
                produced.insert(
                    "registry".into(),
                    json!({"matched": true, "stub": true,
                           "note": "No business registry provider is connected; the details were recorded as given."}),
                );
            }
        }
        Ok(Committed {
            id: c.id.clone(),
            produced,
        })
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &VerifyIdentityPlan,
        _done: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let row = check(&mut tx, &plan.check.id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let want = match plan.step {
            Step::SendCode => "potential",
            _ => "completed",
        };
        let mismatches = match row {
            Some(r) if r.action_status == want => vec![],
            Some(r) => vec![format!("{} is {}", r.id, r.action_status)],
            None => vec![format!("{} was not saved", plan.check.id)],
        };
        Ok(Verification {
            matched: mismatches.is_empty(),
            mismatches,
        })
    }

    async fn undo(
        &self,
        _tx: &mut TxCx,
        _plan: &VerifyIdentityPlan,
        _done: &Committed,
    ) -> Result<(), ToolError> {
        // Not reached: UNDO is NotSupported.
        Ok(())
    }
}

fn too_many(max: i32) -> ToolError {
    ToolError::new(
        TOO_MANY_ATTEMPTS,
        "Every try of this code is used up; send a new code.",
    )
    .detail(json!({"limit": max, "policy_key": "identity_verification.code.max_attempts"}))
    .next("verify_identity")
}

fn rate_limited(retry_after: i64, message: &str) -> ToolError {
    ToolError::new(ErrorCode::RATE_LIMITED, message)
        .detail(json!({"retry_after_seconds": retry_after,
                       "policy_key": "identity_verification.resend_cooldown_seconds"}))
}

#[cfg(test)]
mod tests {
    use acom_contract::schema::{closed_schema_for, violations};

    use super::*;

    #[test]
    fn input_is_closed_with_purpose_words() {
        let s = closed_schema_for::<VerifyIdentityInput>();
        assert_eq!(violations(&s), Vec::<String>::new(), "{s:#}");
        assert_eq!(
            s["properties"]["purpose"]["enum"],
            json!(["signup", "contact_change", "business_verification"])
        );
        assert!(fits_channel(Some("email"), "alex@example.com"));
        assert!(!fits_channel(Some("sms"), "alex@example.com"));
    }
}
