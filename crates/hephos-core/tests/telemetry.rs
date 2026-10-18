//! Telemetry, OpenTelemetry, and RED metrics integration tests (milestone 0.2.0).
//!
//! Validates:
//! 1. RED metrics record route templates (`/items/{id}`), preventing cardinality explosions.
//! 2. Unmatched routes (404) are recorded with bounded route `"__unmatched__"`.
//! 3. Framework probes (`/health`, `/ready`) are recorded with their respective route templates.
//! 4. HTTP 5xx responses record errors.
//! 5. OTLP exporter integration and shutdown flushing.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use hephos_core::telemetry::{RedMetrics, TelemetryConfig, init_telemetry};
use hephos_core::test::TestClient;
use hephos_core::{App, Ctx, Error, Json, LogFormat, Path, Result, Router};
use tokio::net::TcpListener;

#[derive(Clone)]
struct State;

async fn get_item(_ctx: Ctx<State>, Path(id): Path<u32>) -> Result<Json<u32>> {
    Ok(Json(id))
}

async fn boom(_ctx: Ctx<State>) -> Result<Json<&'static str>> {
    Err(Error::internal("database connection lost"))
}

async fn panicky(_ctx: Ctx<State>) -> Result<Json<&'static str>> {
    panic!("unhandled crash in business logic");
}

fn test_router() -> Router<State> {
    Router::new()
        .get("/items/{id}", get_item)
        .get("/boom", boom)
        .get("/panic", panicky)
}

#[tokio::test]
async fn red_metrics_records_route_template_never_raw_path() {
    let recorded = Arc::new(Mutex::new(Vec::new()));
    let rec = recorded.clone();

    let metrics = RedMetrics::new().with_hook(move |method, route, status, duration| {
        rec.lock().unwrap().push((
            method.to_string(),
            route.to_string(),
            status,
            duration >= 0.0,
        ));
    });

    let client = TestClient::with_metrics(test_router(), State, metrics);

    // Hit dynamic route with different parameter values
    let res1 = client.get("/items/1").await;
    assert_eq!(res1.status(), 200);

    let res2 = client.get("/items/42").await;
    assert_eq!(res2.status(), 200);

    let res3 = client.get("/items/999").await;
    assert_eq!(res3.status(), 200);

    let entries = recorded.lock().unwrap().clone();
    assert_eq!(entries.len(), 3);

    for (method, route, status, duration_valid) in entries {
        assert_eq!(method, "GET");
        // Must be route template, NOT /items/1 or /items/42 or /items/999
        assert_eq!(route, "/items/{id}");
        assert_eq!(status, 200);
        assert!(duration_valid);
    }
}

#[tokio::test]
async fn red_metrics_records_unmatched_route_as_bounded_fallback() {
    let recorded = Arc::new(Mutex::new(Vec::new()));
    let rec = recorded.clone();

    let metrics = RedMetrics::new().with_hook(move |method, route, status, _| {
        rec.lock()
            .unwrap()
            .push((method.to_string(), route.to_string(), status));
    });

    let client = TestClient::with_metrics(test_router(), State, metrics);

    let res1 = client.get("/random-route-1").await;
    assert_eq!(res1.status(), 404);

    let res2 = client.get("/another/unmatched/nested/path").await;
    assert_eq!(res2.status(), 404);

    let entries = recorded.lock().unwrap().clone();
    assert_eq!(entries.len(), 2);

    assert_eq!(
        entries[0],
        ("GET".to_string(), "__unmatched__".to_string(), 404)
    );
    assert_eq!(
        entries[1],
        ("GET".to_string(), "__unmatched__".to_string(), 404)
    );
}

#[tokio::test]
async fn red_metrics_records_probes_with_route_templates() {
    let recorded = Arc::new(Mutex::new(Vec::new()));
    let rec = recorded.clone();

    let metrics = RedMetrics::new().with_hook(move |method, route, status, _| {
        rec.lock()
            .unwrap()
            .push((method.to_string(), route.to_string(), status));
    });

    let client = TestClient::with_metrics(test_router(), State, metrics);

    let res_health = client.get("/health").await;
    assert_eq!(res_health.status(), 200);

    let res_ready = client.get("/ready").await;
    assert_eq!(res_ready.status(), 200);

    let entries = recorded.lock().unwrap().clone();
    assert_eq!(entries.len(), 2);

    assert_eq!(entries[0], ("GET".to_string(), "/health".to_string(), 200));
    assert_eq!(entries[1], ("GET".to_string(), "/ready".to_string(), 200));
}

#[tokio::test]
async fn red_metrics_records_server_errors_and_panics() {
    let recorded = Arc::new(Mutex::new(Vec::new()));
    let rec = recorded.clone();

    let metrics = RedMetrics::new().with_hook(move |method, route, status, _| {
        rec.lock()
            .unwrap()
            .push((method.to_string(), route.to_string(), status));
    });

    let client = TestClient::with_metrics(test_router(), State, metrics);

    // 1. Error::internal returned from handler
    let res_boom = client.get("/boom").await;
    assert_eq!(res_boom.status(), 500);

    // 2. Panic caught by catch_panic layer
    let res_panic = client.get("/panic").await;
    assert_eq!(res_panic.status(), 500);

    let entries = recorded.lock().unwrap().clone();
    assert_eq!(entries.len(), 2);

    assert_eq!(entries[0], ("GET".to_string(), "/boom".to_string(), 500));
    assert_eq!(entries[1], ("GET".to_string(), "/panic".to_string(), 500));
}

#[tokio::test]
async fn app_builder_accepts_otlp_and_traces_knobs() {
    let app = App::new(State)
        .routes(test_router())
        .otlp_endpoint("http://localhost:4318")
        .traces(true)
        .metrics(RedMetrics::new());

    let client = TestClient::from_app(app);
    let res = client.get("/health").await;
    assert_eq!(res.status(), 200);
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires network socket permissions for mock OTLP receiver"]
async fn otlp_mock_receiver_receives_export_on_shutdown() {
    unsafe {
        std::env::set_var("NO_PROXY", "*");
        std::env::set_var("no_proxy", "*");
        std::env::remove_var("HTTP_PROXY");
        std::env::remove_var("http_proxy");
    }

    let received_requests = Arc::new(AtomicUsize::new(0));

    // Start in-process mock HTTP collector with raw TCP listener
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let counter = received_requests.clone();
    let mock_server = tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            counter.fetch_add(1, Ordering::SeqCst);
            let mut buf = [0u8; 1024];
            let n = stream.read(&mut buf).await.unwrap_or(0);
            eprintln!(
                "RAW TCP RECEIVED ({} bytes): {}",
                n,
                String::from_utf8_lossy(&buf[..n])
            );
            let response = b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n";
            let _ = stream.write_all(response).await;
        }
    });

    // Ensure mock server is listening
    tokio::time::sleep(Duration::from_millis(50)).await;

    let endpoint = format!("http://{addr}");
    let config = TelemetryConfig {
        otlp_endpoint: Some(endpoint),
        traces_enabled: true,
    };

    let handles = init_telemetry(LogFormat::Json, &config).expect("init telemetry");

    // Record some metrics
    let metrics = RedMetrics::new();
    metrics.record("GET", "/test", 200, 0.05);

    // Emit a tracing log
    tracing::info!("test telemetry log message");

    // Flush and shutdown providers
    handles.shutdown();

    // Small delay to allow mock HTTP server to process requests
    tokio::time::sleep(Duration::from_millis(50)).await;
    mock_server.abort();

    // Verify mock server received at least one export request
    assert!(
        received_requests.load(Ordering::SeqCst) > 0,
        "Mock OTLP collector should have received export requests on shutdown"
    );
}
