//! Auth-flow tests.
//!
//! The flow tests need a live Postgres, wired migrations, and `JWT_SECRET`, so
//! they are `#[ignore]` until migrations land — run with
//! `DATABASE_URL=… JWT_SECRET=… cargo test -p example-auth -- --ignored`.

use rivet::test::TestClient;
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
#[ignore = "requires Postgres, migrations, and JWT_SECRET"]
async fn protected_route_rejects_anonymous() {
    let res = client().await.get("/me").await;
    assert_eq!(res.status(), 401);
}

#[tokio::test]
#[ignore = "requires Postgres, migrations, and JWT_SECRET"]
async fn register_login_then_access_profile() {
    let client = client().await;
    let creds = json!({ "email": "ada@example.com", "password": "correct horse battery" });

    assert_eq!(
        client.post_json("/auth/register", &creds).await.status(),
        200
    );

    let login = client.post_json("/auth/login", &creds).await;
    assert_eq!(login.status(), 200);
    let token = login.json::<Value>()["token"].as_str().unwrap().to_string();

    let me = client.get_with_token("/me", &token).await;
    assert_eq!(me.status(), 200);

    // A non-admin may not read an arbitrary user.
    let other = client
        .get_with_token("/users/00000000-0000-0000-0000-000000000001", &token)
        .await;
    assert_eq!(other.status(), 403);
}
