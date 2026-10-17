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
use crate::tasks::audit::AuditLog;

#[tokio::main]
async fn main() -> Result<()> {
    let state = AppState::init().await?;
    let db = state.db.clone();
    App::new(state)
        .routes(api::routes())
        .readiness(move || {
            let db = db.clone();
            async move { db.ping().await }
        })
        .task(AuditLog)
        .run()
        .await
}
