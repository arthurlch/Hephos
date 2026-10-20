pub mod agent;
pub mod auth;
pub mod items;
pub mod workflows;

use hephos::prelude::*;

use crate::services::auth::AuthService;
use crate::state::AppState;

pub fn routes(auth: AuthService) -> Router<AppState> {
    let protected = items::routes()
        .merge(workflows::routes())
        .merge(agent::routes())
        .authenticated(move |token| {
            let auth = auth.clone();
            async move { auth.verify(&token).await }
        });

    Router::new()
        .merge(auth::routes())
        .merge(protected)
        .merge(crate::mcp::router())
}
