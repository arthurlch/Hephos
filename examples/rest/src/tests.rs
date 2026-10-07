//! HTTP-surface tests. These drive the real router through `rivet::test`, so they
//! exercise route composition, extraction, and the error model — the wiring that
//! unit tests on services never touch.

use rivet::test::TestClient;
use serde_json::{Value, json};

use crate::api;
use crate::state::AppState;

async fn client() -> TestClient {
    let state = AppState::init().await.unwrap();
    TestClient::new(api::routes(), state)
}

/// The canary. Building the router must not panic — this alone catches route
/// composition mistakes (e.g. nesting at the root) that compile cleanly.
#[tokio::test]
async fn routes_build() {
    let _ = api::routes();
}

#[tokio::test]
async fn list_products_ok() {
    let res = client().await.get("/products").await;
    assert_eq!(res.status(), 200);
    assert!(!res.json::<Value>().as_array().unwrap().is_empty());
}

#[tokio::test]
async fn get_missing_product_is_404() {
    let res = client()
        .await
        .get("/products/00000000-0000-0000-0000-0000000000ff")
        .await;
    assert_eq!(res.status(), 404);
    assert_eq!(res.json::<Value>()["error"]["kind"], "not_found");
}

#[tokio::test]
async fn search_empty_category_is_400() {
    let res = client()
        .await
        .post_json("/products/search", &json!({ "category": "" }))
        .await;
    assert_eq!(res.status(), 400);
    assert_eq!(res.json::<Value>()["error"]["kind"], "invalid");
}

#[tokio::test]
async fn search_filters_results() {
    let body = json!({ "category": "merch", "max_price_cents": 2000 });
    let res = client().await.post_json("/products/search", &body).await;
    assert_eq!(res.status(), 200);
    assert_eq!(res.json::<Value>().as_array().unwrap().len(), 1);
}
