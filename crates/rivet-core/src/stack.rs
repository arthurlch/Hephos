//! The standard middleware stack — the one fixed, correct Tower stack every Rivet
//! app runs, assembled here so application code never touches tower. Installing it
//! is what makes [`crate::Ctx`] honest: request id, tracing span, timeouts, body
//! limits, a default identity, and a shutdown-linked cancellation token.
//!
//! Order (outermost → innermost): observe → timeout → body-limit → cancel →
//! (per-route auth, if any) → handler. The order is fixed on purpose; middleware
//! ordering is a classic bug source, so it is decided once, here, and nowhere else.

use std::time::Duration;

use axum::extract::Request;
use axum::http::{HeaderValue, StatusCode};
use axum::middleware::{self, Next};
use axum::response::Response;
use tokio_util::sync::CancellationToken;
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::timeout::TimeoutLayer;
use tracing::Instrument;
use uuid::Uuid;

use crate::ctx::RequestId;
use crate::identity::Identity;

/// Knobs for the stack. Derived from [`crate::Config`] in production; defaulted in
/// the test harness.
#[derive(Debug, Clone, Copy)]
pub(crate) struct StackConfig {
    pub timeout: Duration,
    pub body_limit: usize,
}

impl Default for StackConfig {
    fn default() -> Self {
        StackConfig {
            timeout: Duration::from_secs(30),
            body_limit: 2 * 1024 * 1024,
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
    // `.layer()` applies outermost-last, so the call order below is innermost-first.
    router
        .layer(middleware::from_fn(move |req, next| {
            cancel(req, next, shutdown.clone())
        }))
        .layer(RequestBodyLimitLayer::new(config.body_limit))
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            config.timeout,
        ))
        .layer(middleware::from_fn(observe))
}

/// Outermost: assign a request id, open the request span, seed a default identity,
/// and echo the id back as `x-request-id` for client/log correlation.
async fn observe(mut req: Request, next: Next) -> Response {
    let request_id = Uuid::new_v4();
    req.extensions_mut().insert(RequestId(request_id));
    req.extensions_mut().insert(Identity::Anonymous);

    let method = req.method().clone();
    let path = req.uri().path().to_owned();
    let span = tracing::info_span!("request", %request_id, %method, path = %path);

    let mut response = async move { next.run(req).await }.instrument(span).await;

    if let Ok(value) = HeaderValue::from_str(&request_id.to_string()) {
        response.headers_mut().insert("x-request-id", value);
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
