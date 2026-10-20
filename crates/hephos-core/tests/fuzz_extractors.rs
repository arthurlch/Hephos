use axum::body::Body;
use axum::http::Request;
use hephos_core::test::TestClient;
use hephos_core::{Ctx, Error, Identity, Json, Principal, Query, Result, Router, StatusCode};
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
struct Payload {
    name: String,
    count: u32,
}

#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
struct SearchParams {
    query: String,
    limit: Option<u32>,
}

async fn echo_json(_ctx: Ctx<()>, Json(payload): Json<Payload>) -> Result<Json<Payload>> {
    Ok(Json(payload))
}

async fn search_query(
    _ctx: Ctx<()>,
    Query(params): Query<SearchParams>,
) -> Result<Json<SearchParams>> {
    Ok(Json(params))
}

async fn protected(_ctx: Ctx<()>) -> Result<StatusCode> {
    Ok(StatusCode::OK)
}

fn router() -> Router<()> {
    let auth_group = Router::new()
        .get("/protected", protected)
        .authenticated(|token| async move {
            if token == "valid-token" {
                Ok(Identity::User(Principal {
                    id: uuid::Uuid::nil(),
                    roles: vec!["user".into()],
                }))
            } else {
                Err(Error::Unauthorized)
            }
        });

    Router::new()
        .post("/json", echo_json)
        .get("/query", search_query)
        .merge(auth_group)
}

#[tokio::test]
async fn fuzz_json_extractor_rejects_malformed_inputs_without_panic() {
    let client = TestClient::new(router(), ());

    let nested_obj = "{\"a\":".repeat(50) + "1" + &"}".repeat(50);
    let malformed_inputs = vec![
        "",
        "{",
        "}",
        "[]",
        r#"{"name": 123}"#,
        r#"{"name": "test", "count": -1}"#,
        r#"{"name": "test", "count": "not a number"}"#,
        r#"{"name": "valid", "count": 10, invalid_syntax"#,
        "\0\0\0",
        "<!DOCTYPE html><html><body>Error</body></html>",
        r#"{"name": null, "count": null}"#,
        "\"raw string literal\"",
        "12345",
        "true",
        "false",
        "null",
        nested_obj.as_str(),
    ];

    for input in malformed_inputs {
        let req = Request::builder()
            .method("POST")
            .uri("/json")
            .header("content-type", "application/json")
            .body(Body::from(input.to_string()))
            .unwrap();
        let resp = client.send(req).await;
        assert_eq!(
            resp.status(),
            StatusCode::BAD_REQUEST,
            "failed on malformed input: {input}"
        );
        let error_body = resp.error_kind();
        assert_eq!(error_body, Some("invalid".to_string()));
    }
}

#[tokio::test]
async fn fuzz_query_extractor_rejects_malformed_queries_without_panic() {
    let client = TestClient::new(router(), ());

    let malformed_queries = [
        "/query",
        "/query?",
        "/query?invalid_key=123",
        "/query?limit=10",                      // query field missing
        "/query?query=test&limit=not_a_number", // bad number
        "/query?query=test&limit=-5",           // negative number
    ];

    for q in malformed_queries {
        let resp = client.get(q).await;
        assert_eq!(
            resp.status(),
            StatusCode::BAD_REQUEST,
            "failed on query: {q}"
        );
        assert_eq!(resp.error_kind(), Some("invalid".to_string()));
    }

    // Valid query param succeeds
    let resp = client.get("/query?query=hello&limit=10").await;
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn fuzz_auth_tokens_and_enforce_size_caps() {
    let client = TestClient::new(router(), ());

    // 1. Oversized bearer token (> 4096 bytes) is rejected with 401
    let huge_token = "a".repeat(4097);
    let req = Request::builder()
        .uri("/protected")
        .header("authorization", format!("Bearer {huge_token}"))
        .body(Body::empty())
        .unwrap();
    let resp = client.send(req).await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

    // 2. Exact max token length (4096 bytes) does not crash or panic
    let max_token = "a".repeat(4096);
    let req = Request::builder()
        .uri("/protected")
        .header("authorization", format!("Bearer {max_token}"))
        .body(Body::empty())
        .unwrap();
    let resp = client.send(req).await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

    // 3. Malformed schemes and headers that are valid HTTP header values
    let malformed_auth_headers = [
        "Bearer",
        "Bearer ",
        "bearer   ",
        "BEARER \t\t",
        "Basic dXNlcjpwYXNz",
        "Digest username=\"Mufasa\"",
        "Token xyz",
        "Bearer %FF%FE",
        "Bearer not-a-real-token",
    ];

    for header in malformed_auth_headers {
        let req = Request::builder()
            .uri("/protected")
            .header("authorization", header)
            .body(Body::empty())
            .unwrap();
        let resp = client.send(req).await;
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    // 4. Valid token succeeds
    let resp = client.get_with_token("/protected", "valid-token").await;
    assert_eq!(resp.status(), StatusCode::OK);

    // 5. Case-insensitive bearer scheme
    let req = Request::builder()
        .uri("/protected")
        .header("authorization", "bEaReR valid-token")
        .body(Body::empty())
        .unwrap();
    let resp = client.send(req).await;
    assert_eq!(resp.status(), StatusCode::OK);
}
