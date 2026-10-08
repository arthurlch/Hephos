pub mod products;

use hephos::prelude::*;

use crate::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new().merge(products::routes())
}
