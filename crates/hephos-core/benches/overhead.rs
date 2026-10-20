//! Benchmark measuring overhead of Hephos standard Tower stack vs raw Axum.
//! Target: Negligible, single-digit-% overhead for request lifecycle, extractors, and response handling.

use axum::body::Body;
use axum::http::Request;
use axum::routing::{get, post};
use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use hephos_core::test::TestClient;
use hephos_core::{Ctx, Json, Result, Router};
use serde::{Deserialize, Serialize};
use tower::ServiceExt;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct Item {
    id: u64,
    name: String,
    active: bool,
}

impl Item {
    fn sample() -> Self {
        Self {
            id: 1337,
            name: "benchmark-item".to_string(),
            active: true,
        }
    }
}

// --- Raw Axum Handlers ---

async fn raw_get_item() -> Json<Item> {
    Json(Item::sample())
}

async fn raw_post_item(Json(item): Json<Item>) -> Json<Item> {
    Json(item)
}

// --- Hephos Handlers (Ctx + Result<Json<T>>) ---

async fn hephos_get_item(_ctx: Ctx<()>) -> Result<Json<Item>> {
    Ok(Json(Item::sample()))
}

async fn hephos_post_item(_ctx: Ctx<()>, Json(item): Json<Item>) -> Result<Json<Item>> {
    Ok(Json(item))
}

fn bench_get_overhead(c: &mut Criterion) {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime required for benchmarks");

    // Raw Axum router
    let raw_app = axum::Router::new().route("/items", get(raw_get_item));

    // Hephos Router with standard Tower stack
    let hephos_router = Router::new().get("/items", hephos_get_item);
    let hephos_client = TestClient::new(hephos_router, ());

    let mut group = c.benchmark_group("request_lifecycle_get");

    group.bench_function(BenchmarkId::new("framework", "raw_axum"), |b| {
        b.iter(|| {
            rt.block_on(async {
                let req = Request::builder()
                    .uri("/items")
                    .body(Body::empty())
                    .expect("valid request");
                let response = raw_app.clone().oneshot(req).await.expect("service success");
                assert_eq!(response.status(), 200);
            })
        });
    });

    group.bench_function(
        BenchmarkId::new("framework", "hephos_standard_stack"),
        |b| {
            b.iter(|| {
                rt.block_on(async {
                    let response = hephos_client.get("/items").await;
                    assert_eq!(response.status(), 200);
                })
            });
        },
    );

    group.finish();
}

fn bench_post_overhead(c: &mut Criterion) {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime required for benchmarks");

    let sample = Item::sample();

    // Raw Axum router
    let raw_app = axum::Router::new().route("/items", post(raw_post_item));

    // Hephos Router with standard Tower stack
    let hephos_router = Router::new().post("/items", hephos_post_item);
    let hephos_client = TestClient::new(hephos_router, ());

    let mut group = c.benchmark_group("request_lifecycle_post");

    group.bench_function(BenchmarkId::new("framework", "raw_axum"), |b| {
        b.iter(|| {
            rt.block_on(async {
                let body = serde_json::to_vec(&sample).expect("serialize item");
                let req = Request::builder()
                    .method("POST")
                    .uri("/items")
                    .header("content-type", "application/json")
                    .body(Body::from(body))
                    .expect("valid request");
                let response = raw_app.clone().oneshot(req).await.expect("service success");
                assert_eq!(response.status(), 200);
            })
        });
    });

    group.bench_function(
        BenchmarkId::new("framework", "hephos_standard_stack"),
        |b| {
            b.iter(|| {
                rt.block_on(async {
                    let response = hephos_client.post_json("/items", &sample).await;
                    assert_eq!(response.status(), 200);
                })
            });
        },
    );

    group.finish();
}

criterion_group!(benches, bench_get_overhead, bench_post_overhead);
criterion_main!(benches);
