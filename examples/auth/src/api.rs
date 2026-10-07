pub mod auth;
pub mod users;

use rivet::prelude::*;

use crate::state::AppState;

pub fn routes() -> Router<AppState> {
    // Secret read once at startup (AppState::init already validated it is set).
    let secret = std::env::var("JWT_SECRET").unwrap_or_default().into_bytes();
    let protected =
        users::routes().authenticated(move |token| crate::token::verify(&secret, token));

    Router::new().merge(auth::routes()).merge(protected)
}
