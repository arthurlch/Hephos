use std::convert::Infallible;

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::error::{Error, Result};
use crate::identity::{Identity, Principal};

/// The single request context, and the first parameter of every handler, task,
/// and tool entry point.
///
/// `Ctx<S>` carries the application state `S`, the resolved [`Identity`], a
/// request id, and a [`CancellationToken`] that fires on client disconnect or
/// server shutdown. Every app aliases it once in `state.rs`:
///
/// ```ignore
/// pub type Ctx = rivet::Ctx<AppState>;
/// ```
///
/// After that, every handler signature reads `ctx: Ctx` — no generics in app
/// code. This uniformity is the point: an agent sees `Ctx` and knows exactly
/// what is available.
#[derive(Clone)]
pub struct Ctx<S> {
    state: S,
    identity: Identity,
    request_id: Uuid,
    cancel: CancellationToken,
}

impl<S: Clone + Send + Sync + 'static> Ctx<S> {
    /// The application state. Services and the database live here.
    pub fn state(&self) -> &S {
        &self.state
    }

    /// Who is calling. Prefer [`Ctx::require_user`] / [`Ctx::require_role`] over
    /// matching on this directly.
    pub fn identity(&self) -> &Identity {
        &self.identity
    }

    /// Return the authenticated principal, or [`Error::Unauthorized`].
    pub fn require_user(&self) -> Result<&Principal> {
        match &self.identity {
            Identity::User(p) => Ok(p),
            Identity::Anonymous => Err(Error::Unauthorized),
        }
    }

    /// Return the authenticated principal if it holds `role`, else the narrowest
    /// correct error ([`Error::Unauthorized`] when anonymous,
    /// [`Error::Forbidden`] when authenticated without the role).
    pub fn require_role(&self, role: &str) -> Result<&Principal> {
        let principal = self.require_user()?;
        if principal.has_role(role) {
            Ok(principal)
        } else {
            Err(Error::Forbidden)
        }
    }

    /// Correlation id for this request. Already attached to the tracing span.
    pub fn request_id(&self) -> Uuid {
        self.request_id
    }

    /// Fires on client disconnect or graceful shutdown. Select against this in
    /// any loop or long-running operation.
    pub fn cancel_token(&self) -> &CancellationToken {
        &self.cancel
    }

    /// Construct a context outside the HTTP path — for tasks, tools, and tests.
    pub fn detached(state: S) -> Self {
        Ctx {
            state,
            identity: Identity::Anonymous,
            request_id: Uuid::new_v4(),
            cancel: CancellationToken::new(),
        }
    }

    /// Return a copy of this context carrying a specific identity. Used by the
    /// agent/tool layer to run work on behalf of an authenticated caller.
    pub fn with_identity(&self, identity: Identity) -> Self {
        Ctx {
            state: self.state.clone(),
            identity,
            request_id: self.request_id,
            cancel: self.cancel.clone(),
        }
    }

    /// Return a copy of this context bound to a specific cancellation token.
    /// Used by [`crate::App`] to tie background tasks to the process shutdown
    /// token.
    pub fn with_cancel(mut self, cancel: CancellationToken) -> Self {
        self.cancel = cancel;
        self
    }
}

// The context is produced for every request. `S` is supplied by axum's router
// state. Identity, request id, and cancellation token are read from request
// extensions populated by the standard middleware stack installed by `App`.
impl<S> FromRequestParts<S> for Ctx<S>
where
    S: Clone + Send + Sync + 'static,
{
    type Rejection = Infallible;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let identity = parts
            .extensions
            .get::<Identity>()
            .cloned()
            .unwrap_or_default();
        let request_id = parts
            .extensions
            .get::<RequestId>()
            .map(|r| r.0)
            .unwrap_or_else(Uuid::new_v4);
        let cancel = parts
            .extensions
            .get::<CancellationToken>()
            .cloned()
            .unwrap_or_default();

        Ok(Ctx {
            state: state.clone(),
            identity,
            request_id,
            cancel,
        })
    }
}

/// Request correlation id, read from request extensions here.
///
/// BOUNDARY: inserted by the standard middleware stack (see `app::standard_stack`).
/// Until that stack is wired, no code constructs it and `Ctx` falls back to a
/// freshly generated id — hence `allow(dead_code)` on the constructor path.
#[derive(Clone, Copy)]
#[allow(dead_code)]
pub(crate) struct RequestId(pub Uuid);

#[cfg(test)]
mod tests {
    use super::*;

    fn user(roles: &[&str]) -> Identity {
        Identity::User(Principal {
            id: Uuid::nil(),
            roles: roles.iter().map(|r| r.to_string()).collect(),
        })
    }

    #[test]
    fn anonymous_context_is_unauthorized() {
        let ctx = Ctx::detached(());
        assert!(matches!(ctx.require_user(), Err(Error::Unauthorized)));
        assert!(matches!(
            ctx.require_role("admin"),
            Err(Error::Unauthorized)
        ));
    }

    #[test]
    fn require_role_distinguishes_401_from_403() {
        let ctx = Ctx::detached(()).with_identity(user(&["user"]));
        assert!(ctx.require_user().is_ok());
        assert!(matches!(ctx.require_role("admin"), Err(Error::Forbidden)));
        assert!(ctx.require_role("user").is_ok());
    }

    #[test]
    fn detached_token_is_not_cancelled() {
        let ctx = Ctx::detached(());
        assert!(!ctx.cancel_token().is_cancelled());
    }
}
