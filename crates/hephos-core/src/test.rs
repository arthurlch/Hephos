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
//! use hephos::test::TestClient;
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

use std::time::Duration;

use axum::body::{Body, Bytes, to_bytes};
use axum::http::{Request, StatusCode};
use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio_util::sync::CancellationToken;
use tower::ServiceExt;

use crate::app::App;
use crate::router::Router;
use crate::stack::{self, StackConfig};

/// A client bound to an in-memory instance of the application router — wrapped in
/// the same standard middleware stack `App` installs, so tests exercise the real
/// request lifecycle (request id, timeout, body limit, identity seed, cancellation).
pub struct TestClient {
    service: axum::Router,
}

impl TestClient {
    /// Build a client from the app's router and state, with production-default limits.
    /// Automatically mounts the framework probes (`/health`, `/ready`).
    pub fn new<S>(router: Router<S>, state: S) -> Self
    where
        S: Clone + Send + Sync + 'static,
    {
        Self::from_app(App::new(state).routes(router))
    }

    /// Build a client from an [`App`] instance, preserving registered readiness checks
    /// and auto-mounted probes.
    pub fn from_app<S>(app: App<S>) -> Self
    where
        S: Clone + Send + Sync + 'static,
    {
        Self::build_from_app(app, StackConfig::default())
    }

    /// Build a client with explicit timeout and body limit — for tests that assert
    /// the stack's `408`/`413` behavior without waiting on production defaults.
    pub fn with_stack<S>(router: Router<S>, state: S, timeout: Duration, body_limit: usize) -> Self
    where
        S: Clone + Send + Sync + 'static,
    {
        Self::from_app_with_stack(App::new(state).routes(router), timeout, body_limit)
    }

    /// Build a client from an [`App`] with custom timeout and body limit.
    pub fn from_app_with_stack<S>(app: App<S>, timeout: Duration, body_limit: usize) -> Self
    where
        S: Clone + Send + Sync + 'static,
    {
        Self::build_from_app(
            app,
            StackConfig {
                timeout,
                body_limit,
                ..StackConfig::default()
            },
        )
    }

    /// Build a client with explicit rate limiting (requests per second and burst capacity).
    pub fn with_rate_limit<S>(router: Router<S>, state: S, rps: u64, burst: u32) -> Self
    where
        S: Clone + Send + Sync + 'static,
    {
        Self::from_app(App::new(state).routes(router).rate_limit(rps, burst))
    }

    fn build_from_app<S>(app: App<S>, mut config: StackConfig) -> Self
    where
        S: Clone + Send + Sync + 'static,
    {
        if let Some(rl) = app.rate_limit {
            config.rate_limit = Some(rl);
        }
        let state = app.state.clone();
        let shutdown = CancellationToken::new();
        let (app_router, probe_router) = app.into_routers();
        TestClient {
            service: stack::apply(app_router, probe_router, config, shutdown).with_state(state),
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

    /// `DELETE` with an `Authorization: Bearer` header, for testing protected
    /// delete routes.
    pub async fn delete_with_token(&self, path: &str, token: &str) -> TestResponse {
        self.send(
            Request::builder()
                .method("DELETE")
                .uri(path)
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
    }

    /// `POST` a JSON body with an `Authorization: Bearer` header.
    pub async fn post_json_with_token<T: Serialize>(
        &self,
        path: &str,
        token: &str,
        body: &T,
    ) -> TestResponse {
        self.json_request_with_token("POST", path, token, body)
            .await
    }

    /// `PUT` a JSON body with an `Authorization: Bearer` header.
    pub async fn put_json_with_token<T: Serialize>(
        &self,
        path: &str,
        token: &str,
        body: &T,
    ) -> TestResponse {
        self.json_request_with_token("PUT", path, token, body).await
    }

    async fn json_request_with_token<T: Serialize>(
        &self,
        method: &str,
        path: &str,
        token: &str,
        body: &T,
    ) -> TestResponse {
        let bytes = serde_json::to_vec(body).unwrap();
        let request = Request::builder()
            .method(method)
            .uri(path)
            .header("content-type", "application/json")
            .header("authorization", format!("Bearer {token}"))
            .body(Body::from(bytes))
            .unwrap();
        self.send(request).await
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
        let headers = response.headers().clone();
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        TestResponse {
            status,
            headers,
            body,
        }
    }
}

/// A captured response: status, headers, and body, with typed accessors.
pub struct TestResponse {
    status: StatusCode,
    headers: axum::http::HeaderMap,
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

    /// A response header value as a string, if present.
    pub fn header(&self, name: &str) -> Option<String> {
        self.headers
            .get(name)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned)
    }

    /// The `error.kind` from a Hephos error body — the canonical failure assertion.
    pub fn error_kind(&self) -> Option<String> {
        self.json::<serde_json::Value>()
            .get("error")
            .and_then(|error| error.get("kind"))
            .and_then(|kind| kind.as_str())
            .map(str::to_owned)
    }
}
