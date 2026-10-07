pub mod events;

use rivet::prelude::*;

use crate::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new().nest("/", events::routes())
}
