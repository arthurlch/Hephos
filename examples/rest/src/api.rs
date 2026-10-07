pub mod products;

use rivet::prelude::*;

use crate::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new().nest("/", products::routes())
}
