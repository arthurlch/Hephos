//! Probe endpoints (`/health`, `/ready`) integration tests.
//!
//! Driven through `TestClient` to verify probe auto-mounting, response contracts,
//! readiness flipping to 503 on dependency failure, and standard stack integration.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use hephos_core::test::TestClient;
use hephos_core::{App, Error, ProbeResponse, Router};
use serde_json::Value;

#[tokio::test]
async fn health_probe_returns_200_and_status_ok() {
    let client = TestClient::new(Router::<()>::new(), ());
    let res = client.get("/health").await;

    assert_eq!(res.status(), 200);
    assert_eq!(res.json::<ProbeResponse>(), ProbeResponse::ok());
}

#[tokio::test]
async fn ready_probe_defaults_to_200_and_status_ready() {
    let client = TestClient::new(Router::<()>::new(), ());
    let res = client.get("/ready").await;

    assert_eq!(res.status(), 200);
    assert_eq!(res.json::<ProbeResponse>(), ProbeResponse::ready());
}

#[tokio::test]
async fn ready_probe_reflects_custom_check_state() {
    let is_ready = Arc::new(AtomicBool::new(true));

    let check_flag = is_ready.clone();
    let app = App::new(()).readiness(move || {
        let ok = check_flag.load(Ordering::SeqCst);
        async move {
            if ok {
                Ok(())
            } else {
                Err(Error::unavailable("postgres connection refused"))
            }
        }
    });

    let client = TestClient::from_app(app);

    // Initial state: ready
    let res = client.get("/ready").await;
    assert_eq!(res.status(), 200);
    assert_eq!(res.json::<ProbeResponse>(), ProbeResponse::ready());

    // Flip to unavailable
    is_ready.store(false, Ordering::SeqCst);
    let res = client.get("/ready").await;
    assert_eq!(res.status(), 503);
    let body = res.json::<Value>();
    assert_eq!(body["error"]["kind"], "unavailable");
    assert_eq!(body["error"]["message"], "postgres connection refused");

    // Flip back to ready
    is_ready.store(true, Ordering::SeqCst);
    let res = client.get("/ready").await;
    assert_eq!(res.status(), 200);
    assert_eq!(res.json::<ProbeResponse>(), ProbeResponse::ready());
}

#[tokio::test]
async fn ready_probe_masks_internal_error_cause() {
    let app = App::new(())
        .readiness(|| async { Err(Error::internal("underlying secret db connection string")) });

    let client = TestClient::from_app(app);
    let res = client.get("/ready").await;

    assert_eq!(res.status(), 503);
    let body = res.json::<Value>();
    assert_eq!(body["error"]["kind"], "unavailable");
    assert_eq!(body["error"]["message"], "service unavailable");
}

#[tokio::test]
async fn probes_carry_standard_stack_headers() {
    let client = TestClient::new(Router::<()>::new(), ());
    let res = client.get("/health").await;

    assert_eq!(res.status(), 200);
    assert!(res.header("x-request-id").is_some());
    assert_eq!(
        res.header("x-content-type-options").as_deref(),
        Some("nosniff")
    );
}

#[tokio::test]
async fn health_probe_rejects_non_get_method() {
    let client = TestClient::new(Router::<()>::new(), ());
    let res = client.delete("/health").await;

    assert_eq!(res.status(), 405);
}
