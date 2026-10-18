//! Concurrency limiting and load shedding integration tests (milestone 0.2.0 Slice 4).
//!
//! Validates:
//! 1. Requests under concurrency limit succeed with 200 OK.
//! 2. Requests arriving when concurrency limit is saturated are shed immediately with
//!    503 Service Unavailable and standard JSON error shape.
//! 3. Subsequent requests succeed once in-flight requests complete.
//! 4. Probes (`/health`, `/ready`) are NEVER load-shed even when client concurrency is saturated.

use std::sync::Arc;
use std::time::Duration;

use hephos_core::test::TestClient;
use hephos_core::{App, Ctx, Json, ProbeResponse, Result, Router};
use tokio::sync::Notify;

#[derive(Clone)]
struct S {
    notify: Arc<Notify>,
}

async fn slow(ctx: Ctx<S>) -> Result<Json<&'static str>> {
    ctx.state().notify.notified().await;
    Ok(Json("completed"))
}

async fn fast(_ctx: Ctx<S>) -> Result<Json<&'static str>> {
    Ok(Json("fast"))
}

fn routes() -> Router<S> {
    Router::new().get("/slow", slow).get("/fast", fast)
}

#[tokio::test]
async fn requests_under_concurrency_limit_succeed() {
    let state = S {
        notify: Arc::new(Notify::new()),
    };
    let client = TestClient::with_concurrency_limit(routes(), state, 2);

    let res = client.get("/fast").await;
    assert_eq!(res.status(), 200);
    assert_eq!(res.json::<String>(), "fast");
}

#[tokio::test]
async fn saturated_concurrency_immediately_sheds_excess_requests() {
    let notify = Arc::new(Notify::new());
    let state = S {
        notify: notify.clone(),
    };
    let client = TestClient::with_concurrency_limit(routes(), state, 1);

    // Request 1: acquires the 1 available concurrency permit and holds
    let client_clone = client.clone();
    let handle1 = tokio::spawn(async move { client_clone.get("/slow").await });

    // Yield to let task 1 enter handler and hold the permit
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Request 2: arrives while concurrency limit is saturated -> shed immediately (503)
    let res2 = client.get("/fast").await;
    assert_eq!(res2.status(), 503);
    assert_eq!(res2.error_kind().as_deref(), Some("unavailable"));
    assert_eq!(
        res2.error_message().as_deref(),
        Some("service overloaded, shedding load")
    );

    // Release request 1
    notify.notify_waiters();
    let res1 = handle1.await.unwrap();
    assert_eq!(res1.status(), 200);
    assert_eq!(res1.json::<String>(), "completed");

    // Request 3: arrives after permit is released -> succeeds (200)
    let res3 = client.get("/fast").await;
    assert_eq!(res3.status(), 200);
}

#[tokio::test]
async fn probes_are_never_load_shed_even_when_concurrency_is_saturated() {
    let notify = Arc::new(Notify::new());
    let state = S {
        notify: notify.clone(),
    };
    let app = App::new(state)
        .routes(routes())
        .concurrency_limit(1)
        .readiness(|| async { Ok(()) });
    let client = TestClient::from_app(app);

    // Request 1: saturates the 1 available concurrency permit
    let client_clone = client.clone();
    let handle1 = tokio::spawn(async move { client_clone.get("/slow").await });

    // Yield to let task 1 acquire the permit
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Application requests are shed (503)
    let res_app = client.get("/fast").await;
    assert_eq!(res_app.status(), 503);

    // /health probe is NEVER shed (200 OK)
    let res_health = client.get("/health").await;
    assert_eq!(res_health.status(), 200);
    let health_body: ProbeResponse = res_health.json();
    assert_eq!(health_body.status, "ok");

    // /ready probe is NEVER shed (200 OK)
    let res_ready = client.get("/ready").await;
    assert_eq!(res_ready.status(), 200);
    let ready_body: ProbeResponse = res_ready.json();
    assert_eq!(ready_body.status, "ready");

    // Release request 1
    notify.notify_waiters();
    let res1 = handle1.await.unwrap();
    assert_eq!(res1.status(), 200);
}
