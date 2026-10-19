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
        .routes(api::routes(auth))
        .task(TokenReaper::new(
            std::time::Duration::from_secs(3600),
            time::Duration::days(7),
        ))
        .run()
        .await
}
