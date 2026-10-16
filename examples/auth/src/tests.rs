//! Auth-flow tests.
//!
//! The flow tests need a live Postgres, wired migrations, and `JWT_SECRET`, so
//! they are `#[ignore]` until migrations land — run with
//! `DATABASE_URL=… JWT_SECRET=… cargo test -p example-auth -- --ignored`.

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
    let (access, _refresh) = tokens(&login.json::<Value>());

    let me = client.get_with_token("/me", &access).await;
    assert_eq!(me.status(), 200);

    // A non-admin may not read an arbitrary user.
    let other = client
        .get_with_token("/users/00000000-0000-0000-0000-000000000001", &access)
        .await;
    assert_eq!(other.status(), 403);
}

fn tokens(res: &Value) -> (String, String) {
    (
        res["access_token"].as_str().unwrap().to_string(),
        res["refresh_token"].as_str().unwrap().to_string(),
    )
}

#[tokio::test]
#[ignore = "requires Postgres, migrations, and JWT_SECRET"]
async fn refresh_rotates_and_logout_revokes() {
    let client = client().await;
    let creds = json!({ "email": "grace@example.com", "password": "correct horse battery" });
    assert_eq!(
        client.post_json("/auth/register", &creds).await.status(),
        200
    );

    let (_, refresh) = tokens(
        &client
            .post_json("/auth/login", &creds)
            .await
            .json::<Value>(),
    );

    // Refresh yields a new, different pair whose access token works.
    let refreshed = client
        .post_json("/auth/refresh", &json!({ "refresh_token": refresh }))
        .await;
    assert_eq!(refreshed.status(), 200);
    let (new_access, new_refresh) = tokens(&refreshed.json::<Value>());
    assert_ne!(refresh, new_refresh);
    assert_eq!(
        client.get_with_token("/me", &new_access).await.status(),
        200
    );

    // Logout revokes the current refresh token; it can no longer be exchanged.
    assert_eq!(
        client
            .post_json("/auth/logout", &json!({ "refresh_token": new_refresh }))
            .await
            .status(),
        204
    );
    assert_eq!(
        client
            .post_json("/auth/refresh", &json!({ "refresh_token": new_refresh }))
            .await
            .status(),
        401
    );
}

#[tokio::test]
#[ignore = "requires Postgres, migrations, and JWT_SECRET"]
async fn replaying_a_rotated_refresh_token_is_rejected() {
    let client = client().await;
    let creds = json!({ "email": "linus@example.com", "password": "correct horse battery" });
    assert_eq!(
        client.post_json("/auth/register", &creds).await.status(),
        200
    );

    let (_, refresh) = tokens(
        &client
            .post_json("/auth/login", &creds)
            .await
            .json::<Value>(),
    );

    // First use rotates `refresh` away.
    let (_, rotated) = tokens(
        &client
            .post_json("/auth/refresh", &json!({ "refresh_token": refresh }))
            .await
            .json::<Value>(),
    );

    // Replaying the now-revoked original is rejected (reuse/theft detection)...
    assert_eq!(
        client
            .post_json("/auth/refresh", &json!({ "refresh_token": refresh }))
            .await
            .status(),
        401
    );
    // ...and that detection revokes the whole family, including the rotated token.
    assert_eq!(
        client
            .post_json("/auth/refresh", &json!({ "refresh_token": rotated }))
            .await
            .status(),
        401
    );
}

#[tokio::test]
#[ignore = "requires Postgres, migrations, and JWT_SECRET"]
async fn logout_with_unknown_token_is_opaque() {
    let client = client().await;
    // Revoking a token that never existed still succeeds — no existence oracle.
    assert_eq!(
        client
            .post_json("/auth/logout", &json!({ "refresh_token": "deadbeef" }))
            .await
            .status(),
        204
    );
}
