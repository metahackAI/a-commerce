//! Person-only `PUT /v1/customer/me/password`: a signed-in member changes their password with
//! the current one (or, for an account that has none yet, a verification token). Other sessions
//! end (`password.on_change`).

use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, PersonOp, ToolError,
};
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{attempt_recorded, check_password, locked_until, log_person, set_password};
use crate::access::not_signed_in;
use crate::verification::{usable, use_token};
use crate::{
    CURRENT_PASSWORD_WRONG, NOT_SIGNED_IN, PASSWORD_POLICY, PASSWORD_REUSED,
    REAUTHENTICATION_LOCKED, repo,
};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ChangePasswordInput {
    /// The current password. An account without one gives `verification_token` instead.
    #[schemars(length(min = 1, max = 200), example = &"harbor-lantern-42")]
    pub current_password: Option<String>,
    /// A finished email or phone check of the member (`withdrawal_reauth` purpose), for accounts
    /// that have no password yet.
    #[schemars(regex(pattern = r"^ivf_[a-z0-9_]+$"), example = &"ivf_204")]
    pub verification_token: Option<String>,
    /// The new password (rules: `get_policy` `password`).
    #[schemars(length(min = 1, max = 200), example = &"quiet-meadow-81")]
    pub new_password: String,
}

/// Person-only: change the password while signed in.
pub struct ChangePassword;

impl Operation for ChangePassword {
    const META: OpMeta = OpMeta {
        address: "customer.person.change_password",
        tool: "change_password",
        group: Group::Engage,
        kind: Kind::Person,
        method: Method::Put,
        path: "/v1/customer/me/password",
        summary: "Change the password while signed in; other sessions end.",
        consumes: &[],
        produces: &[],
        checks: &[],
        errors: &[
            NOT_SIGNED_IN,
            CURRENT_PASSWORD_WRONG,
            REAUTHENTICATION_LOCKED,
            PASSWORD_POLICY,
            PASSWORD_REUSED,
        ],
        audience: Audience::Shopper,
        action_type: ActionType::UPDATE,
        person_scope: "human:account",
    };
    type Input = ChangePasswordInput;
    type Output = Value;
}

impl PersonOp<Cx> for ChangePassword {
    async fn run(&self, tx: &mut TxCx, input: ChangePasswordInput) -> Result<Value, ToolError> {
        let me = tx
            .principal
            .customer_id()
            .map(str::to_owned)
            .ok_or_else(not_signed_in)?;
        let a = repo::account(tx, &me).await?.ok_or_else(not_signed_in)?;
        if let Some(until) = locked_until(tx, &a) {
            return Err(ToolError::new(
                REAUTHENTICATION_LOCKED,
                "Too many wrong passwords; try again later or reset the password.",
            )
            .detail(json!({"until": tx.time.format(until)})));
        }
        match (
            &a.password_hash,
            &input.current_password,
            &input.verification_token,
        ) {
            (Some(_), Some(current), _) => {
                if !check_password(tx, &a, current).await? {
                    return Err(attempt_recorded(ToolError::new(
                        CURRENT_PASSWORD_WRONG,
                        "The current password is not right.",
                    )));
                }
            }
            (None, _, Some(token)) => {
                let c = usable(tx, token, "withdrawal_reauth", None, "verify_identity").await?;
                if c.customer_id.as_deref() != Some(a.id.as_str()) {
                    return Err(ToolError::new(
                        CURRENT_PASSWORD_WRONG,
                        "That verification is not for this account.",
                    ));
                }
                use_token(tx, &c.id).await?;
            }
            _ => {
                return Err(
                    ToolError::new(CURRENT_PASSWORD_WRONG, "Enter the current password.")
                        .detail(json!({"field": "current_password", "reason": "missing"})),
                );
            }
        }
        set_password(tx, &a, &input.new_password).await?;
        let keep = match tx
            .policy
            .lookup("customer.password.on_change")
            .and_then(Value::as_str)
        {
            Some("keep") => None,
            Some("revoke_all_sessions") => Some(None),
            _ => Some(tx.principal.session_id.clone()),
        };
        let revoked = match keep {
            Some(k) => {
                acom_platform::service::for_customer::end_sessions(tx, &a.id, k.as_deref()).await?
            }
            None => 0,
        };
        log_person(
            tx,
            Self::META.tool,
            ActionType::UPDATE,
            &a.id,
            json!({"password": "changed", "sessions_revoked": revoked}),
        )
        .await?;
        Ok(json!({"ok": true, "id": a.id, "sessions_revoked": revoked}))
    }
}

#[cfg(test)]
mod tests {
    use acom_contract::schema::{closed_schema_for, violations};

    use super::*;

    #[test]
    fn it_is_person_only() {
        let s = closed_schema_for::<ChangePasswordInput>();
        assert_eq!(violations(&s), Vec::<String>::new(), "{s:#}");
        assert_eq!(ChangePassword::META.method, Method::Put);
        assert_eq!(ChangePassword::META.person_scope, "human:account");
    }
}
