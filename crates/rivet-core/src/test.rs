//! The canonical test harness.
//!
//! `TestClient` drives a real [`crate::Router`] — the same routes, extractors, and
//! middleware an app serves — without binding a socket. This exists so that
//! testing an app through its HTTP surface is the one obvious thing to do. A unit
//! test on a service proves the service works; only a test that drives the router
//! proves the app is wired correctly (routes compose, extractors resolve,
//! middleware runs). The `nest("/")` class of bug is only caught here.
//!
//! ```ignore
//! use rivet::test::TestClient;
//!
//! #[tokio::test]
//! async fn list_ok() {
//!     let state = AppState::init().await.unwrap();
//!     let client = TestClient::new(api::routes(), state);
//!     let res = client.get("/products").await;
//!     assert_eq!(res.status(), 200);
//! }
//! ```
//!
//! The harness is always available (not behind a feature) because testing is a
//! first-class concern, not an add-on. Its `unwrap`s are deliberate: a malformed
//! request in a test is a test bug and should fail loudly.

use axum::body::{Body, Bytes, to_bytes};
use axum::http::{Request, StatusCode};
use serde::Serialize;
use serde::de::DeserializeOwned;
use tower::ServiceExt;

use crate::router::Router;

/// A client bound to an in-memory instance of the application router.
pub struct TestClient {
    service: axum::Router,
}

impl TestClient {
    /// Build a client from the app's router and state — exactly what `App` serves.
    pub fn new<S>(router: Router<S>, state: S) -> Self
    where
        S: Clone + Send + Sync + 'static,
    {
        TestClient {
            service: router.into_inner().with_state(state),
        }
    }

    pub async fn get(&self, path: &str) -> TestResponse {
        self.send(Request::builder().uri(path).body(Body::empty()).unwrap())
            .await
    }

    pub async fn delete(&self, path: &str) -> TestResponse {
        self.send(
            Request::builder()
                .method("DELETE")
                .uri(path)
                .body(Body::empty())
                .unwrap(),
        )
        .await
    }

    pub async fn post_json<T: Serialize>(&self, path: &str, body: &T) -> TestResponse {
        self.json_request("POST", path, body).await
    }

    pub async fn put_json<T: Serialize>(&self, path: &str, body: &T) -> TestResponse {
        self.json_request("PUT", path, body).await
    }

    /// Send with an `Authorization: Bearer` header, for testing protected routes.
    pub async fn get_with_token(&self, path: &str, token: &str) -> TestResponse {
        self.send(
            Request::builder()
                .uri(path)
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
    }

    async fn json_request<T: Serialize>(&self, method: &str, path: &str, body: &T) -> TestResponse {
        let bytes = serde_json::to_vec(body).unwrap();
        let request = Request::builder()
            .method(method)
            .uri(path)
            .header("content-type", "application/json")
            .body(Body::from(bytes))
            .unwrap();
        self.send(request).await
    }

    /// The escape hatch: send a fully custom request.
    pub async fn send(&self, request: Request<Body>) -> TestResponse {
        let response = self.service.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        TestResponse { status, body }
    }
}

/// A captured response: status plus body, with typed accessors.
pub struct TestResponse {
    status: StatusCode,
    body: Bytes,
}

impl TestResponse {
    pub fn status(&self) -> u16 {
        self.status.as_u16()
    }

    pub fn json<T: DeserializeOwned>(&self) -> T {
        serde_json::from_slice(&self.body)
            .unwrap_or_else(|e| panic!("response body was not valid JSON for the target type: {e}"))
    }

    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}
