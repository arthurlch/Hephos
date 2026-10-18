//! Auth-flow tests.
//!
//! The flow tests need a live Postgres, wired migrations, and `JWT_SECRET`, so
//! they are `#[ignore]` until migrations land — run with
//! `DATABASE_URL=… JWT_SECRET=… cargo test -p example-auth -- --ignored`.

use hephos::test::TestClient;
use serde_json::{Value, json};

use crate::api;
use crate::services::auth::AuthService;
use crate::state::AppState;

#[tokio::test]
async fn routes_build() {
    // Offline: a lazily-connected `AuthService` lets the router assemble without a
    // database or `JWT_SECRET`, so this composition canary stays in the offline gate.
    let _ = api::routes(AuthService::offline());
}

async fn client() -> TestClient {
    let state = AppState::init().await.unwrap();
    let router = api::routes(state.auth.clone());
    TestClient::new(router, state)
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

/// Promote a registered user to admin directly in the database. The first admin is
/// always bootstrapped out-of-band; this stands in for that step.
async fn promote_to_admin(email: &str) {
    let url = std::env::var("DATABASE_URL").unwrap();
    let pool = sqlx::PgPool::connect(&url).await.unwrap();
    sqlx::query("update users set roles = array['admin'] where email = $1")
        .bind(email)
        .execute(&pool)
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "requires Postgres, migrations, and JWT_SECRET"]
async fn admin_mints_api_key_that_authenticates_then_is_revoked() {
    let client = client().await;

    // Bootstrap an admin, then log in so the access token carries the admin role.
    let admin = json!({ "email": "svc-admin@example.com", "password": "correct horse battery" });
    assert_eq!(
        client.post_json("/auth/register", &admin).await.status(),
        200
    );
    promote_to_admin("svc-admin@example.com").await;
    let (admin_access, _) = tokens(
        &client
            .post_json("/auth/login", &admin)
            .await
            .json::<Value>(),
    );

    // A target user the key will read.
    let target = json!({ "email": "target@example.com", "password": "correct horse battery" });
    let target_id = client
        .post_json("/auth/register", &target)
        .await
        .json::<Value>()["id"]
        .as_str()
        .unwrap()
        .to_string();

    // Admin mints an admin-scoped API key; the raw key is returned once.
    let created = client
        .post_json_with_token(
            "/api-keys",
            &admin_access,
            &json!({ "name": "ci-runner", "roles": ["admin"] }),
        )
        .await;
    assert_eq!(created.status(), 200);
    let created = created.json::<Value>();
    let api_key = created["api_key"].as_str().unwrap().to_string();
    let key_id = created["id"].as_str().unwrap().to_string();
    assert!(api_key.starts_with("hephos_sk_"));

    // The API key resolves to the same Identity (admin scope): it can read any user.
    assert_eq!(
        client
            .get_with_token(&format!("/users/{target_id}"), &api_key)
            .await
            .status(),
        200
    );

    // Revoke it; the key no longer authenticates.
    assert_eq!(
        client
            .delete_with_token(&format!("/api-keys/{key_id}"), &admin_access)
            .await
            .status(),
        204
    );
    assert_eq!(
        client
            .get_with_token(&format!("/users/{target_id}"), &api_key)
            .await
            .status(),
        401
    );
}

#[tokio::test]
#[ignore = "requires Postgres, migrations, and JWT_SECRET"]
async fn non_admin_cannot_mint_api_key() {
    let client = client().await;
    let creds = json!({ "email": "plain@example.com", "password": "correct horse battery" });
    assert_eq!(
        client.post_json("/auth/register", &creds).await.status(),
        200
    );
    let (access, _) = tokens(
        &client
            .post_json("/auth/login", &creds)
            .await
            .json::<Value>(),
    );

    assert_eq!(
        client
            .post_json_with_token(
                "/api-keys",
                &access,
                &json!({ "name": "nope", "roles": [] })
            )
            .await
            .status(),
        403
    );
}

/// Register a user and return their access token.
async fn register_and_login(client: &TestClient, email: &str) -> String {
    let creds = json!({ "email": email, "password": "correct horse battery" });
    assert_eq!(
        client.post_json("/auth/register", &creds).await.status(),
        200
    );
    tokens(
        &client
            .post_json("/auth/login", &creds)
            .await
            .json::<Value>(),
    )
    .0
}

#[tokio::test]
#[ignore = "requires Postgres, migrations, and JWT_SECRET"]
async fn owner_can_crud_note_but_others_get_403() {
    let client = client().await;
    let owner = register_and_login(&client, "owner@example.com").await;
    let intruder = register_and_login(&client, "intruder@example.com").await;

    // Owner creates a note.
    let created = client
        .post_json_with_token("/notes", &owner, &json!({ "body": "my secret" }))
        .await;
    assert_eq!(created.status(), 200);
    let note_id = created.json::<Value>()["id"].as_str().unwrap().to_string();

    // Owner can read, update, and (later) delete it.
    assert_eq!(
        client
            .get_with_token(&format!("/notes/{note_id}"), &owner)
            .await
            .status(),
        200
    );
    assert_eq!(
        client
            .put_json_with_token(
                &format!("/notes/{note_id}"),
                &owner,
                &json!({ "body": "edited" })
            )
            .await
            .status(),
        200
    );

    // A different authenticated user cannot touch it (require_owner → 403).
    assert_eq!(
        client
            .get_with_token(&format!("/notes/{note_id}"), &intruder)
            .await
            .status(),
        403
    );
    assert_eq!(
        client
            .put_json_with_token(
                &format!("/notes/{note_id}"),
                &intruder,
                &json!({ "body": "hijack" })
            )
            .await
            .status(),
        403
    );
    assert_eq!(
        client
            .delete_with_token(&format!("/notes/{note_id}"), &intruder)
            .await
            .status(),
        403
    );

    // An anonymous caller is 401, not 403.
    assert_eq!(client.get(&format!("/notes/{note_id}")).await.status(), 401);

    // The owner can delete it.
    assert_eq!(
        client
            .delete_with_token(&format!("/notes/{note_id}"), &owner)
            .await
            .status(),
        204
    );
}

#[tokio::test]
#[ignore = "requires Postgres, migrations, and JWT_SECRET"]
async fn reaper_purges_expired_and_revoked_tokens() {
    let state = AppState::init().await.unwrap();
    let client = TestClient::new(api::routes(state.auth.clone()), state.clone());
    let creds = json!({ "email": "reaper@example.com", "password": "correct horse battery" });

    assert_eq!(
        client.post_json("/auth/register", &creds).await.status(),
        200
    );

    let login = client.post_json("/auth/login", &creds).await;
    assert_eq!(login.status(), 200);
    let (access, refresh) = tokens(&login.json::<Value>());

    // Create an API key.
    let key_res = client
        .post_json_with_token(
            "/auth/api-keys",
            &access,
            &json!({ "name": "reaper-key", "roles": ["read"] }),
        )
        .await;
    assert_eq!(key_res.status(), 200);
    let key_id = key_res.json::<Value>()["id"].as_str().unwrap().to_string();

    // Revoke both: refresh token via logout, API key via DELETE.
    let logout_res = client
        .post_json("/auth/logout", &json!({ "refresh_token": refresh }))
        .await;
    assert_eq!(logout_res.status(), 200);

    let revoke_key_res = client
        .delete_with_token(&format!("/auth/api-keys/{key_id}"), &access)
        .await;
    assert_eq!(revoke_key_res.status(), 200);

    // Run reaper with zero grace period so recently-revoked tokens are purged.
    let summary = state
        .auth
        .reap_dead_tokens(time::Duration::ZERO)
        .await
        .unwrap();

    assert!(summary.purged_refresh_tokens >= 1);
    assert!(summary.purged_api_keys >= 1);
}
