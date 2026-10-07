mod api;
mod domain;
mod repos;
mod services;
mod state;
mod tasks;

use rivet::prelude::*;

use crate::state::AppState;
use crate::tasks::audit::AuditLog;

#[tokio::main]
async fn main() -> Result<()> {
    let state = AppState::init().await?;
    App::new(state)
        .routes(api::routes())
        .task(AuditLog)
        .run()
        .await
}
