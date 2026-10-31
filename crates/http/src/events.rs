//! `GET /v1/platform/events`: the store's progress events as server-sent events
//! (`change_set.updated`, `task.updated`, `job.updated`). The engine writes them with
//! `pg_notify('acom_events', …)` inside the transaction that made the change, so an event is
//! sent only once the change is committed.
//!
//! A `LISTEN` holds its connection for as long as it runs, so the process has one and every stream
//! reads from it. With a listener per stream, a few open tabs would take the whole pool and every
//! other request would fail while they stay open.

use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::extract::State;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use futures::{Stream, StreamExt};
use serde_json::Value;
use sqlx::PgPool;
use sqlx::postgres::PgListener;
use tokio::sync::{OnceCell, broadcast};

use crate::{Api, ApiError, Caller};

const CHANNEL: &str = "acom_events";
/// Events a slow stream can fall behind by before it misses some. Events say "look again", so a
/// stream that misses a few still shows the latest state at the next one.
const BACKLOG: usize = 1024;
/// How long to wait after the listener failed to reconnect.
const RETRY: Duration = Duration::from_secs(1);

pub(crate) fn routes() -> Router<Arc<Api>> {
    Router::new().route("/v1/platform/events", get(events))
}

/// One event, read once for all the streams.
#[derive(Debug, PartialEq)]
struct StoreEvent {
    tenant: String,
    kind: String,
    /// The payload without `tenant` and `type`, which stay inside the store.
    data: String,
}

impl StoreEvent {
    fn parse(payload: &str) -> Option<Self> {
        let mut v: Value = serde_json::from_str(payload).ok()?;
        let tenant = v.get("tenant")?.as_str()?.to_owned();
        let kind = v.get("type")?.as_str()?.to_owned();
        if let Value::Object(o) = &mut v {
            o.remove("tenant");
            o.remove("type");
        }
        Some(Self {
            tenant,
            kind,
            data: v.to_string(),
        })
    }
}

type Sender = broadcast::Sender<Arc<StoreEvent>>;

/// The process's one listener, started by the first stream.
#[derive(Debug, Default)]
pub struct EventHub(OnceCell<Sender>);

impl EventHub {
    /// A receiver that has every event from now on. The listener is `LISTEN`ing by the time this
    /// returns; if it cannot start, the next stream tries again.
    async fn subscribe(
        &self,
        pool: &PgPool,
    ) -> Result<broadcast::Receiver<Arc<StoreEvent>>, sqlx::Error> {
        let sender = self
            .0
            .get_or_try_init(|| async {
                let mut listener = PgListener::connect_with(pool).await?;
                listener.listen(CHANNEL).await?;
                let (sender, _) = broadcast::channel(BACKLOG);
                tokio::spawn(listen(listener, sender.clone()));
                Ok::<_, sqlx::Error>(sender)
            })
            .await?;
        Ok(sender.subscribe())
    }
}

/// Passes on every notification until the pool closes. `recv` reconnects on its own when the
/// connection is lost, and an error means that attempt failed.
async fn listen(mut listener: PgListener, sender: Sender) {
    loop {
        match listener.recv().await {
            Ok(n) => {
                if let Some(event) = StoreEvent::parse(n.payload()) {
                    // No receivers is fine: nobody is watching right now.
                    let _ = sender.send(Arc::new(event));
                }
            }
            Err(sqlx::Error::PoolClosed) => return,
            Err(e) => {
                tracing::warn!(error = %e, "store events listener could not reconnect");
                tokio::time::sleep(RETRY).await;
            }
        }
    }
}

/// The events of one tenant.
fn of_tenant(
    receiver: broadcast::Receiver<Arc<StoreEvent>>,
    tenant: String,
) -> impl Stream<Item = Arc<StoreEvent>> {
    futures::stream::unfold((receiver, tenant), |(mut receiver, tenant)| async move {
        loop {
            match receiver.recv().await {
                Ok(e) if e.tenant == tenant => return Some((e, (receiver, tenant))),
                Ok(_) => {}
                Err(broadcast::error::RecvError::Lagged(missed)) => {
                    tracing::warn!(missed, "a store events stream fell behind");
                }
                Err(broadcast::error::RecvError::Closed) => return None,
            }
        }
    })
}

async fn events(State(api): State<Arc<Api>>, caller: Caller) -> Response {
    match stream(&api, &caller).await {
        Ok(s) => Sse::new(s)
            .keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
            .into_response(),
        Err(e) => e.into_response(),
    }
}

async fn stream(
    api: &Api,
    caller: &Caller,
) -> Result<impl Stream<Item = Result<Event, Infallible>> + use<>, ApiError> {
    caller.require("tools:read:platform")?;
    let tenant = caller.principal.tenant.as_str().to_owned();
    let receiver = api
        .events
        .subscribe(api.runtime.db.pool())
        .await
        .map_err(|e| ApiError(acom_store::StoreError::from(e).into()))?;
    Ok(of_tenant(receiver, tenant).map(|e| Ok(Event::default().event(&e.kind).data(&e.data))))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(tenant: &str, id: u32) -> Arc<StoreEvent> {
        Arc::new(StoreEvent {
            tenant: tenant.into(),
            kind: "task.updated".into(),
            data: format!("{{\"task_id\":{id}}}"),
        })
    }

    async fn data_of(stream: impl Stream<Item = Arc<StoreEvent>>) -> Vec<String> {
        stream.map(|e| e.data.clone()).collect().await
    }

    #[test]
    fn an_event_keeps_its_tenant_and_type_out_of_the_data() {
        let e = StoreEvent::parse(r#"{"tenant":"store_a","type":"task.updated","task_id":"t1"}"#);
        assert_eq!(
            e,
            Some(StoreEvent {
                tenant: "store_a".into(),
                kind: "task.updated".into(),
                data: r#"{"task_id":"t1"}"#.into(),
            })
        );
    }

    #[test]
    fn a_payload_without_a_tenant_or_type_is_not_an_event() {
        assert_eq!(StoreEvent::parse(r#"{"type":"task.updated"}"#), None);
        assert_eq!(StoreEvent::parse(r#"{"tenant":"store_a"}"#), None);
        assert_eq!(StoreEvent::parse("not json"), None);
    }

    #[tokio::test]
    async fn each_stream_gets_every_event_of_its_own_tenant_only() {
        let (sender, _) = broadcast::channel(8);
        let a = of_tenant(sender.subscribe(), "store_a".into());
        let also_a = of_tenant(sender.subscribe(), "store_a".into());
        let b = of_tenant(sender.subscribe(), "store_b".into());
        for e in [
            event("store_a", 1),
            event("store_b", 2),
            event("store_a", 3),
        ] {
            sender.send(e).unwrap();
        }
        drop(sender);
        assert_eq!(data_of(a).await, [r#"{"task_id":1}"#, r#"{"task_id":3}"#]);
        assert_eq!(
            data_of(also_a).await,
            [r#"{"task_id":1}"#, r#"{"task_id":3}"#]
        );
        assert_eq!(data_of(b).await, [r#"{"task_id":2}"#]);
    }

    #[tokio::test]
    async fn a_stream_that_falls_behind_goes_on_with_the_latest() {
        let (sender, _) = broadcast::channel(2);
        let slow = of_tenant(sender.subscribe(), "store_a".into());
        for id in 1..=5 {
            sender.send(event("store_a", id)).unwrap();
        }
        drop(sender);
        assert_eq!(
            data_of(slow).await,
            [r#"{"task_id":4}"#, r#"{"task_id":5}"#]
        );
    }
}
