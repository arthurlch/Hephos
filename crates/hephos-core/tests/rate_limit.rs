//! Rate limiting integration tests (milestone 0.2.0 Slice 3).
//!
//! Validates:
//! 1. Requests under burst limit succeed.
//! 2. Exceeding burst limit returns 429 with standard `rate_limited` error body,
//!    `retry-after` header, and request correlation.
//! 3. Independent quotas per client IP (via `SmartIpKeyExtractor` / `x-forwarded-for`).
//! 4. Probes (`/health`, `/ready`) are NEVER rate-limited even when client quota is exhausted.

use axum::body::Body;
use axum::http::Request;
use hephos_core::test::TestClient;
use hephos_core::{App, Ctx, Json, ProbeResponse, Result, Router};
use serde_json::Value;

#[derive(Clone)]
struct S;

async fn ping(_ctx: Ctx<S>) -> Result<Json<&'static str>> {
    Ok(Json("pong"))
}

fn routes() -> Router<S> {
    Router::new().get("/ping", ping)
}

#[tokio::test]
async fn requests_under_burst_limit_succeed() {
    let client = TestClient::with_rate_limit(routes(), S, 1, 3);

    for _ in 0..3 {
        let res = client.get("/ping").await;
        assert_eq!(res.status(), 200);
        assert_eq!(res.json::<String>(), "pong");
    }
}

#[tokio::test]
async fn exceeding_burst_returns_429_with_rate_limited_error_contract() {
    let client = TestClient::with_rate_limit(routes(), S, 1, 2);

    // Request 1 and 2: within burst capacity
    assert_eq!(client.get("/ping").await.status(), 200);
    assert_eq!(client.get("/ping").await.status(), 200);

    // Request 3: burst exhausted -> 429
    let res = client.get("/ping").await;
    assert_eq!(res.status(), 429);
    assert_eq!(res.error_kind().as_deref(), Some("rate_limited"));

    // Standard error body structure
    let body = res.json::<Value>();
    let message = body["error"]["message"].as_str().unwrap();
    assert!(
        message.starts_with("rate limit exceeded; try again in "),
        "message was: {message}"
    );

    // Standard headers: retry-after and x-request-id
    assert!(
        res.header("retry-after").is_some(),
        "retry-after header must be present on 429"
    );
    assert!(
        res.header("x-request-id").is_some(),
        "x-request-id header must be present on 429"
    );
    assert_eq!(
        res.header("x-content-type-options").as_deref(),
        Some("nosniff"),
        "security headers preserved on rate limit response"
    );
}

#[tokio::test]
async fn distinct_ips_are_rate_limited_independently() {
    let client = TestClient::with_rate_limit(routes(), S, 1, 1);

    let req_ip1 = || {
        Request::builder()
            .uri("/ping")
            .header("x-forwarded-for", "198.51.100.1")
            .body(Body::empty())
            .unwrap()
    };
    let req_ip2 = || {
        Request::builder()
            .uri("/ping")
            .header("x-forwarded-for", "198.51.100.2")
            .body(Body::empty())
            .unwrap()
    };

    // IP 1: first request passes
    let res1 = client.send(req_ip1()).await;
    assert_eq!(res1.status(), 200);

    // IP 1: second request rate-limited
    let res2 = client.send(req_ip1()).await;
    assert_eq!(res2.status(), 429);

    // IP 2: first request passes independently of IP 1
    let res3 = client.send(req_ip2()).await;
    assert_eq!(res3.status(), 200);
}

#[tokio::test]
async fn probes_are_never_rate_limited_even_when_quota_exhausted() {
    let app = App::new(S).routes(routes()).rate_limit(1, 1);
    let client = TestClient::from_app(app);

    // Exhaust client quota on application endpoint
    assert_eq!(client.get("/ping").await.status(), 200);
    assert_eq!(client.get("/ping").await.status(), 429);

    // Probes must remain accessible (200 OK) even while application routes are 429ing
    for _ in 0..5 {
        let health = client.get("/health").await;
        assert_eq!(health.status(), 200);
        assert_eq!(health.json::<ProbeResponse>(), ProbeResponse::ok());

        let ready = client.get("/ready").await;
        assert_eq!(ready.status(), 200);
        assert_eq!(ready.json::<ProbeResponse>(), ProbeResponse::ready());
    }
}
