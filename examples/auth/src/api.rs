pub mod auth;
pub mod users;

use hephos::prelude::*;

use crate::state::AppState;

pub fn routes() -> Router<AppState> {
    // Verification keyring, read once at startup (AppState::init already validated the
    // current secret is set). `JWT_SECRET` is the current signing key; set
    // `JWT_SECRET_PREVIOUS` during a rotation so tokens signed with the old key keep
    // verifying until it is retired.
    let mut keyring = vec![std::env::var("JWT_SECRET").unwrap_or_default().into_bytes()];
    if let Ok(previous) = std::env::var("JWT_SECRET_PREVIOUS") {
        if !previous.is_empty() {
            keyring.push(previous.into_bytes());
        }
    }
    let protected =
        users::routes().authenticated(move |token| crate::token::verify(&keyring, token));

    Router::new().merge(auth::routes()).merge(protected)
}
