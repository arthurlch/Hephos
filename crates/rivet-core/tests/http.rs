//! A small end-to-end app built on the public surface, driven through `TestClient`.
//! This exercises the wiring unit tests can't: route composition, the typed
//! extractors, the one error type's HTTP mapping, and the auth middleware.

use rivet_core::test::TestClient;
use rivet_core::{Ctx, Error, Identity, Json, Path, Principal, Result, Router};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

#[derive(Clone)]
struct S;

#[derive(Debug, Serialize, Deserialize)]
struct Payload {
    name: String,
}

async fn echo(_ctx: Ctx<S>, Json(p): Json<Payload>) -> Result<Json<Payload>> {
    Ok(Json(p))
}

async fn get_item(_ctx: Ctx<S>, Path(id): Path<u32>) -> Result<Json<u32>> {
    if id == 0 {
        return Err(Error::not_found("item"));
    }
    Ok(Json(id))
}

async fn admin(ctx: Ctx<S>) -> Result<Json<&'static str>> {
    ctx.require_role("admin")?;
    Ok(Json("secret"))
}

fn verify(token: &str) -> Result<Identity> {
    match token {
        "admin" => Ok(Identity::User(Principal {
            id: Uuid::nil(),
            roles: vec!["admin".into()],
        })),
        "user" => Ok(Identity::User(Principal {
            id: Uuid::nil(),
            roles: vec![],
        })),
        _ => Err(Error::Unauthorized),
    }
}

fn app() -> TestClient {
    let protected = Router::<S>::new()
        .get("/admin", admin)
        .authenticated(verify);
    let router = Router::<S>::new()
        .post("/echo", echo)
        .get("/items/{id}", get_item)
        .merge(protected);
    TestClient::new(router, S)
}

#[tokio::test]
async fn json_body_roundtrips() {
    let res = app().post_json("/echo", &json!({ "name": "ada" })).await;
    assert_eq!(res.status(), 200);
    assert_eq!(res.json::<Payload>().name, "ada");
}

#[tokio::test]
async fn wrong_json_type_is_400_invalid() {
    let res = app().post_json("/echo", &json!({ "name": 123 })).await;
    assert_eq!(res.status(), 400);
    assert_eq!(res.json::<Value>()["error"]["kind"], "invalid");
}

#[tokio::test]
async fn domain_not_found_maps_to_404() {
    let res = app().get("/items/0").await;
    assert_eq!(res.status(), 404);
    assert_eq!(res.json::<Value>()["error"]["kind"], "not_found");
}

#[tokio::test]
async fn path_parse_error_is_400() {
    let res = app().get("/items/not-a-number").await;
    assert_eq!(res.status(), 400);
}

#[tokio::test]
async fn unknown_route_is_404() {
    let res = app().get("/does-not-exist").await;
    assert_eq!(res.status(), 404);
}

#[tokio::test]
async fn protected_route_rejects_missing_token() {
    let res = app().get("/admin").await;
    assert_eq!(res.status(), 401);
}

#[tokio::test]
async fn protected_route_forbids_without_role() {
    let res = app().get_with_token("/admin", "user").await;
    assert_eq!(res.status(), 403);
}

#[tokio::test]
async fn protected_route_allows_admin() {
    let res = app().get_with_token("/admin", "admin").await;
    assert_eq!(res.status(), 200);
}
