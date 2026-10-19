use axum::handler::Handler;
use axum::http::header::AUTHORIZATION;
use axum::response::IntoResponse;
use axum::routing;

use crate::error::{Error, Result};
use crate::identity::Identity;

/// The one router.
///
/// Hephos wraps `axum::Router` to expose a single, verb-first method per HTTP
/// method. `router.get(path, handler)` instead of `router.route(path, get(h))`
/// removes the method-filter indirection and gives an agent one shape to copy.
///
/// Path parameters use axum 0.8 brace syntax: `/users/{id}`, `/files/{*path}`.
///
/// `layer` exists but is reserved for the framework's own middleware. Application
/// code should not add layers here; cross-cutting behavior belongs in the
/// standard stack installed by [`crate::App`].
pub struct Router<S> {
    inner: axum::Router<S>,
}

/// Probe paths reserved by the framework. Applications cannot define custom routes
/// at these paths; `/health` is auto-mounted by [`crate::App`], and `/ready` is
/// configured via [`crate::App::readiness`].
pub const RESERVED_PROBE_PATHS: &[&str] = &["/health", "/ready"];

impl<S> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    pub fn new() -> Self {
        Router {
            inner: axum::Router::new(),
        }
    }

    pub fn get<H, T>(self, path: &str, handler: H) -> Self
    where
        H: Handler<T, S>,
        T: 'static,
    {
        self.method(path, routing::get(handler))
    }

    pub fn post<H, T>(self, path: &str, handler: H) -> Self
    where
        H: Handler<T, S>,
        T: 'static,
    {
        self.method(path, routing::post(handler))
    }

    pub fn put<H, T>(self, path: &str, handler: H) -> Self
    where
        H: Handler<T, S>,
        T: 'static,
    {
        self.method(path, routing::put(handler))
    }

    pub fn patch<H, T>(self, path: &str, handler: H) -> Self
    where
        H: Handler<T, S>,
        T: 'static,
    {
        self.method(path, routing::patch(handler))
    }

    pub fn delete<H, T>(self, path: &str, handler: H) -> Self
    where
        H: Handler<T, S>,
        T: 'static,
    {
        self.method(path, routing::delete(handler))
    }

    /// Combine another router at the same level. This is how `api::routes()`
    /// assembles the per-resource routers, each of which already declares full
    /// paths (`/users`, `/products`).
    pub fn merge(mut self, router: Router<S>) -> Self {
        self.inner = self.inner.merge(router.inner);
        self
    }

    /// Mount a sub-router under a non-root prefix, e.g. nesting an orders router
    /// under `/users/{id}`. Do not nest at `"/"` — use [`Router::merge`] for
    /// same-level composition.
    ///
    /// Panics early, with a Hephos message, if `prefix` is `"/"` or empty. The
    /// underlying router also rejects this, but with a cryptic message; catching
    /// it here names the fix (`merge`) so the mistake is obvious the first time.
    pub fn nest(mut self, prefix: &str, router: Router<S>) -> Self {
        assert!(
            prefix != "/" && !prefix.is_empty(),
            "Router::nest(\"{prefix}\", ..) is invalid: nesting at the root is not \
             supported. Use Router::merge(..) to combine routers at the same level.",
        );
        self.inner = self.inner.nest(prefix, router.inner);
        self
    }

    /// Require authentication for every route in this router.
    ///
    /// `verify` turns a bearer token into an [`Identity`]. It is **async**, because
    /// real authentication needs I/O: a stateless JWT check returns immediately, but
    /// resolving an opaque API key or checking a revocation list is a database lookup.
    /// `verify` captures whatever it needs to do that (a signing keyring, a credential
    /// store) — typically a service cloned into the closure.
    ///
    /// A missing token, or a token `verify` rejects with [`Error::Unauthorized`],
    /// produces a `401` before any handler runs. An [`Error::Internal`] from `verify`
    /// (e.g. the credential store is down) surfaces as `500`, not a misleading `401`.
    /// Handlers then read the resolved identity via `ctx` and may narrow further with
    /// `ctx.require_role(..)`.
    ///
    /// This is the one canonical way application code applies authentication —
    /// it wraps the group, so individual handlers never parse auth headers.
    pub fn authenticated<F, Fut>(mut self, verify: F) -> Self
    where
        F: Fn(String) -> Fut + Clone + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<Identity>> + Send + 'static,
    {
        let middleware = axum::middleware::from_fn(
            move |mut request: axum::extract::Request, next: axum::middleware::Next| {
                let verify = verify.clone();
                async move {
                    let token = extract_credential(&request);

                    match token {
                        Some(token) => match verify(token).await {
                            Ok(identity) => {
                                // Overwrites the `Identity::Anonymous` the standard stack
                                // seeds. Correct because this `route_layer` runs *inside*
                                // the stack (which is applied outermost by `App`), so our
                                // resolved identity is the last writer `Ctx` reads.
                                request.extensions_mut().insert(identity);
                                next.run(request).await
                            }
                            Err(error) => error.into_response(),
                        },
                        None => Error::Unauthorized.into_response(),
                    }
                }
            },
        );
        // `route_layer`, not `layer`: the auth check runs only for requests that
        // match a route in this group. An unmatched path stays a `404` instead of
        // being turned into a `401` by a fallback the middleware would otherwise
        // wrap once routers are merged.
        self.inner = self.inner.route_layer(middleware);
        self
    }

    fn method(mut self, path: &str, method: routing::MethodRouter<S>) -> Self {
        assert!(
            !RESERVED_PROBE_PATHS.contains(&path),
            "route \"{path}\" is reserved by Hephos for framework probes. \
             Liveness (/health) is auto-mounted; configure readiness via App::readiness.",
        );
        self.inner = self.inner.route(path, method);
        self
    }

    pub(crate) fn into_inner(self) -> axum::Router<S> {
        self.inner
    }
}

impl<S> Default for Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    fn default() -> Self {
        Self::new()
    }
}

/// An upper bound on an accepted bearer credential. Real tokens (JWTs with roles,
/// opaque API keys) are well under 4 KiB; anything larger is rejected before `verify`
/// runs, so a caller can't force signature checks or credential-store lookups on
/// pathologically large input.
const MAX_TOKEN_LEN: usize = 4096;

/// Extract authentication credential from either the `Authorization: Bearer <token>`
/// header or query parameters (`?token=<token>` / `?access_token=<token>`).
///
/// Query parameter extraction provides auth-on-upgrade support for WebSocket handshakes
/// and Server-Sent Events where browser JavaScript cannot attach custom headers.
fn extract_credential(request: &axum::extract::Request) -> Option<String> {
    if let Some(token) = request
        .headers()
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(bearer_token)
    {
        return Some(token.to_string());
    }

    if let Some(query) = request.uri().query() {
        for pair in query.split('&') {
            if let Some((k, v)) = pair.split_once('=') {
                if (k == "token" || k == "access_token")
                    && !v.is_empty()
                    && v.len() <= MAX_TOKEN_LEN
                {
                    return percent_decode(v);
                }
            }
        }
    }

    None
}

/// Simple, allocation-friendly percent-decoding for query parameter credentials.
fn percent_decode(input: &str) -> Option<String> {
    if !input.contains('%') {
        return Some(input.to_string());
    }
    let mut bytes = Vec::with_capacity(input.len());
    let mut chars = input.bytes();
    while let Some(b) = chars.next() {
        if b == b'%' {
            let h1 = chars.next()?;
            let h2 = chars.next()?;
            let hex = [h1, h2];
            let hex_str = std::str::from_utf8(&hex).ok()?;
            let byte = u8::from_str_radix(hex_str, 16).ok()?;
            bytes.push(byte);
        } else {
            bytes.push(b);
        }
    }
    String::from_utf8(bytes).ok()
}

/// Extract the credential from an `Authorization` header value, accepting the
/// `Bearer` scheme case-insensitively (RFC 7235 schemes are case-insensitive).
/// Returns `None` for a missing/empty credential, a different scheme, or a credential
/// longer than [`MAX_TOKEN_LEN`].
fn bearer_token(header: &str) -> Option<&str> {
    let (scheme, credential) = header.split_once(' ')?;
    if !scheme.eq_ignore_ascii_case("bearer") {
        return None;
    }
    let token = credential.trim();
    (!token.is_empty() && token.len() <= MAX_TOKEN_LEN).then_some(token)
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn ok() -> &'static str {
        "ok"
    }

    #[test]
    fn parses_bearer_case_insensitively() {
        assert_eq!(bearer_token("Bearer abc"), Some("abc"));
        assert_eq!(bearer_token("bearer abc"), Some("abc"));
        assert_eq!(bearer_token("BEARER   abc  "), Some("abc"));
    }

    #[test]
    fn rejects_other_schemes_and_empty() {
        assert_eq!(bearer_token("Basic abc"), None);
        assert_eq!(bearer_token("Bearer "), None);
        assert_eq!(bearer_token("abc"), None);
    }

    #[test]
    fn rejects_oversized_tokens() {
        let ok = "a".repeat(MAX_TOKEN_LEN);
        assert_eq!(bearer_token(&format!("Bearer {ok}")), Some(ok.as_str()));
        let too_long = "a".repeat(MAX_TOKEN_LEN + 1);
        assert_eq!(bearer_token(&format!("Bearer {too_long}")), None);
    }

    #[test]
    fn extracts_credential_from_header_and_query() {
        use axum::http::Request;

        // 1. Authorization header
        let req_header = Request::builder()
            .header(AUTHORIZATION, "Bearer my-secret-token")
            .body(axum::body::Body::empty())
            .unwrap();
        assert_eq!(
            extract_credential(&req_header),
            Some("my-secret-token".to_string())
        );

        // 2. Query parameter: token
        let req_query_token = Request::builder()
            .uri("/ws?token=ws-token-123")
            .body(axum::body::Body::empty())
            .unwrap();
        assert_eq!(
            extract_credential(&req_query_token),
            Some("ws-token-123".to_string())
        );

        // 3. Query parameter: access_token with percent-encoded characters
        let req_query_access = Request::builder()
            .uri("/events?access_token=jwt%2Babc%20123")
            .body(axum::body::Body::empty())
            .unwrap();
        assert_eq!(
            extract_credential(&req_query_access),
            Some("jwt+abc 123".to_string())
        );

        // 4. Missing or unrelated query params
        let req_unrelated = Request::builder()
            .uri("/ws?foo=bar")
            .body(axum::body::Body::empty())
            .unwrap();
        assert_eq!(extract_credential(&req_unrelated), None);
    }

    #[test]
    fn percent_decode_cases() {
        assert_eq!(percent_decode("simple"), Some("simple".to_string()));
        assert_eq!(percent_decode("a%20b"), Some("a b".to_string()));
        assert_eq!(percent_decode("a%2Bb%3Dc"), Some("a+b=c".to_string()));
        assert_eq!(percent_decode("invalid%"), None);
        assert_eq!(percent_decode("invalid%2"), None);
        assert_eq!(percent_decode("invalid%ZZ"), None);
    }

    #[test]
    fn extract_credential_edge_cases() {
        use axum::http::Request;

        // Empty token in query
        let req_empty = Request::builder()
            .uri("/ws?token=")
            .body(axum::body::Body::empty())
            .unwrap();
        assert_eq!(extract_credential(&req_empty), None);

        // Multiple query parameters
        let req_multi = Request::builder()
            .uri("/ws?theme=dark&token=my-token&page=1")
            .body(axum::body::Body::empty())
            .unwrap();
        assert_eq!(extract_credential(&req_multi), Some("my-token".to_string()));

        // Oversized query token
        let oversized = "x".repeat(MAX_TOKEN_LEN + 1);
        let req_oversized = Request::builder()
            .uri(format!("/ws?token={oversized}"))
            .body(axum::body::Body::empty())
            .unwrap();
        assert_eq!(extract_credential(&req_oversized), None);

        // Corrupted percent encoding in query token
        let req_bad_pct = Request::builder()
            .uri("/ws?token=bad%XXtoken")
            .body(axum::body::Body::empty())
            .unwrap();
        assert_eq!(extract_credential(&req_bad_pct), None);
    }

    #[test]
    fn builds_routes_all_methods() {
        let _ = Router::<()>::new()
            .get("/a", ok)
            .post("/b", ok)
            .put("/c", ok)
            .patch("/d", ok)
            .delete("/e", ok)
            .merge(Router::<()>::new().get("/sub", ok));
    }

    #[test]
    #[should_panic(expected = "merge")]
    fn nesting_empty_prefix_panics_with_guidance() {
        let _ = Router::<()>::new().nest("", Router::<()>::new());
    }

    #[test]
    #[should_panic(expected = "merge")]
    fn nesting_at_root_panics_with_guidance() {
        let _ = Router::<()>::new().nest("/", Router::<()>::new());
    }

    #[test]
    #[should_panic(expected = "reserved by Hephos")]
    fn reserving_health_panics_with_guidance() {
        let _ = Router::<()>::new().get("/health", ok);
    }

    #[test]
    #[should_panic(expected = "reserved by Hephos")]
    fn reserving_ready_panics_with_guidance() {
        let _ = Router::<()>::new().get("/ready", ok);
    }
}
