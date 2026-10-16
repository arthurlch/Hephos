//! The standard middleware stack — the one fixed, correct Tower stack every Hephos
//! app runs, assembled here so application code never touches tower. Installing it
//! is what makes [`crate::Ctx`] honest: request id, tracing span, timeouts, body
//! limits, a default identity, and a shutdown-linked cancellation token.
//!
//! Order (outermost → innermost): observe → timeout → body-limit → cancel →
//! (per-route auth, if any) → handler. The order is fixed on purpose; middleware
//! ordering is a classic bug source, so it is decided once, here, and nowhere else.

use std::time::Duration;

use axum::extract::Request;
use axum::http::{HeaderValue, Method, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::Response;
use tokio_util::sync::CancellationToken;
use tower_http::cors::{Any, CorsLayer};
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::timeout::TimeoutLayer;
use tracing::Instrument;
use uuid::Uuid;

use crate::ctx::RequestId;
use crate::identity::Identity;

/// Knobs for the stack. Derived from [`crate::Config`] in production; defaulted in
/// the test harness.
#[derive(Debug, Clone)]
pub(crate) struct StackConfig {
    pub timeout: Duration,
    pub body_limit: usize,
    pub hsts: bool,
    pub cors_origins: Option<Vec<String>>,
}

impl Default for StackConfig {
    fn default() -> Self {
        StackConfig {
            timeout: Duration::from_secs(30),
            body_limit: 2 * 1024 * 1024,
            hsts: false,
            cors_origins: None,
        }
    }
}

/// Wrap `router` in the standard stack. `shutdown` is the process-wide token; each
/// request gets a child of it, so a graceful shutdown cancels in-flight work.
pub(crate) fn apply<S>(
    router: axum::Router<S>,
    config: StackConfig,
    shutdown: CancellationToken,
) -> axum::Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    let hsts = config.hsts;
    // `.layer()` applies outermost-last, so the call order below is innermost-first.
    // CORS is outermost so preflight (`OPTIONS`) is answered before auth/timeout.
    let mut router = router
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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request as HttpRequest;
    use tower::ServiceExt;

    fn app(cors_origins: Option<Vec<String>>) -> axum::Router {
        let router = axum::Router::new().route("/x", axum::routing::get(|| async { "ok" }));
        let config = StackConfig {
            cors_origins,
            ..StackConfig::default()
        };
        apply(router, config, CancellationToken::new()).with_state(())
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
