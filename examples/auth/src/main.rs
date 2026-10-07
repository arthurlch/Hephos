mod api;
mod domain;
mod repos;
mod services;
mod state;
#[cfg(test)]
mod tests;
mod token;

use rivet::prelude::*;

use crate::state::AppState;

#[tokio::main]
async fn main() -> Result<()> {
    let state = AppState::init().await?;
    App::new(state).routes(api::routes()).run().await
}
