use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::http::StatusCode;
use axum::response::IntoResponse;
use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;
use tokio::signal;
use tokio_util::sync::CancellationToken;

use crate::config::{Config, RateLimitConfig};
use crate::ctx::Ctx;
use crate::error::{Error, Result};
use crate::router::Router;
use crate::stack::{self, StackConfig};
use crate::task::Task;

/// Type alias for boxed future returned by readiness checks.
pub type BoxFuture<'a, T> = std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send + 'a>>;

/// A registered readiness check callback.
pub type ReadinessCheck = Arc<dyn Fn() -> BoxFuture<'static, Result<()>> + Send + Sync>;

/// Standard probe response body for `/health` and `/ready`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProbeResponse {
    pub status: String,
}

impl ProbeResponse {
    pub fn ok() -> Self {
        ProbeResponse {
            status: "ok".to_string(),
        }
    }

    pub fn ready() -> Self {
        ProbeResponse {
            status: "ready".to_string(),
        }
    }
}

/// How long to wait for background tasks to finish after shutdown is signalled
/// before giving up on them.
const DRAIN_DEADLINE: Duration = Duration::from_secs(10);

/// The application. One per process. Built once in `main`, then `run`.
///
/// `App` owns the composition root: state, routes, and background tasks. It reads
/// [`Config`] from the environment, installs the standard middleware stack
/// (tracing span + request id, timeout, body limit, cancellation), binds the
/// listener, starts tasks, serves, and shuts everything down gracefully on
/// SIGINT/SIGTERM.
///
/// ```ignore
/// #[tokio::main]
/// async fn main() -> hephos::Result<()> {
///     let state = AppState::init().await?;
///     App::new(state)
///         .routes(api::routes())
///         .task(OutboxDrain)
///         .run()
///         .await
/// }
/// ```
pub struct App<S> {
    pub(crate) state: S,
    router: Router<S>,
    tasks: Vec<Box<dyn ErasedTask<S>>>,
    readiness: Option<ReadinessCheck>,
    pub(crate) rate_limit: Option<RateLimitConfig>,
    pub(crate) concurrency_limit: Option<usize>,
    pub(crate) max_connections: Option<usize>,
    pub(crate) connection_caps: Vec<crate::conn::ConnectionCap>,
    pub(crate) otlp_endpoint: Option<String>,
    pub(crate) traces_enabled: Option<bool>,
    pub(crate) metrics: Option<crate::telemetry::RedMetrics>,
}

impl<S> App<S>
where
    S: Clone + Send + Sync + 'static,
{
    pub fn new(state: S) -> Self {
        App {
            state,
            router: Router::new(),
            tasks: Vec::new(),
            readiness: None,
            rate_limit: None,
            concurrency_limit: None,
            max_connections: None,
            connection_caps: Vec::new(),
            otlp_endpoint: None,
            traces_enabled: None,
            metrics: None,
        }
    }

    /// Install the application's routes. Called once; the router is built by the
    /// app's `api::routes()` function.
    pub fn routes(mut self, router: Router<S>) -> Self {
        self.router = router;
        self
    }

    /// Register a background task. Tasks run for the lifetime of the process and
    /// are cancelled on shutdown.
    pub fn task<T: Task<S>>(mut self, task: T) -> Self {
        self.tasks.push(Box::new(task));
        self
    }

    /// Register a readiness check for `/ready`.
    ///
    /// By default, `/ready` always returns `200 OK` (process is ready). When a check
    /// is registered (typically pinging dependencies like the database), `/ready` returns
    /// `200 OK` when the check succeeds, and `503 Service Unavailable` when it fails.
    ///
    /// ```ignore
    /// let db = state.db.clone();
    /// App::new(state)
    ///     .routes(api::routes())
    ///     .readiness(move || {
    ///         let db = db.clone();
    ///         async move { db.ping().await }
    ///     })
    /// ```
    pub fn readiness<F, Fut>(mut self, check: F) -> Self
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<()>> + Send + 'static,
    {
        self.readiness = Some(Arc::new(move || Box::pin(check())));
        self
    }

    /// Configure per-IP rate limiting (requests per second and burst capacity).
    ///
    /// Overrides environment variables (`HEPHOS_RATE_LIMIT_RPS`, `HEPHOS_RATE_LIMIT_BURST`).
    /// Probes (`/health`, `/ready`) are never rate limited.
    pub fn rate_limit(mut self, rps: u64, burst: u32) -> Self {
        self.rate_limit = Some(RateLimitConfig::new(rps, burst));
        self
    }

    /// Configure concurrency limit and load shedding (maximum in-flight requests).
    ///
    /// Excess requests are rejected immediately with `503 Service Unavailable`
    /// (`"service overloaded, shedding load"`).
    /// Overrides environment variable `HEPHOS_CONCURRENCY_LIMIT`.
    /// Probes (`/health`, `/ready`) are never load-shed.
    pub fn concurrency_limit(mut self, limit: usize) -> Self {
        self.concurrency_limit = Some(limit);
        self
    }

    /// Configure maximum concurrent connections and global connection capping.
    ///
    /// Excess connections are rejected with `503 Service Unavailable`.
    /// Overrides environment variable `HEPHOS_MAX_CONNECTIONS`.
    /// Probes (`/health`, `/ready`) are never connection-capped.
    pub fn max_connections(mut self, limit: usize) -> Self {
        self.max_connections = Some(limit);
        self
    }

    /// Register an external [`crate::conn::ConnectionCap`] to be tracked and drained during graceful shutdown.
    pub fn connection_cap(mut self, cap: crate::conn::ConnectionCap) -> Self {
        self.connection_caps.push(cap);
        self
    }

    /// Configure the OTLP exporter endpoint (e.g. `http://localhost:4318`).
    ///
    /// When configured, metrics and logs are exported via standard OTLP HTTP.
    /// Overrides environment variable `HEPHOS_OTLP_ENDPOINT` / `OTEL_EXPORTER_OTLP_ENDPOINT`.
    pub fn otlp_endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.otlp_endpoint = Some(endpoint.into());
        self
    }

    /// Enable or disable distributed tracing export.
    ///
    /// Tracing export is gated (defaults to off while upstream OpenTelemetry tracing is beta).
    /// Overrides environment variable `HEPHOS_TRACES` / `HEPHOS_OTEL_TRACES`.
    pub fn traces(mut self, enabled: bool) -> Self {
        self.traces_enabled = Some(enabled);
        self
    }

    /// Provide custom RED metrics instruments.
    pub fn metrics(mut self, metrics: crate::telemetry::RedMetrics) -> Self {
        self.metrics = Some(metrics);
        self
    }

    /// Split into application router and auto-mounted probe router (`/health`, `/ready`).
    pub(crate) fn into_routers(self) -> (axum::Router<S>, axum::Router<S>) {
        let readiness = self.readiness;
        let probe_router = axum::Router::new()
            .route("/health", axum::routing::get(health_handler))
            .route(
                "/ready",
                axum::routing::get(move || ready_handler(readiness.clone())),
            );

        (self.router.into_inner(), probe_router)
    }

    /// Read config, initialize observability, start tasks, and serve until a
    /// shutdown signal, then drain in-flight requests and tasks.
    pub async fn run(self) -> Result<()> {
        let config = Config::from_env()?;
        let otlp_endpoint = self.otlp_endpoint.or(config.otlp_endpoint);
        let traces_enabled = self.traces_enabled.unwrap_or(config.traces_enabled);
        let telemetry_config = crate::telemetry::TelemetryConfig {
            otlp_endpoint,
            traces_enabled,
        };
        let telemetry_handles = crate::telemetry::init_telemetry(config.log, &telemetry_config)?;

        let shutdown = CancellationToken::new();
        let rate_limit = self.rate_limit.or(config.rate_limit);
        let concurrency_limit = self.concurrency_limit.or(config.concurrency_limit);
        let max_connections = self.max_connections.or(config.max_connections);
        let global_cap = max_connections.map(crate::conn::ConnectionCap::new);
        let metrics = self.metrics.unwrap_or_default();
        let App {
            state,
            router,
            tasks,
            readiness,
            rate_limit: _,
            concurrency_limit: _,
            max_connections: _,
            connection_caps,
            otlp_endpoint: _,
            traces_enabled: _,
            metrics: _,
        } = self;

        // Background tasks share the app's shutdown token.
        let mut handles = Vec::new();
        for task in tasks {
            let ctx = Ctx::detached(state.clone()).with_cancel(shutdown.clone());
            let name = task.name();
            handles.push(tokio::spawn(async move {
                if let Err(error) = task.run_erased(ctx).await {
                    tracing::error!(task = name, error = %error, "task failed");
                }
            }));
        }

        let stack_config = StackConfig {
            timeout: config.timeout,
            body_limit: config.body_limit,
            hsts: config.hsts,
            cors_origins: config.cors_origins.clone(),
            rate_limit,
            concurrency_limit,
            connection_cap: global_cap.clone(),
            metrics,
        };
        let probe_router = axum::Router::new()
            .route("/health", axum::routing::get(health_handler))
            .route(
                "/ready",
                axum::routing::get(move || ready_handler(readiness.clone())),
            );
        let app = stack::apply(
            router.into_inner(),
            probe_router,
            stack_config,
            shutdown.clone(),
        )
        .with_state(state);

        let listener = TcpListener::bind(config.addr)
            .await
            .map_err(|e| Error::internal(format!("bind {}: {e}", config.addr)))?;
        tracing::info!(addr = %config.addr, "hephos listening");

        let serve = axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .with_graceful_shutdown({
            let token = shutdown.clone();
            async move {
                wait_for_signal().await;
                tracing::info!("shutdown signal received");
                token.cancel();
            }
        });

        serve
            .await
            .map_err(|e| Error::internal(format!("serve: {e}")))?;

        // Drain active long-lived connections (WebSockets, SSE) and background tasks against
        // a single shared deadline (so N stuck tasks/sockets cost one deadline, not N).
        let deadline = tokio::time::Instant::now() + DRAIN_DEADLINE;
        if let Some(ref cap) = global_cap {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if let Err(error) = cap.drain(remaining).await {
                tracing::warn!(%error, "global connections did not finish within the drain deadline");
            }
        }
        for cap in connection_caps {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if let Err(error) = cap.drain(remaining).await {
                tracing::warn!(%error, "connection cap did not finish within the drain deadline");
            }
        }

        for handle in handles {
            match tokio::time::timeout_at(deadline, handle).await {
                Ok(Ok(())) => {}
                Ok(Err(join_error)) => {
                    tracing::error!(error = %join_error, "background task panicked");
                }
                Err(_) => {
                    tracing::warn!("background tasks did not finish within the drain deadline");
                    break;
                }
            }
        }

        // Flush telemetry providers (metrics, logs, traces)
        telemetry_handles.shutdown();

        Ok(())
    }
}

async fn wait_for_signal() {
    let ctrl_c = async {
        let _ = signal::ctrl_c().await;
    };

    #[cfg(unix)]
    let terminate = async {
        if let Ok(mut sig) = signal::unix::signal(signal::unix::SignalKind::terminate()) {
            sig.recv().await;
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}

// Object-safe shim so tasks of different concrete types can share one Vec.
trait ErasedTask<S>: Send + Sync {
    fn name(&self) -> &'static str;
    fn run_erased(
        self: Box<Self>,
        ctx: Ctx<S>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send>>;
}

impl<S, T> ErasedTask<S> for T
where
    S: Clone + Send + Sync + 'static,
    T: Task<S>,
{
    fn name(&self) -> &'static str {
        Task::name(self)
    }

    fn run_erased(
        self: Box<Self>,
        ctx: Ctx<S>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send>> {
        Box::pin(async move { self.run(ctx).await })
    }
}

async fn health_handler() -> (StatusCode, axum::Json<ProbeResponse>) {
    (StatusCode::OK, axum::Json(ProbeResponse::ok()))
}

async fn ready_handler(readiness: Option<ReadinessCheck>) -> axum::response::Response {
    if let Some(check) = readiness {
        match check().await {
            Ok(()) => (StatusCode::OK, axum::Json(ProbeResponse::ready())).into_response(),
            Err(Error::Unavailable(msg)) => Error::Unavailable(msg).into_response(),
            Err(err) => {
                tracing::warn!(error = %err, "readiness check failed");
                Error::unavailable("service unavailable").into_response()
            }
        }
    } else {
        (StatusCode::OK, axum::Json(ProbeResponse::ready())).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;
    use serde_json::Value;

    #[test]
    fn probe_response_serialization_and_values() {
        let ok = ProbeResponse::ok();
        assert_eq!(ok.status, "ok");
        let json_ok = serde_json::to_string(&ok).unwrap();
        let parsed_ok: ProbeResponse = serde_json::from_str(&json_ok).unwrap();
        assert_eq!(parsed_ok, ok);

        let ready = ProbeResponse::ready();
        assert_eq!(ready.status, "ready");
        let json_ready = serde_json::to_string(&ready).unwrap();
        let parsed_ready: ProbeResponse = serde_json::from_str(&json_ready).unwrap();
        assert_eq!(parsed_ready, ready);
    }

    #[tokio::test]
    async fn health_handler_returns_ok() {
        let (status, axum::Json(body)) = health_handler().await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body.status, "ok");
    }

    #[tokio::test]
    async fn ready_handler_defaults_to_ready_when_no_check() {
        let res = ready_handler(None).await;
        assert_eq!(res.status(), StatusCode::OK);
        let bytes = to_bytes(res.into_body(), usize::MAX).await.unwrap();
        let json: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(json["status"], "ready");
    }

    #[tokio::test]
    async fn ready_handler_succeeds_when_check_passes() {
        let check: ReadinessCheck = Arc::new(|| Box::pin(async { Ok(()) }));
        let res = ready_handler(Some(check)).await;
        assert_eq!(res.status(), StatusCode::OK);
        let bytes = to_bytes(res.into_body(), usize::MAX).await.unwrap();
        let json: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(json["status"], "ready");
    }

    #[tokio::test]
    async fn ready_handler_surfaces_unavailable_error() {
        let check: ReadinessCheck =
            Arc::new(|| Box::pin(async { Err(Error::unavailable("database offline")) }));
        let res = ready_handler(Some(check)).await;
        assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
        let bytes = to_bytes(res.into_body(), usize::MAX).await.unwrap();
        let json: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(json["error"]["kind"], "unavailable");
        assert_eq!(json["error"]["message"], "database offline");
    }

    #[tokio::test]
    async fn ready_handler_masks_unexpected_internal_error() {
        let check: ReadinessCheck = Arc::new(|| {
            Box::pin(async { Err(Error::internal("leaked connection error with password")) })
        });
        let res = ready_handler(Some(check)).await;
        assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
        let bytes = to_bytes(res.into_body(), usize::MAX).await.unwrap();
        let json: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(json["error"]["kind"], "unavailable");
        // Must never leak the internal error cause
        assert_eq!(json["error"]["message"], "service unavailable");
    }

    #[test]
    fn app_builder_configures_options() {
        let app = App::new("test_state".to_string())
            .rate_limit(50, 100)
            .concurrency_limit(32)
            .otlp_endpoint("http://otel.local:4318")
            .traces(true);

        assert_eq!(app.state, "test_state");
        assert_eq!(
            app.rate_limit,
            Some(crate::config::RateLimitConfig::new(50, 100))
        );
        assert_eq!(app.concurrency_limit, Some(32));
        assert_eq!(app.otlp_endpoint.as_deref(), Some("http://otel.local:4318"));
        assert_eq!(app.traces_enabled, Some(true));
    }

    struct DummyTask;

    impl Task<()> for DummyTask {
        fn name(&self) -> &'static str {
            "dummy_task"
        }

        async fn run(&self, _ctx: Ctx<()>) -> Result<()> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn erased_task_dispatch() {
        let boxed: Box<dyn ErasedTask<()>> = Box::new(DummyTask);
        assert_eq!(boxed.name(), "dummy_task");
        let ctx = Ctx::detached(());
        let res = boxed.run_erased(ctx).await;
        assert!(res.is_ok());
    }
}
