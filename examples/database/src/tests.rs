//! HTTP-surface tests.
//!
//! `routes_build` runs wherever the crate compiles. The data-flow tests need a
//! live Postgres *and* wired migrations (`Db::migrate` is a BOUNDARY today), so
//! they are `#[ignore]` until that lands — run them with
//! `DATABASE_URL=… cargo test -p example-database -- --ignored`.

use hephos::test::TestClient;
use serde_json::{Value, json};

use crate::api;
use crate::state::AppState;

#[tokio::test]
async fn routes_build() {
    let _ = api::routes();
}

async fn client() -> TestClient {
    let state = AppState::init().await.unwrap();
    TestClient::new(api::routes(), state)
}

#[tokio::test]
#[ignore = "requires Postgres and wired migrations"]
async fn create_then_get_roundtrips() {
    let client = client().await;
    let created = client
        .post_json("/users", &json!({ "email": "ada@example.com" }))
        .await;
    assert_eq!(created.status(), 200);
    let id = created.json::<Value>()["id"].as_str().unwrap().to_string();

    let fetched = client.get(&format!("/users/{id}")).await;
    assert_eq!(fetched.status(), 200);
}

#[tokio::test]
#[ignore = "requires Postgres and wired migrations"]
async fn create_empty_email_is_400() {
    let res = client()
        .await
        .post_json("/users", &json!({ "email": "" }))
        .await;
    assert_eq!(res.status(), 400);
    assert_eq!(res.json::<Value>()["error"]["kind"], "invalid");
}
