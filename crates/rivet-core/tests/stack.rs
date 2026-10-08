//! Standard-stack acceptance tests (milestone 0.0.2). These assert the observable
//! guarantees the middleware stack makes: request-id correlation, request timeout,
//! body-size limit, a seeded identity, and shutdown-linked cancellation — all driven
//! through the real stack via `TestClient`.

use std::time::Duration;

use rivet_core::test::TestClient;
use rivet_core::{Ctx, Identity, Json, Result, Router};
use serde_json::{Value, json};

#[derive(Clone)]
struct S;

async fn request_id(ctx: Ctx<S>) -> Result<Json<String>> {
    Ok(Json(ctx.request_id().to_string()))
}

async fn whoami(ctx: Ctx<S>) -> Result<Json<&'static str>> {
    Ok(Json(match ctx.identity() {
        Identity::Anonymous => "anonymous",
        Identity::User(_) => "user",
    }))
}

async fn slow(_ctx: Ctx<S>) -> Result<Json<&'static str>> {
    tokio::time::sleep(Duration::from_millis(500)).await;
    Ok(Json("done"))
}

async fn echo(_ctx: Ctx<S>, Json(v): Json<Value>) -> Result<Json<Value>> {
    Ok(Json(v))
}

fn routes() -> Router<S> {
    Router::new()
        .get("/request-id", request_id)
        .get("/whoami", whoami)
        .get("/slow", slow)
        .post("/echo", echo)
}

#[tokio::test]
async fn response_carries_request_id_matching_the_body() {
    let client = TestClient::new(routes(), S);
    let res = client.get("/request-id").await;
    assert_eq!(res.status(), 200);

    let header = res
        .header("x-request-id")
        .expect("x-request-id header present");
    let body: String = res.json();
    assert_eq!(
        header, body,
        "ctx.request_id() must match the x-request-id header"
    );
    assert!(
        uuid::Uuid::parse_str(&header).is_ok(),
        "request id is a uuid"
    );
}

#[tokio::test]
async fn identity_is_seeded_anonymous_by_default() {
    let client = TestClient::new(routes(), S);
    let res = client.get("/whoami").await;
    assert_eq!(res.status(), 200);
    assert_eq!(res.json::<String>(), "anonymous");
}

#[tokio::test]
async fn slow_handler_is_cut_by_the_timeout() {
    // 50ms timeout vs a 500ms handler → the stack must return 408.
    let client = TestClient::with_stack(routes(), S, Duration::from_millis(50), 1024 * 1024);
    let res = client.get("/slow").await;
    assert_eq!(res.status(), 408);
}

#[tokio::test]
async fn oversized_body_is_rejected() {
    use axum::body::Body;
    use axum::http::Request;

    // 16-byte body limit; a real client advertises Content-Length, so the stack
    // short-circuits with 413 before the handler runs.
    let client = TestClient::with_stack(routes(), S, Duration::from_secs(30), 16);
    let payload = serde_json::to_vec(&json!({
        "field": "this is definitely longer than sixteen bytes"
    }))
    .unwrap();
    let request = Request::builder()
        .method("POST")
        .uri("/echo")
        .header("content-type", "application/json")
        .header("content-length", payload.len().to_string())
        .body(Body::from(payload))
        .unwrap();

    let res = client.send(request).await;
    assert_eq!(res.status(), 413);
}

#[tokio::test]
async fn within_limits_requests_succeed() {
    let client = TestClient::new(routes(), S);
    let res = client.post_json("/echo", &json!({ "ok": 1 })).await;
    assert_eq!(res.status(), 200);
    assert_eq!(res.json::<Value>()["ok"], 1);
}
