mod api;
mod domain;
mod repos;
mod services;
mod state;
mod tasks;
#[cfg(test)]
mod tests;

use hephos::prelude::*;

use crate::state::AppState;
use crate::tasks::reaper::TokenReaper;

#[tokio::main]
async fn main() -> Result<()> {
    let state = AppState::init().await?;
    let auth = state.auth.clone();
    let db = state.db.clone();
    App::new(state)
        .readiness(move || {
            let db = db.clone();
            async move { db.ping().await }
        })
        // Per-IP rate limiting bounds credential-stuffing / brute-force against the auth
        // endpoints (keyed on the unspoofable peer IP by default). This example applies a
        // single global limit for simplicity; a production app would apply a tighter
        // sensitive limit (e.g. RateLimitConfig::sensitive) specifically to /auth/login
        // and /auth/refresh via a per-route layer.
        .rate_limit(50, 100)
        .routes(api::routes(auth))
        .task(TokenReaper::new(
            std::time::Duration::from_secs(3600),
            time::Duration::days(7),
        ))
        .run()
        .await
}
