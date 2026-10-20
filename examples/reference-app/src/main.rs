mod agents;
mod api;
mod domain;
mod events;
mod mcp;
mod repos;
mod services;
mod state;
mod tasks;
mod tools;
mod workflows;

#[cfg(test)]
mod tests;

use hephos::prelude::*;

use crate::events::EventSubscriberTask;
use crate::state::AppState;
use crate::tasks::DataReaper;

#[tokio::main]
async fn main() -> Result<()> {
    let state = AppState::init().await?;
    let db = state.db.clone();
    let auth = state.auth.clone();

    App::new(state)
        .readiness(move || {
            let db = db.clone();
            async move { db.ping().await }
        })
        .routes(api::routes(auth))
        .task(DataReaper::default())
        .task(EventSubscriberTask)
        .run()
        .await
}
