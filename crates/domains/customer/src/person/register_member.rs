//! `register_member`: sign a person up (individual or business). Needs a finished identity check
//! (`verification_token`) and the mandatory consents. No password goes through this tool: once the
//! account exists, a password set-up link goes to the verified contact and the member sets the
//! password through the person-only `POST /v1/customer/password-reset/confirm`.

use acom_contract::{
    ActionType, Audience, Change, ChangeOp, Check, Committed, Group, Kind, Method, OpMeta,
    Operation, Planned, Slot, ToolError, UndoSupport, Verification, WriteOp,
};
use acom_core::enums::{ConsentType, MemberType, TierChangeCause};
use acom_store::{Cx, TxCx};
use chrono::{DateTime, Duration, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::{entry_tiers, identifier_taken, membership_number, withdrawals_matching};
use crate::access::{approval, day, db, policy_number, policy_words};
use crate::verification::{self, not_delivered, usable, use_token};
use crate::{
    DUPLICATE_MEMBER, ID_REUSE_BLOCKED, IDENTITY_NOT_VERIFIED, MANDATORY_CONSENT_MISSING,
    REJOIN_BLOCKED, consent, repo, secret, wording,
};

/// One consent given at sign-up.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SignupConsent {
    /// terms; privacy; marketing_email; marketing_sms; marketing_push; marketing_in_app.
    #[schemars(example = &"terms")]
    pub consent_type: ConsentType,
    /// Whether the person agreed.
    #[schemars(example = true)]
    pub granted: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RegisterMemberInput {
    /// individual or business.
    #[serde(rename = "type")]
    #[schemars(example = &"individual")]
    pub member_type: MemberType,
    /// The name to show.
    #[schemars(length(min = 1, max = 100), example = &"Alex Kim")]
    pub name: String,
    /// Email, also the sign-in id.
    #[schemars(length(min = 3, max = 200), example = &"alex@example.com")]
    pub email: String,
    /// Phone number. Required when the identity check went by SMS.
    #[schemars(length(min = 7, max = 30), example = &"+1 718 555 0142")]
    pub telephone: Option<String>,
    /// Registered business name. Required for a business member.
    #[schemars(length(min = 1, max = 200), example = &"Harbor Supply Co.")]
    pub legal_name: Option<String>,
    /// Business registration number. Required for a business member.
    #[schemars(length(min = 4, max = 40), example = &"12-3456789")]
    pub tax_id: Option<String>,
    /// From `verify_identity` with `purpose: signup`, for this email or phone.
    #[schemars(regex(pattern = r"^ivf_[a-z0-9_]+$"), example = &"ivf_201")]
    pub verification_token: String,
    /// The consents given. Terms and privacy must be granted.
    #[schemars(example = consents_example())]
    pub consents: Vec<SignupConsent>,
}

fn consents_example() -> Value {
    json!([{"consent_type": "terms", "granted": true}, {"consent_type": "privacy", "granted": true}])
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisterPlan {
    pub customer_id: String,
    pub member_type: String,
    pub name: String,
    pub email: String,
    pub telephone: Option<String>,
    pub legal_name: Option<String>,
    pub tax_id: Option<String>,
    /// The identity check the token named, its destination and when it succeeded.
    pub verification_id: String,
    pub verified_email_at: Option<DateTime<Utc>>,
    pub verified_telephone_at: Option<DateTime<Utc>>,
    /// `(program_id, tier_id, membership_number)` per program: the entry tier.
    pub memberships: Vec<(String, String, String)>,
    /// `(consent_id, consent_type, granted)`.
    pub consents: Vec<(String, String, bool)>,
    /// The password set-up request and where it goes.
    pub setup_request_id: String,
    pub setup_method: Option<String>,
    pub setup_channel: String,
    pub setup_ttl_seconds: i64,
}

pub struct RegisterMember;

impl Operation for RegisterMember {
    const META: OpMeta = OpMeta {
        address: "customer.person.register",
        tool: "register_member",
        group: Group::Engage,
        kind: Kind::Write,
        method: Method::Post,
        path: "/v1/customer/people",
        summary: "Sign up a member (individual or business) after verify_identity, with the consents given. Never takes a password: a set-up link goes to the verified contact. The new member enters each program's entry tier.",
        consumes: &[],
        produces: &[Slot::one("customer_id")],
        checks: &[
            "identity_verification.required_for",
            "consent.mandatory",
            "withdrawal.rejoin.block_days",
        ],
        errors: &[
            IDENTITY_NOT_VERIFIED,
            MANDATORY_CONSENT_MISSING,
            DUPLICATE_MEMBER,
            ID_REUSE_BLOCKED,
            REJOIN_BLOCKED,
        ],
        audience: Audience::Both,
        action_type: ActionType::REGISTER,
        person_scope: "",
    };
    type Input = RegisterMemberInput;
    type Output = Value;
}

fn bad(field: &str, reason: &str, message: String) -> ToolError {
    ToolError::invalid_input(message)
        .detail(json!({"field": field, "reason": reason}))
        .next("register_member")
}

impl WriteOp<Cx> for RegisterMember {
    type Plan = RegisterPlan;
    /// An account, once made, is closed by its member (withdrawal), not undone.
    const UNDO: UndoSupport = UndoSupport::NotSupported;

    async fn preview(
        &self,
        cx: &Cx,
        input: RegisterMemberInput,
    ) -> Result<Planned<RegisterPlan>, ToolError> {
        let email = input.email.trim().to_owned();
        if !wording::looks_like_email(&email) {
            return Err(bad(
                "email",
                "format",
                format!("`{email}` is not an email address."),
            ));
        }
        let telephone = input.telephone.map(|t| t.trim().to_owned());
        if let Some(t) = &telephone
            && !wording::looks_like_phone(t)
        {
            return Err(bad(
                "telephone",
                "format",
                format!("`{t}` is not a phone number."),
            ));
        }
        let business = input.member_type == MemberType::Business;
        let legal_name = input.legal_name.map(|s| s.trim().to_owned());
        let tax_id = input.tax_id.map(|s| s.trim().to_owned());
        if business && (legal_name.is_none() || tax_id.is_none()) {
            return Err(bad(
                "legal_name",
                "missing",
                "A business member needs `legal_name` and `tax_id`.".into(),
            ));
        }
        // Mandatory consents (consent.mandatory).
        let mandatory = policy_words(cx, "customer.consent.mandatory");
        let optional = policy_words(cx, "customer.consent.optional");
        let missing: Vec<&String> = mandatory
            .iter()
            .filter(|m| {
                !input
                    .consents
                    .iter()
                    .any(|c| c.granted && c.consent_type.as_str() == m.as_str())
            })
            .collect();
        if !missing.is_empty() {
            return Err(ToolError::new(
                MANDATORY_CONSENT_MISSING,
                format!(
                    "Signing up needs {} accepted.",
                    missing
                        .iter()
                        .map(|s| s.as_str())
                        .collect::<Vec<_>>()
                        .join(" and ")
                ),
            )
            .detail(json!({"rule": "consent.mandatory", "missing": missing}))
            .next("set_consent"));
        }
        let mut consents: Vec<(String, String, bool)> = Vec::new();
        for c in &input.consents {
            let kind = c.consent_type.as_str();
            let known = mandatory.iter().chain(&optional).any(|k| k == kind);
            if known && !consents.iter().any(|(_, k, _)| k == kind) {
                consents.push((consent::new_id(), kind.to_owned(), c.granted));
            }
        }
        let now = cx.clock.now();
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        // The token must be a finished sign-up check of this email or phone.
        let check = usable(
            &mut tx,
            &input.verification_token,
            "signup",
            None,
            "verify_identity",
        )
        .await?;
        let destination = check.destination.clone().unwrap_or_default();
        let by_email = secret::normalize(&destination) == secret::normalize(&email);
        let by_phone = telephone
            .as_deref()
            .is_some_and(|t| secret::normalize(t) == secret::normalize(&destination));
        if !by_email && !by_phone {
            return Err(ToolError::new(
                IDENTITY_NOT_VERIFIED,
                "The verification_token was made for another email or phone; verify this one.",
            )
            .detail(json!({"field": "verification_token", "reason": "destination"}))
            .next("verify_identity"));
        }
        let method = verification::method(&cx.policy, "signup", &check.method)?;
        if method.channel.as_deref() == Some("sms") && telephone.is_none() {
            return Err(bad(
                "telephone",
                "missing",
                "The identity check went by SMS; give the `telephone` it used.".into(),
            ));
        }
        // One account per email and phone.
        let email_n = secret::normalize(&email);
        let mut dupes = vec![];
        if identifier_taken(&mut tx, "email", &email_n).await? {
            dupes.push("email");
        }
        if let Some(t) = &telephone
            && identifier_taken(&mut tx, "telephone", &secret::normalize(t)).await?
        {
            dupes.push("telephone");
        }
        if let Some(field) = dupes.first() {
            return Err(ToolError::new(
                DUPLICATE_MEMBER,
                format!("A member already uses this {field}."),
            )
            .detail(json!({"entity": "person", "field": field}))
            .next("check_availability"));
        }
        // People who left recently wait before joining again (withdrawal.rejoin).
        let mut ids = vec![email_n.clone()];
        ids.extend(telephone.as_deref().map(secret::normalize));
        let held = withdrawals_matching(&mut tx, &ids).await?;
        if let Some(h) = held.iter().find(|h| h.rejoin_after > now) {
            return Err(ToolError::new(
                REJOIN_BLOCKED,
                format!(
                    "This person left the store recently and can sign up again from {}.",
                    day(cx.time, h.rejoin_after)
                ),
            )
            .detail(json!({"rule": "withdrawal.rejoin.block_days",
                           "until": cx.time.format(h.rejoin_after)})));
        }
        if held
            .iter()
            .any(|h| h.identifier_reusable_after.is_none_or(|t| t > now))
        {
            return Err(ToolError::new(
                ID_REUSE_BLOCKED,
                "This email or phone cannot be used for a new account yet.",
            )
            .detail(json!({"rule": "withdrawal.identifier_reuse"}))
            .next("check_availability"));
        }
        let entries = entry_tiers(&mut tx).await?;
        tx.commit().await.map_err(ToolError::from)?;

        let block_days = policy_number(cx, "customer.withdrawal.rejoin.block_days")?;
        let setup = verification::allowed(&cx.policy, "password_reset");
        let (setup_method, setup_channel) = if by_email {
            (
                setup.iter().find(|m| m.starts_with("email")).cloned(),
                "email",
            )
        } else {
            (setup.iter().find(|m| m.starts_with("sms")).cloned(), "sms")
        };
        let link_ttl = policy_number(cx, "customer.identity_verification.link_ttl_seconds")?;
        let ago = (now - check.verified_at.unwrap_or(now))
            .num_minutes()
            .max(0);
        let mut changes = vec![Change {
            op: ChangeOp::Create,
            entity: "person".into(),
            id: String::new(),
            from: Value::Null,
            to: json!("active"),
            note: Some(format!("{} member", input.member_type)),
        }];
        changes.extend(entries.iter().map(|(p, t)| Change {
            op: ChangeOp::Create,
            entity: "program_membership".into(),
            id: String::new(),
            from: Value::Null,
            to: json!({"program_id": p, "tier_id": t}),
            note: Some("Entry tier".into()),
        }));
        changes.push(Change {
            op: ChangeOp::Create,
            entity: "consent_record".into(),
            id: String::new(),
            from: Value::Null,
            to: json!(
                consents
                    .iter()
                    .map(|(_, k, g)| json!({k: g}))
                    .collect::<Vec<_>>()
            ),
            note: None,
        });
        changes.push(Change {
            op: ChangeOp::Send,
            entity: "recovery_request".into(),
            id: String::new(),
            from: Value::Null,
            to: json!("link_sent"),
            note: Some(format!(
                "Password set-up link to {}",
                verification::masked(&destination)
            )),
        });
        let facts = json!({"member_type": input.member_type.as_str()});
        Ok(Planned {
            changes,
            checks: vec![
                Check {
                    rule: "identity_verification.required_for".into(),
                    passed: true,
                    detail: Some(format!(
                        "{} verified {ago} min ago",
                        if by_email { "Email" } else { "Phone" }
                    )),
                    fix_tool: Some("verify_identity".into()),
                },
                Check {
                    rule: "consent.mandatory".into(),
                    passed: true,
                    detail: Some(format!("{} accepted", mandatory.join(" and "))),
                    fix_tool: Some("set_consent".into()),
                },
                Check {
                    rule: "withdrawal.rejoin.block_days".into(),
                    passed: true,
                    detail: Some(format!(
                        "No withdrawal of this person in the last {block_days} days"
                    )),
                    fix_tool: None,
                },
            ],
            basis: vec![],
            approval: approval(cx, Self::META.tool, &facts),
            plan: RegisterPlan {
                customer_id: format!("cus_{}", acom_core::id::new_suffix()),
                member_type: input.member_type.as_str().to_owned(),
                name: input.name.trim().to_owned(),
                email,
                telephone,
                legal_name,
                tax_id,
                verification_id: check.id,
                verified_email_at: by_email.then_some(check.verified_at).flatten(),
                verified_telephone_at: (!by_email).then_some(check.verified_at).flatten(),
                memberships: entries
                    .into_iter()
                    .map(|(p, t)| (p, t, membership_number()))
                    .collect(),
                consents,
                setup_request_id: format!("rcv_{}", acom_core::id::new_suffix()),
                setup_method,
                setup_channel: setup_channel.into(),
                setup_ttl_seconds: link_ttl,
            },
        })
    }

    async fn commit(&self, tx: &mut TxCx, plan: &RegisterPlan) -> Result<Committed, ToolError> {
        let now = tx.clock.now();
        let fresh = sqlx::query(
            "INSERT INTO customer.person (tenant_id, id, name, email, telephone, member_state,
               member_type, legal_name, tax_id, registered_at, email_verified_at,
               telephone_verified_at)
             VALUES ($1, $2, $3, $4, $5, 'active', $6, $7, $8, $9, $10, $11)
             ON CONFLICT (tenant_id, id) DO NOTHING",
        )
        .bind(tx.tenant.as_str())
        .bind(&plan.customer_id)
        .bind(&plan.name)
        .bind(&plan.email)
        .bind(plan.telephone.as_deref())
        .bind(&plan.member_type)
        .bind(plan.legal_name.as_deref())
        .bind(plan.tax_id.as_deref())
        .bind(now)
        .bind(plan.verified_email_at)
        .bind(plan.verified_telephone_at)
        .execute(&mut *tx.tx)
        .await
        .map_err(db)?
        .rows_affected()
            > 0;
        let mut produced = Map::new();
        produced.insert("customer_id".into(), json!(plan.customer_id));
        if !fresh {
            // A repeated run: the account is there already.
            return Ok(Committed {
                id: plan.customer_id.clone(),
                produced,
            });
        }
        use_token(tx, &plan.verification_id).await?;
        let agent = tx.principal.subject.clone();
        for (program, tier, number) in &plan.memberships {
            sqlx::query(
                "INSERT INTO customer.program_membership (tenant_id, customer_id,
                   member_program_id, tier_id, membership_number, tier_since)
                 VALUES ($1, $2, $3, $4, $5, $6) ON CONFLICT DO NOTHING",
            )
            .bind(tx.tenant.as_str())
            .bind(&plan.customer_id)
            .bind(program)
            .bind(tier)
            .bind(number)
            .bind(now)
            .execute(&mut *tx.tx)
            .await
            .map_err(db)?;
            repo::add_history(
                tx,
                &repo::TierChange {
                    customer_id: plan.customer_id.clone(),
                    program_id: program.clone(),
                    from_tier_id: None,
                    to_tier_id: Some(tier.clone()),
                    cause: TierChangeCause::Entry.as_str().into(),
                    evaluation_id: None,
                    snapshot: None,
                    agent_id: agent.clone(),
                    reason: None,
                    expires: None,
                },
            )
            .await?;
        }
        for (id, kind, granted) in &plan.consents {
            consent::record(tx, id, &plan.customer_id, kind, *granted, Self::META.tool).await?;
        }
        // The password set-up link: a reset request for the new account. Its token is made here
        // and only its hash is kept; delivery is a stub.
        let token = secret::token();
        let to = if plan.setup_channel == "email" {
            plan.email.clone()
        } else {
            plan.telephone.clone().unwrap_or_default()
        };
        sqlx::query(
            "INSERT INTO customer.recovery_request (tenant_id, id, recovery_kind, customer_id,
               identifier_fingerprint, method, message_channel, token_hash, agent_id, reason,
               requested_at, expires)
             VALUES ($1, $2, 'password_reset', $3, $4, $5, $6, $7, $8, $9, $10, $11)
             ON CONFLICT (tenant_id, id) DO NOTHING",
        )
        .bind(tx.tenant.as_str())
        .bind(&plan.setup_request_id)
        .bind(&plan.customer_id)
        .bind(secret::fingerprint(&to))
        .bind(plan.setup_method.as_deref())
        .bind(&plan.setup_channel)
        .bind(secret::hash(&token))
        .bind(&agent)
        .bind("Password set-up after sign-up")
        .bind(now)
        .bind(now + Duration::seconds(plan.setup_ttl_seconds))
        .execute(&mut *tx.tx)
        .await
        .map_err(db)?;
        let mut setup = not_delivered(&plan.setup_channel, &to);
        setup["request_id"] = json!(plan.setup_request_id);
        produced.insert("password_setup".into(), setup);
        produced.insert(
            "tier_id".into(),
            json!(plan.memberships.first().map(|(_, t, _)| t)),
        );
        Ok(Committed {
            id: plan.customer_id.clone(),
            produced,
        })
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &RegisterPlan,
        _done: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let a = repo::account(&mut tx, &plan.customer_id).await?;
        let m = repo::memberships(&mut tx, &plan.customer_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let mut mismatches = Vec::new();
        match a {
            Some(a) if a.member_state == "active" => {}
            Some(a) => mismatches.push(format!("{} is {}", a.id, a.member_state)),
            None => mismatches.push(format!("{} was not saved", plan.customer_id)),
        }
        if m.len() != plan.memberships.len() {
            mismatches.push(format!("{} has {} memberships", plan.customer_id, m.len()));
        }
        Ok(Verification {
            matched: mismatches.is_empty(),
            mismatches,
        })
    }

    async fn undo(
        &self,
        _tx: &mut TxCx,
        _plan: &RegisterPlan,
        _done: &Committed,
    ) -> Result<(), ToolError> {
        // Not reached: UNDO is NotSupported.
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use acom_contract::schema::{closed_schema_for, violations};

    use super::*;

    #[test]
    fn input_is_closed_and_has_no_password() {
        let s = closed_schema_for::<RegisterMemberInput>();
        assert_eq!(violations(&s), Vec::<String>::new(), "{s:#}");
        assert!(s["properties"].get("password").is_none());
        assert_eq!(
            s["properties"]["type"]["enum"],
            json!(["individual", "business"])
        );
    }
}
