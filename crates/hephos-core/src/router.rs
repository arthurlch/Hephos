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
    /// `verify` turns a bearer token into an [`Identity`] (a stateless JWT
    /// verification: signature + claims → id + roles). A missing token, or a
    /// token `verify` rejects, produces a `401` before any handler runs. Handlers
    /// then read the resolved identity via `ctx` and may narrow further with
    /// `ctx.require_role(..)`.
    ///
    /// This is the one canonical way application code applies authentication —
    /// it wraps the group, so individual handlers never parse auth headers.
    pub fn authenticated<F>(mut self, verify: F) -> Self
    where
        F: Fn(&str) -> Result<Identity> + Clone + Send + Sync + 'static,
    {
        let middleware = axum::middleware::from_fn(
            move |mut request: axum::extract::Request, next: axum::middleware::Next| {
                let verify = verify.clone();
                async move {
                    let token = request
                        .headers()
                        .get(AUTHORIZATION)
                        .and_then(|value| value.to_str().ok())
                        .and_then(|value| value.strip_prefix("Bearer "));

                    match token {
                        Some(token) => match verify(token) {
                            Ok(identity) => {
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

#[cfg(test)]
mod tests {
    use super::*;

    async fn ok() -> &'static str {
        "ok"
    }

    #[test]
    fn builds_routes_without_panic() {
        let _ = Router::<()>::new()
            .get("/a", ok)
            .post("/b", ok)
            .merge(Router::<()>::new().get("/c", ok));
    }

    #[test]
    #[should_panic(expected = "merge")]
    fn nesting_at_root_panics_with_guidance() {
        let _ = Router::<()>::new().nest("/", Router::<()>::new());
    }
}
