//! The standard middleware stack — the one fixed, correct Tower stack every Hephos
//! app runs, assembled here so application code never touches tower. Installing it
//! is what makes [`crate::Ctx`] honest: request id, tracing span, timeouts, body
//! limits, a default identity, and a shutdown-linked cancellation token.
//!
//! Order (outermost → innermost): observe → timeout → body-limit → cancel →
//! catch-panic → (per-route auth, if any) → handler. The order is fixed on purpose;
//! middleware ordering is a classic bug source, so it is decided once, here, and
//! nowhere else.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::Request;
use axum::http::{HeaderValue, Method, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use tokio_util::sync::CancellationToken;
use tower_governor::GovernorLayer;
use tower_governor::errors::GovernorError;
use tower_governor::governor::GovernorConfigBuilder;
use tower_governor::key_extractor::SmartIpKeyExtractor;
use tower_http::catch_panic::CatchPanicLayer;
use tower_http::cors::{Any, CorsLayer};
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::timeout::TimeoutLayer;
use tracing::Instrument;
use uuid::Uuid;

use crate::config::RateLimitConfig;
use crate::ctx::RequestId;
use crate::error::Error;
use crate::identity::Identity;

/// Knobs for the stack. Derived from [`crate::Config`] in production; defaulted in
/// the test harness.
#[derive(Debug, Clone)]
pub(crate) struct StackConfig {
    pub timeout: Duration,
    pub body_limit: usize,
    pub hsts: bool,
    pub cors_origins: Option<Vec<String>>,
    pub rate_limit: Option<RateLimitConfig>,
}

impl Default for StackConfig {
    fn default() -> Self {
        StackConfig {
            timeout: Duration::from_secs(30),
            body_limit: 2 * 1024 * 1024,
            hsts: false,
            cors_origins: None,
            rate_limit: None,
        }
    }
}

/// Wrap application and probe routes in the standard stack.
///
/// If rate limiting is configured, `app_router` is wrapped in [`GovernorLayer`]
/// before being merged with `probe_router`, ensuring probes (`/health`, `/ready`)
/// are never rate limited. `shutdown` is the process-wide token; each request gets
/// a child of it, so a graceful shutdown cancels in-flight work.
pub(crate) fn apply<S>(
    app_router: axum::Router<S>,
    probe_router: axum::Router<S>,
    config: StackConfig,
    shutdown: CancellationToken,
) -> axum::Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    let hsts = config.hsts;
    let app_router = if let Some(rl) = config.rate_limit {
        let mut builder = GovernorConfigBuilder::default()
            .key_extractor(SmartIpKeyExtractor)
            .use_headers();
        builder
            .period(Duration::from_nanos((1_000_000_000 / rl.rps).max(1)))
            .burst_size(rl.burst);
        let gov_conf = builder.finish().expect("valid rate limit config");
        let limiter = Arc::downgrade(gov_conf.limiter());
        let cancel = shutdown.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(60));
            interval.tick().await;
            loop {
                tokio::select! {
                    _ = cancel.cancelled() => break,
                    _ = interval.tick() => {
                        match limiter.upgrade() {
                            Some(limiter) => limiter.retain_recent(),
                            None => break,
                        }
                    }
                }
            }
        });
        let layer = GovernorLayer::new(gov_conf).error_handler(on_governor_error);
        app_router.layer(layer)
    } else {
        app_router
    };

    // `.layer()` applies outermost-last, so the call order below is innermost-first.
    // CORS is outermost so preflight (`OPTIONS`) is answered before auth/timeout.
    let mut router = app_router
        .merge(probe_router)
        // Innermost: a handler panic is caught here and turned into a `500`, so the
        // worker task never unwinds and the server stays up. Placed inside `observe`,
        // so the 500 still gets the request id, security headers, and span log.
        .layer(CatchPanicLayer::custom(on_panic))
        .layer(middleware::from_fn(move |req, next| {
            cancel(req, next, shutdown.clone())
        }))
        .layer(RequestBodyLimitLayer::new(config.body_limit))
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            config.timeout,
        ))
        .layer(middleware::from_fn(move |req, next| {
            observe(req, next, hsts)
        }));
    if let Some(origins) = config.cors_origins {
        router = router.layer(cors_layer(&origins));
    }
    router
}

fn on_governor_error(error: GovernorError) -> Response {
    match error {
        GovernorError::TooManyRequests { wait_time, headers } => {
            let msg = format!("rate limit exceeded; try again in {wait_time}s");
            let mut response = Error::rate_limited(msg).into_response();
            if let Some(headers) = headers {
                for (k, v) in headers {
                    if let Some(k) = k {
                        response.headers_mut().insert(k, v);
                    }
                }
            }
            response
        }
        GovernorError::UnableToExtractKey => {
            tracing::error!("rate limiter: unable to extract client key");
            Error::internal("unable to extract client ip for rate limiting").into_response()
        }
        GovernorError::Other { code, msg, headers } => {
            let msg = msg.unwrap_or_else(|| "rate limit error".to_string());
            tracing::warn!(status = %code, message = %msg, "rate limiter error");
            let mut response = (
                code,
                axum::Json(serde_json::json!({
                    "error": {
                        "kind": "rate_limited",
                        "message": msg,
                    }
                })),
            )
                .into_response();
            if let Some(headers) = headers {
                for (k, v) in headers {
                    if let Some(k) = k {
                        response.headers_mut().insert(k, v);
                    }
                }
            }
            response
        }
    }
}

/// An opinionated, minimal CORS layer: the standard methods and `content-type` /
/// `authorization` headers, for either any origin (`*`) or an explicit allow-list.
/// Credentials are not allowed (bearer-token APIs don't need cookies), so `*` is safe.
fn cors_layer(origins: &[String]) -> CorsLayer {
    let layer = CorsLayer::new()
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::PATCH,
            Method::DELETE,
        ])
        .allow_headers([header::CONTENT_TYPE, header::AUTHORIZATION]);
    if origins.iter().any(|o| o == "*") {
        layer.allow_origin(Any)
    } else {
        let list: Vec<HeaderValue> = origins.iter().filter_map(|o| o.parse().ok()).collect();
        layer.allow_origin(list)
    }
}

/// Outermost: assign a request id, open the request span, seed a default identity,
/// echo the id back as `x-request-id`, and set baseline security response headers.
async fn observe(mut req: Request, next: Next, hsts: bool) -> Response {
    let request_id = Uuid::new_v4();
    req.extensions_mut().insert(RequestId(request_id));
    req.extensions_mut().insert(Identity::Anonymous);
    if req.extensions().get::<SocketAddr>().is_none()
        && req
            .extensions()
            .get::<axum::extract::ConnectInfo<SocketAddr>>()
            .is_none()
    {
        req.extensions_mut()
            .insert(SocketAddr::from(([127, 0, 0, 1], 0)));
    }

    let method = req.method().clone();
    let path = req.uri().path().to_owned();
    let span = tracing::info_span!("request", %request_id, %method, path = %path);

    let mut response = async move { next.run(req).await }.instrument(span).await;

    let headers = response.headers_mut();
    if let Ok(value) = HeaderValue::from_str(&request_id.to_string()) {
        headers.insert("x-request-id", value);
    }
    // Safe-by-default security headers for an API. These are always correct; HSTS is
    // opt-in because it only applies behind TLS (see `Config::hsts`). CSP is
    // app-specific and intentionally left to the application.
    headers.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    headers.insert("x-frame-options", HeaderValue::from_static("DENY"));
    headers.insert("referrer-policy", HeaderValue::from_static("no-referrer"));
    if hsts {
        headers.insert(
            "strict-transport-security",
            HeaderValue::from_static("max-age=63072000; includeSubDomains"),
        );
    }
    response
}

/// Innermost: give the request a cancellation token that is a child of the process
/// shutdown token. It fires on graceful shutdown; on client disconnect, the handler
/// future (and any streaming body) is dropped by the runtime, which stops the work
/// regardless. Per-request token-on-disconnect wiring is a `0.3` concern.
async fn cancel(mut req: Request, next: Next, shutdown: CancellationToken) -> Response {
    req.extensions_mut().insert(shutdown.child_token());
    next.run(req).await
}

/// Turn a caught handler panic into the standard opaque `500`. The panic payload is
/// captured in the log (via `Error::Internal`, which logs its full cause) and never
/// leaks to the client. Reusing `Error` keeps the error-body shape identical to every
/// other `500`.
fn on_panic(payload: Box<dyn std::any::Any + Send + 'static>) -> Response {
    let detail = panic_message(&payload);
    Error::internal(format!("handler panicked: {detail}")).into_response()
}

fn panic_message(payload: &(dyn std::any::Any + Send)) -> &str {
    if let Some(s) = payload.downcast_ref::<&'static str>() {
        s
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s
    } else {
        "unknown panic"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request as HttpRequest;
    use tower::ServiceExt;

    async fn boom() -> &'static str {
        panic!("boom")
    }

    fn app(cors_origins: Option<Vec<String>>) -> axum::Router {
        let router = axum::Router::new()
            .route("/x", axum::routing::get(|| async { "ok" }))
            .route("/panic", axum::routing::get(boom));
        let config = StackConfig {
            cors_origins,
            ..StackConfig::default()
        };
        apply(
            router,
            axum::Router::new(),
            config,
            CancellationToken::new(),
        )
        .with_state(())
    }

    #[tokio::test]
    async fn handler_panic_becomes_500_and_is_decorated() {
        let request = HttpRequest::builder()
            .uri("/panic")
            .body(Body::empty())
            .unwrap();
        let response = app(None).oneshot(request).await.unwrap();
        // Caught, not crashed: a `500` came back instead of the task unwinding.
        assert_eq!(response.status(), 500);
        // It flowed back out through `observe`, so it still carries the request id and
        // the baseline security headers — a panic response is a normal response.
        assert!(response.headers().get("x-request-id").is_some());
        assert_eq!(
            response.headers().get("x-content-type-options").unwrap(),
            "nosniff"
        );
    }

    #[tokio::test]
    async fn cors_preflight_allows_listed_origin() {
        let request = HttpRequest::builder()
            .method("OPTIONS")
            .uri("/x")
            .header("origin", "https://app.example")
            .header("access-control-request-method", "GET")
            .body(Body::empty())
            .unwrap();
        let response = app(Some(vec!["https://app.example".into()]))
            .oneshot(request)
            .await
            .unwrap();
        assert_eq!(
            response
                .headers()
                .get("access-control-allow-origin")
                .unwrap(),
            "https://app.example"
        );
    }

    #[tokio::test]
    async fn no_cors_layer_when_unconfigured() {
        let request = HttpRequest::builder()
            .uri("/x")
            .header("origin", "https://app.example")
            .body(Body::empty())
            .unwrap();
        let response = app(None).oneshot(request).await.unwrap();
        assert_eq!(response.status(), 200);
        assert!(
            response
                .headers()
                .get("access-control-allow-origin")
                .is_none()
        );
    }
}
