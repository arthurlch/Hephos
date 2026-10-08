pub mod users;

use hephos::prelude::*;

use crate::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new().merge(users::routes())
}
