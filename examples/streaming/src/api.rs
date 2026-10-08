pub mod events;

use hephos::prelude::*;

use crate::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new().merge(events::routes())
}
