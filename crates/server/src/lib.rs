//! What every A-Commerce service binary shares, so all of them behave the same in a container:
//!
//! - JSON logs (`ACOM_LOG_FORMAT=json`, the default) filtered by `RUST_LOG`;
//! - `/healthz` (the process is up), `/readyz` (it can serve: dependencies reachable), `/metrics`
//!   (Prometheus text);
//! - a request id on every request (`x-request-id`, generated when missing) and request metrics;
//! - graceful shutdown on SIGTERM and Ctrl-C, which is how Kubernetes and Compose stop containers;
//! - `healthcheck`, a tiny HTTP probe for images without a shell (distroless).
//!
//! Design: `infra/README.md` (serving architecture).

use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::process::ExitCode;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use axum::Router;
use axum::extract::{Request, State};
use axum::http::{HeaderValue, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};
use tracing_subscriber::EnvFilter;

/// Header carrying the request id across the gateway, the services, and the logs.
pub const REQUEST_ID: &str = "x-request-id";

/// Sets up logging once. `ACOM_LOG_FORMAT=pretty` gives human-readable logs for local runs.
pub fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let pretty = std::env::var("ACOM_LOG_FORMAT").is_ok_and(|v| v == "pretty");
    let builder = tracing_subscriber::fmt().with_env_filter(filter);
    let result = if pretty {
        builder.try_init()
    } else {
        builder.json().flatten_event(true).try_init()
    };
    if result.is_err() {
        // Already initialized (tests); nothing to do.
    }
}

fn metrics_handle() -> &'static PrometheusHandle {
    static HANDLE: OnceLock<PrometheusHandle> = OnceLock::new();
    HANDLE.get_or_init(|| {
        PrometheusBuilder::new()
            .install_recorder()
            .unwrap_or_else(|_| PrometheusBuilder::new().build_recorder().handle())
    })
}

/// What a service checks before it says it can take traffic.
pub trait Readiness: Send + Sync + 'static {
    /// `Err(reason)` keeps the service out of rotation with that reason in `/readyz`.
    fn check(&self) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + '_>>;
}

/// For services with nothing to wait for.
#[derive(Debug, Clone, Copy)]
pub struct AlwaysReady;

impl Readiness for AlwaysReady {
    fn check(&self) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + '_>> {
        Box::pin(async { Ok(()) })
    }
}

#[derive(Clone)]
struct Ops {
    service: &'static str,
    readiness: Arc<dyn Readiness>,
}

async fn healthz(State(ops): State<Ops>) -> impl IntoResponse {
    axum::Json(serde_json::json!({"status": "ok", "service": ops.service}))
}

async fn readyz(State(ops): State<Ops>) -> Response {
    match ops.readiness.check().await {
        Ok(()) => axum::Json(serde_json::json!({"status": "ready", "service": ops.service})).into_response(),
        Err(reason) => (
            StatusCode::SERVICE_UNAVAILABLE,
            axum::Json(serde_json::json!({"status": "not_ready", "service": ops.service, "reason": reason})),
        )
            .into_response(),
    }
}

async fn metrics_text() -> impl IntoResponse {
    (
        [(
            axum::http::header::CONTENT_TYPE,
            "text/plain; version=0.0.4",
        )],
        metrics_handle().render(),
    )
}

/// `/healthz`, `/readyz`, `/metrics` for one service.
pub fn ops_router(service: &'static str, readiness: Arc<dyn Readiness>) -> Router {
    metrics_handle();
    Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .route("/metrics", get(metrics_text))
        .with_state(Ops { service, readiness })
}

/// Gives every request an id (keeps the caller's), logs it, and counts it.
async fn observe(service: &'static str, mut req: Request, next: Next) -> Response {
    let id = req
        .headers()
        .get(REQUEST_ID)
        .and_then(|v| v.to_str().ok())
        .filter(|v| !v.is_empty() && v.len() <= 128)
        .map(str::to_owned)
        .unwrap_or_else(|| {
            format!(
                "req_{}",
                ulid::Ulid::generate().to_string().to_ascii_lowercase()
            )
        });
    if let Ok(value) = HeaderValue::from_str(&id) {
        req.headers_mut().insert(REQUEST_ID, value.clone());
    }
    let method = req.method().clone();
    let path = req.uri().path().to_owned();
    let started = Instant::now();
    let mut res = next.run(req).await;
    let elapsed = started.elapsed();
    if let Ok(value) = HeaderValue::from_str(&id) {
        res.headers_mut().insert(REQUEST_ID, value);
    }
    let status = res.status().as_u16();
    let route_class = if path.starts_with("/healthz")
        || path.starts_with("/readyz")
        || path.starts_with("/metrics")
    {
        "ops"
    } else {
        "app"
    };
    metrics::counter!("http_requests_total", "service" => service, "method" => method.to_string(),
        "status" => status.to_string(), "route" => route_class)
    .increment(1);
    metrics::histogram!("http_request_duration_seconds", "service" => service, "route" => route_class)
        .record(elapsed.as_secs_f64());
    if route_class == "app" {
        tracing::info!(request_id = %id, %method, %path, status, ms = elapsed.as_millis() as u64, "request");
    }
    res
}

/// Adds request ids, logs, and metrics to a service's router.
pub fn observed(service: &'static str, router: Router) -> Router {
    router.layer(middleware::from_fn(move |req, next| {
        observe(service, req, next)
    }))
}

/// Resolves on SIGTERM (Kubernetes, Compose) or Ctrl-C.
pub async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut s) => {
                s.recv().await;
            }
            Err(_) => futures::future::pending::<()>().await,
        }
    };
    #[cfg(not(unix))]
    let terminate = futures::future::pending::<()>();
    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }
    tracing::info!("shutdown signal received; finishing in-flight requests");
}

/// Binds `addr` and serves until a shutdown signal, finishing in-flight requests first.
pub async fn serve(service: &'static str, addr: &str, router: Router) -> std::io::Result<()> {
    let listener = tokio::net::TcpListener::bind(addr).await?;
    let local: SocketAddr = listener.local_addr()?;
    tracing::info!(service, addr = %local, version = env!("CARGO_PKG_VERSION"), "listening");
    axum::serve(listener, observed(service, router))
        .with_graceful_shutdown(shutdown_signal())
        .await
}

/// `<binary> healthcheck`: GET `/healthz` on the local port of `addr`; exit 0 when it answers 200.
/// Used as the container health check, since the runtime images have no shell or curl.
pub async fn healthcheck(addr: &str) -> ExitCode {
    let port = addr.rsplit(':').next().unwrap_or("80");
    let url = format!("http://127.0.0.1:{port}/healthz");
    let client: hyper_util::client::legacy::Client<_, http_body_util::Empty<hyper::body::Bytes>> =
        hyper_util::client::legacy::Client::builder(hyper_util::rt::TokioExecutor::new())
            .build_http();
    let Ok(uri) = url.parse::<hyper::Uri>() else {
        return ExitCode::FAILURE;
    };
    match tokio::time::timeout(Duration::from_secs(3), client.get(uri)).await {
        Ok(Ok(res)) if res.status() == StatusCode::OK => ExitCode::SUCCESS,
        _ => ExitCode::FAILURE,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use tower::ServiceExt;

    struct NotYet;
    impl Readiness for NotYet {
        fn check(&self) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + '_>> {
            Box::pin(async { Err("migrations pending".to_owned()) })
        }
    }

    #[tokio::test]
    async fn ops_routes_answer_and_requests_get_ids() {
        let app = observed("test", ops_router("test", Arc::new(NotYet)));
        let res = app
            .clone()
            .oneshot(Request::get("/healthz").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        assert!(
            res.headers()[REQUEST_ID]
                .to_str()
                .unwrap()
                .starts_with("req_")
        );

        let res = app
            .clone()
            .oneshot(
                Request::get("/readyz")
                    .header(REQUEST_ID, "req_given")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(res.headers()[REQUEST_ID], "req_given");

        let res = app
            .oneshot(Request::get("/metrics").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
    }
}
