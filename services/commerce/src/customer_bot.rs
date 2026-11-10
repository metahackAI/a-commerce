//! Ailism Customer Bot's internal endpoints (docs/api/platform.md#customer_bot, internal
//! endpoints): what the gateway's workers ask for and record while the bot shops. Only the
//! system subject `sys_customer_bot`, only in a demo store; never tools.
//!
//! - `POST /v1/platform/customer-bot/runs/next`: the run to work on now, with its sessions
//! - `POST /v1/platform/customer-bot/accounts`: a new member for a session that signs up
//! - `POST /v1/platform/customer-bot/sessions/{customer_bot_session_id}/steps`: steps it took
//! - `PUT  /v1/platform/customer-bot/personas/{customer_id}`: the persona it made
//! - `POST /v1/platform/customer-bot/sessions/{customer_bot_session_id}/finish`: how it ended

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use acom_contract::{ErrorClass, ErrorCode, ToolError};
use acom_store::TxCx;
use axum::Router;
use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use axum::routing::{post, put};
use serde_json::{Value, json};

/// The system subject the gateway's workers use.
const SUBJECT: &str = "sys_customer_bot";
const DEMO_ONLY: ErrorCode = ErrorCode::new("demo_mode_off", ErrorClass::InvalidState);

type Work = Box<
    dyn for<'a> FnOnce(
            &'a mut TxCx,
        )
            -> Pin<Box<dyn Future<Output = Result<Value, ToolError>> + Send + 'a>>
        + Send,
>;

/// Runs `work` in one transaction for the bot's own caller, in a demo store only.
async fn in_tx(api: &acom_http::Api, caller: &acom_http::Caller, work: Work) -> Response {
    let refuse = |e: ToolError| acom_http::ApiError(e).into_response();
    if caller.principal.kind != acom_core::enums::ActorKind::System
        || caller.principal.subject != SUBJECT
    {
        return refuse(
            ToolError::new(
                ErrorCode::FORBIDDEN,
                "Only Ailism Customer Bot's workers record here.",
            )
            .detail(json!({"reason": "actor_kind", "required": SUBJECT})),
        );
    }
    if !api.runtime.demo_mode() {
        return refuse(
            ToolError::new(DEMO_ONLY, "Ailism Customer Bot shops only in a demo store.")
                .detail(json!({"reason": "demo_mode_off"})),
        );
    }
    let cx = match api
        .runtime
        .context(caller.principal.clone(), caller.request_id.clone())
        .await
    {
        Ok(cx) => cx,
        Err(e) => return refuse(e),
    };
    let mut tx = match cx.begin().await {
        Ok(tx) => tx,
        Err(e) => return refuse(ToolError::from(e)),
    };
    match work(&mut tx).await {
        Ok(out) => match tx.commit().await {
            Ok(()) => axum::Json(out).into_response(),
            Err(e) => refuse(ToolError::from(e)),
        },
        Err(e) => refuse(e),
    }
}

/// The run to work on now: a queued one, or a scheduled one whose time has come. Its sessions
/// are opened here: `workers` of them, some signing up a new account by `new_account_percent`,
/// the others shopping as members drawn at random (never the scenes' cast, never a member
/// another session is shopping as).
async fn next_run(tx: &mut TxCx) -> Result<Value, ToolError> {
    let Some(run) = acom_platform::customer_bot_record::claim_next_run(tx).await? else {
        return Ok(json!({"run": null}));
    };
    let workers = run.workers.max(1) as usize;
    // Which sessions sign up: an even spread by the percentage, never all of them by chance.
    let created = (workers * run.new_account_percent as usize + 50) / 100;
    let mut never = run.never_shop_as.clone();
    never.extend(acom_platform::customer_bot_record::members_shopping_now(tx).await?);
    let members = acom_customer::shopper_bot::pick_members(tx, workers - created, &never).await?;
    let mut accounts: Vec<Option<String>> = members.into_iter().map(Some).collect();
    // Too few members to draw from: the rest sign up.
    accounts.resize(workers, None);
    let sessions =
        acom_platform::customer_bot_record::open_sessions(tx, &run.id, &accounts).await?;
    Ok(
        json!({"run": {"id": run.id, "trigger": run.trigger, "workers": run.workers,
                      "settings": run.settings, "sessions": sessions}}),
    )
}

pub fn routes(api: Arc<acom_http::Api>) -> Router {
    Router::new()
        .route(
            "/v1/platform/customer-bot/runs/next",
            post(
                |State(api): State<Arc<acom_http::Api>>, caller: acom_http::Caller| async move {
                    in_tx(&api, &caller, Box::new(|tx| Box::pin(next_run(tx)))).await
                },
            ),
        )
        .route(
            "/v1/platform/customer-bot/accounts",
            post(
                |State(api): State<Arc<acom_http::Api>>,
                 caller: acom_http::Caller,
                 axum::Json(body): axum::Json<Value>| async move {
                    in_tx(
                        &api,
                        &caller,
                        Box::new(move |tx| {
                            Box::pin(
                                async move { acom_customer::shopper_bot::sign_up(tx, &body).await },
                            )
                        }),
                    )
                    .await
                },
            ),
        )
        .route(
            "/v1/platform/customer-bot/sessions/{customer_bot_session_id}/steps",
            post(
                |State(api): State<Arc<acom_http::Api>>,
                 caller: acom_http::Caller,
                 Path(session): Path<String>,
                 axum::Json(body): axum::Json<Value>| async move {
                    in_tx(
                        &api,
                        &caller,
                        Box::new(move |tx| {
                            Box::pin(async move {
                                acom_platform::customer_bot_record::append(tx, &session, &body)
                                    .await
                            })
                        }),
                    )
                    .await
                },
            ),
        )
        .route(
            "/v1/platform/customer-bot/sessions/{customer_bot_session_id}/finish",
            post(
                |State(api): State<Arc<acom_http::Api>>,
                 caller: acom_http::Caller,
                 Path(session): Path<String>,
                 axum::Json(body): axum::Json<Value>| async move {
                    in_tx(
                        &api,
                        &caller,
                        Box::new(move |tx| {
                            Box::pin(async move {
                                acom_platform::customer_bot_record::finish_session(
                                    tx, &session, &body,
                                )
                                .await
                            })
                        }),
                    )
                    .await
                },
            ),
        )
        .route(
            "/v1/platform/customer-bot/personas/{customer_id}",
            put(
                |State(api): State<Arc<acom_http::Api>>,
                 caller: acom_http::Caller,
                 Path(customer): Path<String>,
                 axum::Json(body): axum::Json<Value>| async move {
                    in_tx(
                        &api,
                        &caller,
                        Box::new(move |tx| {
                            Box::pin(async move {
                                acom_platform::customer_bot_record::save_persona(
                                    tx, &customer, &body,
                                )
                                .await
                            })
                        }),
                    )
                    .await
                },
            ),
        )
        .with_state(api)
}
