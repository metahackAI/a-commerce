//! `issue_api_key` (`platform.api_key.issue`): a key for an outside agent or an integration
//! client. A person always approves it; the secret is made when the approved set runs, shown once
//! in that answer, and kept only as a hash.

use acom_contract::{
    ActionType, ApprovalNeed, Audience, Change, ChangeOp, Check, Committed, Group, Kind, Method,
    OpMeta, Operation, Planned, Slot, ToolError, UndoSupport, Verification, WriteOp,
};
use acom_core::enums::ActorKind;
use acom_store::{Cx, TxCx};
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::{DUPLICATE_KEY_NAME, SCOPE_NOT_GRANTED, SECRET_PREFIX, SHOWN_PREFIX_LEN, key};
use crate::repo::{db, end_client_sessions, new_secret, role_scopes_of, sha256_hex};
use crate::role::{known_scopes, tidy};
use crate::rules::ApiKeyRules;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IssueApiKeyInput {
    /// What the key is for, unique among active keys.
    #[schemars(length(min = 1, max = 80), example = &"Outside shopping agent")]
    pub name: String,
    /// The scopes a session of the key may get, from list_permissions. An outside agent's key
    /// carries only `external:*` scopes.
    #[schemars(length(min = 1), example = ["external:read", "external:cart"])]
    pub scopes: Vec<String>,
    /// When the key stops working (store time). Never, when left out.
    #[schemars(example = &"2027-05-01T00:00")]
    pub expires: Option<String>,
}

/// What `issue_api_key` runs. The secret is not in it: it is made when the plan runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueApiKeyPlan {
    pub api_key_id: String,
    pub name: String,
    pub scopes: Vec<String>,
    pub expires: Option<DateTime<Utc>>,
    /// The operator who asked for the key.
    pub creator_id: Option<String>,
}

/// Issue an API key.
pub struct IssueApiKey;

impl Operation for IssueApiKey {
    const META: OpMeta = OpMeta {
        address: "platform.api_key.issue",
        tool: "issue_api_key",
        group: Group::Operate,
        kind: Kind::WriteApproval,
        method: Method::Post,
        path: "/v1/platform/api-keys",
        summary: "Issue an API key for an outside agent or an integration, with scopes within the issuer's roles. A platform administrator always approves it; the secret is shown once, to the approver, and never again.",
        consumes: &[],
        produces: &[Slot::one("api_key_id")],
        checks: &[
            "api_keys.scope_within_issuer",
            "api_keys.external_agent_scopes",
        ],
        errors: &[SCOPE_NOT_GRANTED, DUPLICATE_KEY_NAME],
        audience: Audience::Operator,
        action_type: ActionType::CREATE,
        person_scope: "",
    };
    type Input = IssueApiKeyInput;
    type Output = Value;
}

async fn active_named(tx: &mut TxCx, name: &str) -> Result<Option<String>, ToolError> {
    sqlx::query_scalar(
        "SELECT id FROM platform.api_key
         WHERE tenant_id = $1 AND lower(name) = lower($2) AND api_key_status = 'active'",
    )
    .bind(tx.tenant.as_str())
    .bind(name)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)
}

fn not_granted(scopes: &[String], why: &str) -> ToolError {
    ToolError::new(
        SCOPE_NOT_GRANTED,
        format!("A key cannot get {}: {why}", scopes.join(", ")),
    )
    .detail(json!({"field": "scopes", "items": scopes, "reason": "scope"}))
    .next("list_permissions")
}

impl WriteOp<Cx> for IssueApiKey {
    type Plan = IssueApiKeyPlan;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(
        &self,
        cx: &Cx,
        input: IssueApiKeyInput,
    ) -> Result<Planned<IssueApiKeyPlan>, ToolError> {
        crate::require_admin(cx, Some("list_roles"))?;
        let name = crate::required_text("name", &input.name)?;
        let scopes = tidy(&input.scopes);
        if scopes.is_empty() {
            return Err(ToolError::invalid_input("A key needs at least one scope.")
                .detail(json!({"field": "scopes", "reason": "missing"}))
                .next("list_permissions"));
        }
        let unknown: Vec<String> = match known_scopes(cx, &scopes) {
            Ok(_) => Vec::new(),
            Err(e) => e
                .detail
                .as_ref()
                .and_then(|d| d.get("items"))
                .and_then(|v| serde_json::from_value(v.clone()).ok())
                .unwrap_or_default(),
        };
        if !unknown.is_empty() {
            return Err(not_granted(
                &unknown,
                "they are not scopes this store knows.",
            ));
        }
        // A key is not a person: it never acts as one, drafts change sets, or moves the clock
        // (docs/backend/gateway.md §1).
        let personal: Vec<String> = scopes
            .iter()
            .filter(|s| {
                s.starts_with("human:") || s.starts_with("admin:") || s.starts_with("changeset:")
            })
            .cloned()
            .collect();
        if !personal.is_empty() {
            return Err(not_granted(
                &personal,
                "they belong to people and our agent, never to a key.",
            ));
        }
        let expires = crate::parse_time(cx, "expires", input.expires.as_deref())?;
        if expires.is_some_and(|e| e <= cx.clock.now()) {
            return Err(ToolError::invalid_input("`expires` must be in the future.")
                .detail(json!({"field": "expires", "reason": "past"})));
        }
        let rules = ApiKeyRules::from_policy(&cx.policy)?;
        let p = cx.principal.clone();
        // The issuer: the operator who asks, or the one our agent acts for. When the change-set
        // runner previews again at approval, the issuer was checked when the item was made.
        let issuer = (p.subject.starts_with("adm_")
            && matches!(p.kind, ActorKind::Operator | ActorKind::AgentOnBehalf))
        .then(|| p.subject.clone());
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        if let Some(existing) = active_named(&mut tx, &name).await? {
            return Err(ToolError::new(
                DUPLICATE_KEY_NAME,
                format!("The active key {existing} is already named {name}."),
            )
            .detail(json!({"entity": "api_key", "field": "name", "existing_id": existing}))
            .next("list_api_keys"));
        }
        let within = match (&issuer, rules.scope_within_issuer) {
            (Some(id), true) => {
                let held = role_scopes_of(&mut tx, id).await?;
                let beyond: Vec<String> = scopes
                    .iter()
                    .filter(|s| !held.contains(*s))
                    .cloned()
                    .collect();
                if !beyond.is_empty() {
                    return Err(not_granted(&beyond, "the issuer's roles do not have them."));
                }
                format!("All {} scopes are within {id}'s roles", scopes.len())
            }
            (None, true) => "Checked against the issuer's roles when it was previewed".into(),
            (_, false) => "The store does not limit keys to the issuer's roles".into(),
        };
        tx.commit().await.map_err(ToolError::from)?;
        let outside: Vec<&String> = scopes
            .iter()
            .filter(|s| s.starts_with("external:"))
            .collect();
        let allowed = &rules.external_agent_scopes;
        let stray: Vec<&String> = scopes.iter().filter(|s| !allowed.contains(*s)).collect();
        let external_check = if outside.is_empty() {
            Check {
                rule: "api_keys.external_agent_scopes".into(),
                passed: true,
                detail: Some("An integration key; no outside agent scopes".into()),
                fix_tool: None,
            }
        } else if stray.is_empty() {
            Check {
                rule: "api_keys.external_agent_scopes".into(),
                passed: true,
                detail: Some(format!(
                    "An outside agent's key with {} of the allowed scopes",
                    outside.len()
                )),
                fix_tool: None,
            }
        } else {
            Check {
                rule: "api_keys.external_agent_scopes".into(),
                passed: false,
                detail: Some(format!(
                    "An outside agent's key may carry only {}; drop {}",
                    allowed.join(", "),
                    stray
                        .iter()
                        .map(|s| s.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                )),
                fix_tool: Some("issue_api_key".into()),
            }
        };
        let checks = vec![
            Check {
                rule: "api_keys.scope_within_issuer".into(),
                passed: true,
                detail: Some(within),
                fix_tool: None,
            },
            external_check,
        ];
        let plan = IssueApiKeyPlan {
            api_key_id: format!("key_{}", acom_core::id::new_suffix()),
            name,
            scopes,
            expires,
            creator_id: issuer,
        };
        let expires_text = plan.expires.map(|e| cx.time.format(e));
        Ok(Planned {
            changes: vec![Change {
                op: ChangeOp::Create,
                entity: "api_key".into(),
                id: String::new(),
                from: Value::Null,
                to: json!({"name": plan.name, "scopes": plan.scopes, "expires": expires_text}),
                note: None,
            }],
            checks,
            basis: Vec::new(),
            approval: ApprovalNeed::Needed {
                reason: format!(
                    "Issues a key that can sign in with {}{}",
                    plan.scopes.join(", "),
                    expires_text.as_deref().map_or_else(
                        || ", with no end date".to_owned(),
                        |e| format!(" until {e}")
                    )
                ),
            },
            plan,
        })
    }

    async fn commit(&self, tx: &mut TxCx, plan: &IssueApiKeyPlan) -> Result<Committed, ToolError> {
        let mut produced = Map::new();
        produced.insert("api_key_id".into(), json!(plan.api_key_id));
        if let Some(k) = key(tx, &plan.api_key_id).await? {
            // Ran before: the secret was shown then and is not shown again.
            produced.insert("key_prefix".into(), json!(k.key_prefix));
            return Ok(Committed {
                id: plan.api_key_id.clone(),
                produced,
            });
        }
        if let Some(existing) = active_named(tx, &plan.name).await? {
            return Err(ToolError::new(
                DUPLICATE_KEY_NAME,
                format!("The active key {existing} is already named {}.", plan.name),
            )
            .detail(json!({"entity": "api_key", "field": "name", "existing_id": existing}))
            .next("list_api_keys"));
        }
        let secret = new_secret(SECRET_PREFIX);
        let prefix: String = secret.chars().take(SHOWN_PREFIX_LEN).collect();
        sqlx::query(
            "INSERT INTO platform.api_key (tenant_id, id, name, key_prefix, secret_hash, scopes,
               api_key_status, expires, creator_id) VALUES ($1, $2, $3, $4, $5, $6, 'active', $7, $8)",
        )
        .bind(tx.tenant.as_str())
        .bind(&plan.api_key_id)
        .bind(&plan.name)
        .bind(&prefix)
        .bind(sha256_hex(&secret))
        .bind(&plan.scopes)
        .bind(plan.expires)
        .bind(plan.creator_id.as_deref())
        .execute(&mut *tx.tx)
        .await
        .map_err(db)?;
        produced.insert("key_prefix".into(), json!(prefix));
        produced.insert(
            acom_changeset::SHOWN_ONCE.into(),
            json!({"secret": secret,
                   "note": "Copy the secret now: it is shown this once and never kept."}),
        );
        Ok(Committed {
            id: plan.api_key_id.clone(),
            produced,
        })
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &IssueApiKeyPlan,
        _done: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let k = key(&mut tx, &plan.api_key_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let mut mismatches = Vec::new();
        match k {
            None => mismatches.push(format!("{} does not exist", plan.api_key_id)),
            Some(k) => {
                let mut scopes = k.scopes.clone();
                scopes.sort();
                if k.api_key_status != "active" {
                    mismatches.push(format!("{} is {}", k.id, k.api_key_status));
                }
                if scopes != plan.scopes || k.name != plan.name || k.expires != plan.expires {
                    mismatches.push(format!("{} is not as previewed", k.id));
                }
            }
        }
        Ok(Verification {
            matched: mismatches.is_empty(),
            mismatches,
        })
    }

    /// Undoing the issue revokes the key and ends its sessions.
    async fn undo(
        &self,
        tx: &mut TxCx,
        plan: &IssueApiKeyPlan,
        _done: &Committed,
    ) -> Result<(), ToolError> {
        let now = tx.clock.now();
        sqlx::query(
            "UPDATE platform.api_key SET api_key_status = 'revoked', revoked_at = $3,
               revocation_reason = 'The change set that issued it was undone.',
               updated_at = now(), version = version + 1
             WHERE tenant_id = $1 AND id = $2 AND api_key_status = 'active'",
        )
        .bind(tx.tenant.as_str())
        .bind(&plan.api_key_id)
        .bind(now)
        .execute(&mut *tx.tx)
        .await
        .map_err(db)?;
        end_client_sessions(tx, &plan.api_key_id, now).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use acom_contract::schema::{closed_schema_for, violations};

    fn input(v: Value) -> IssueApiKeyInput {
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn the_input_is_closed() {
        let s = closed_schema_for::<IssueApiKeyInput>();
        assert!(violations(&s).is_empty(), "{s:#}");
        assert_eq!(s["required"], json!(["name", "scopes"]));
    }

    #[tokio::test]
    async fn a_key_never_gets_a_persons_scopes() {
        let cx = crate::testdb::offline("adm_admin", acom_auth::scopes::admin());
        let e = IssueApiKey
            .preview(
                &cx,
                input(json!({"name": "Robot", "scopes": ["external:read", "human:approve"]})),
            )
            .await
            .unwrap_err();
        assert_eq!(
            (e.code.as_ref(), e.next_tool.as_deref()),
            ("scope_not_granted", Some("list_permissions"))
        );
        assert_eq!(e.detail.unwrap()["items"], json!(["human:approve"]));
        let e = IssueApiKey
            .preview(&cx, input(json!({"name": "Robot", "scopes": ["root:all"]})))
            .await
            .unwrap_err();
        assert_eq!(e.code, "scope_not_granted");
        let e = IssueApiKey
            .preview(
                &cx,
                input(json!({"name": "Robot", "scopes": ["external:read"],
                             "expires": "2026-01-01T00:00"})),
            )
            .await
            .unwrap_err();
        assert_eq!(e.detail.unwrap()["reason"], "past");
    }

    #[tokio::test]
    #[ignore = "integration: needs the seeded demo database"]
    async fn issues_within_the_issuers_roles_shows_the_secret_once_and_keeps_its_hash() {
        let cx = crate::testdb::admin().await;
        // An issuer gives only what their own roles hold: SJ's roles cannot write sales.
        let sj = crate::testdb::cx_with(
            "adm_sj",
            ActorKind::AgentOnBehalf,
            [crate::ADMIN_SCOPE.to_owned()].into(),
        )
        .await;
        let e = IssueApiKey
            .preview(
                &sj,
                input(json!({"name": "Test order sync", "scopes": ["tools:write:sales"]})),
            )
            .await
            .unwrap_err();
        assert_eq!(e.code, "scope_not_granted");
        assert_eq!(e.detail.unwrap()["items"], json!(["tools:write:sales"]));
        // Outside agents' keys carry only their own scopes.
        let mixed = IssueApiKey
            .preview(
                &cx,
                input(json!({"name": "Test agent key",
                             "scopes": ["external:read", "tools:write:sales"]})),
            )
            .await
            .unwrap();
        assert!(!mixed.checks[1].passed);
        assert_eq!(mixed.checks[1].fix_tool.as_deref(), Some("issue_api_key"));
        let e = IssueApiKey
            .preview(
                &cx,
                input(json!({"name": "Outside shopping agent", "scopes": ["tools:read:catalog"]})),
            )
            .await
            .unwrap_err();
        assert_eq!(e.code, "duplicate_key_name");
        let p = IssueApiKey
            .preview(
                &cx,
                input(json!({"name": "Test sheet sync",
                             "scopes": ["tools:read:catalog", "tools:write:catalog"],
                             "expires": "2027-05-01T00:00"})),
            )
            .await
            .unwrap();
        assert!(p.checks.iter().all(|c| c.passed), "{:?}", p.checks);
        assert!(matches!(p.approval, ApprovalNeed::Needed { .. }));
        // The runner previews again at approval: the same changes, and the checks still pass.
        let runner = cx.as_system(acom_changeset::RUNNER);
        let again = IssueApiKey
            .preview(
                &runner,
                input(json!({"name": "Test sheet sync",
                             "scopes": ["tools:read:catalog", "tools:write:catalog"],
                             "expires": "2027-05-01T00:00"})),
            )
            .await
            .unwrap();
        assert_eq!(again.changes, p.changes);
        assert!(again.checks.iter().all(|c| c.passed));
        // Run in a transaction that is rolled back: the database ends as it was.
        let mut tx = cx.begin().await.unwrap();
        let done = IssueApiKey.commit(&mut tx, &p.plan).await.unwrap();
        let secret = done.produced["shown_once"]["secret"]
            .as_str()
            .unwrap()
            .to_owned();
        assert!(secret.starts_with("ak_live_"));
        let stored: (String, String) = sqlx::query_as(
            "SELECT secret_hash, key_prefix FROM platform.api_key WHERE tenant_id = $1 AND id = $2",
        )
        .bind("store_northwind")
        .bind(&p.plan.api_key_id)
        .fetch_one(&mut *tx.tx)
        .await
        .unwrap();
        assert_eq!(stored.0, sha256_hex(&secret));
        assert_eq!(stored.1, secret[..10]);
        let again = IssueApiKey.commit(&mut tx, &p.plan).await.unwrap();
        assert!(
            again.produced.get("shown_once").is_none(),
            "shown once only"
        );
        drop(tx);
    }
}
