//! Connection cap and socket draining integration tests (milestone 0.3.0 Slice 3).
//!
//! Validates:
//! 1. Requests under connection cap succeed with 200 OK.
//! 2. Requests arriving when connection cap is saturated are rejected immediately with
//!    503 Service Unavailable and standard JSON error shape.
//! 3. Subsequent requests succeed once active connections complete and drop their guards.
//! 4. Probes (`/health`, `/ready`) are NEVER connection-capped even when client capacity is saturated.
//! 5. Per-route connection caps isolate capacity per endpoint.
//! 6. Sockets with [`ConnectionGuard`] drain cleanly and unblock drain waiters.

use std::sync::Arc;
use std::time::Duration;

use hephos_core::conn::{ConnectionCap, ConnectionGuard};
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
async fn requests_under_connection_cap_succeed() {
    let state = S {
        notify: Arc::new(Notify::new()),
    };
    let client = TestClient::with_connection_cap(routes(), state, 2);

    let res = client.get("/fast").await;
    assert_eq!(res.status(), 200);
    assert_eq!(res.json::<String>(), "fast");
}

#[tokio::test]
async fn saturated_connection_cap_rejects_excess_requests_with_503() {
    let notify = Arc::new(Notify::new());
    let state = S {
        notify: notify.clone(),
    };
    let client = TestClient::with_connection_cap(routes(), state, 1);

    // Request 1: acquires the 1 available connection permit and holds it
    let client_clone = client.clone();
    let handle1 = tokio::spawn(async move { client_clone.get("/slow").await });

    // Yield to let task 1 enter handler and hold the permit
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Request 2: arrives while connection cap is saturated -> rejected immediately (503)
    let res2 = client.get("/fast").await;
    assert_eq!(res2.status(), 503);
    assert_eq!(res2.error_kind().as_deref(), Some("unavailable"));
    assert_eq!(
        res2.error_message().as_deref(),
        Some("connection cap reached")
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
async fn probes_are_never_connection_capped_even_when_saturated() {
    let notify = Arc::new(Notify::new());
    let state = S {
        notify: notify.clone(),
    };
    let app = App::new(state)
        .routes(routes())
        .max_connections(1)
        .readiness(|| async { Ok(()) });
    let client = TestClient::from_app(app);

    // Request 1: saturates the 1 available connection permit
    let client_clone = client.clone();
    let handle1 = tokio::spawn(async move { client_clone.get("/slow").await });

    // Yield to let task 1 acquire the permit
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Probes bypass connection caps completely
    let health_res = client.get("/health").await;
    assert_eq!(health_res.status(), 200);
    assert_eq!(health_res.json::<ProbeResponse>(), ProbeResponse::ok());

    let ready_res = client.get("/ready").await;
    assert_eq!(ready_res.status(), 200);
    assert_eq!(ready_res.json::<ProbeResponse>(), ProbeResponse::ready());

    // Release request 1
    notify.notify_waiters();
    let res1 = handle1.await.unwrap();
    assert_eq!(res1.status(), 200);
}

#[tokio::test]
async fn per_route_connection_cap_isolates_endpoints() {
    let notify = Arc::new(Notify::new());
    let state = S {
        notify: notify.clone(),
    };

    let capped_pool = ConnectionCap::new(1);
    let capped_route = Router::new()
        .get("/capped/slow", slow)
        .connection_cap(capped_pool);

    let router = Router::new().get("/uncapped", fast).merge(capped_route);
    let client = TestClient::new(router, state);

    // Request 1: saturates the capped route
    let client_clone = client.clone();
    let handle1 = tokio::spawn(async move { client_clone.get("/capped/slow").await });
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Request 2 to capped route -> 503
    let res_capped = client.get("/capped/slow").await;
    assert_eq!(res_capped.status(), 503);

    // Request 3 to uncapped route -> 200 OK (unaffected by capped route pool)
    let res_uncapped = client.get("/uncapped").await;
    assert_eq!(res_uncapped.status(), 200);

    // Release request 1
    notify.notify_waiters();
    let res1 = handle1.await.unwrap();
    assert_eq!(res1.status(), 200);
}

#[tokio::test]
async fn connection_guard_extractor_and_draining() {
    let notify = Arc::new(Notify::new());
    let notify_clone = notify.clone();

    let cap = ConnectionCap::new(1);
    let cap_clone = cap.clone();

    let router = Router::new()
        .get("/stream", move |guard: ConnectionGuard| {
            let notify = notify_clone.clone();
            async move {
                tokio::spawn(async move {
                    let _guard = guard;
                    notify.notified().await;
                });
                "connected"
            }
        })
        .connection_cap(cap);

    let client = TestClient::new(router, ());

    // Connect to /stream
    let res = client.get("/stream").await;
    assert_eq!(res.status(), 200);
    assert_eq!(res.text(), "connected");
    assert_eq!(cap_clone.active(), 1);

    // Second connection attempt is rejected while guard is held in background task:
    let res2 = client.get("/stream").await;
    assert_eq!(res2.status(), 503);

    // Spawn drain in background
    let drain_cap = cap_clone.clone();
    let drain_handle = tokio::spawn(async move { drain_cap.drain(Duration::from_secs(2)).await });

    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(cap_clone.active(), 1);

    // Release socket background task
    notify.notify_waiters();

    // Drain completes cleanly
    let drain_result = drain_handle.await.unwrap();
    assert!(drain_result.is_ok());
    assert_eq!(cap_clone.active(), 0);
}
