//! Proves the meta crate's single import surface is sufficient to build and serve a
//! working app — `use hephos::prelude::*;` plus the shipped test harness, nothing else.

use hephos::prelude::*;

async fn echo(
    _ctx: Ctx<()>,
    Json(value): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>> {
    Ok(Json(value))
}

#[tokio::test]
async fn prelude_builds_a_working_app() {
    let router = Router::<()>::new().post("/echo", echo);
    let client = hephos::test::TestClient::new(router, ());

    let res = client
        .post_json("/echo", &serde_json::json!({ "ok": true }))
        .await;
    assert_eq!(res.status(), 200);
    assert_eq!(res.json::<serde_json::Value>()["ok"], true);
}
