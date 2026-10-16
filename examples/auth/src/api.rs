pub mod auth;
pub mod keys;
pub mod users;

use hephos::prelude::*;

use crate::services::auth::AuthService;
use crate::state::AppState;

/// Build the router. Takes the `AuthService` so the authenticated group's verify
/// closure can resolve both user JWTs and machine API keys against it; the handlers
/// themselves reach state through `ctx` at request time. Keeping this the only
/// build-time dependency lets the `routes_build` canary run offline.
pub fn routes(auth: AuthService) -> Router<AppState> {
    let protected = users::routes()
        .merge(keys::routes())
        .authenticated(move |token| {
            let auth = auth.clone();
            async move { auth.authenticate(&token).await }
        });

    Router::new().merge(auth::routes()).merge(protected)
}
