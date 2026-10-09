//! Tests. `routes_build` runs wherever the crate compiles. The data-flow tests need
//! a live Postgres, so they are `#[ignore]` offline — run them with
//! `DATABASE_URL=… cargo test -p example-database -- --include-ignored`.

use hephos::db::test::TestDb;
use hephos::test::TestClient;
use serde_json::{Value, json};

use crate::api;
use crate::domain::user::CreateUser;
use crate::repos::users::UserRepo;
use crate::state::AppState;

#[tokio::test]
async fn routes_build() {
    let _ = api::routes();
}

/// Proves `Db::migrate` builds the schema from scratch and a repo round-trips —
/// on a disposable database that is dropped afterward, so it leaves no residue.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn migrate_creates_schema_and_repo_roundtrips() {
    let test_db = TestDb::create(&sqlx::migrate!("./migrations"))
        .await
        .unwrap();

    let created = UserRepo::create(
        test_db.db().pool(),
        &CreateUser {
            email: "ada@example.com".into(),
        },
    )
    .await
    .unwrap();
    let found = UserRepo::find(test_db.db().pool(), created.id)
        .await
        .unwrap();
    assert_eq!(found.unwrap().email, "ada@example.com");

    test_db.cleanup().await.unwrap();
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
