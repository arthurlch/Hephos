use rivet::prelude::*;

use crate::domain::user::{Credentials, TokenResponse, User};
use crate::state::{AppState, Ctx};

pub fn routes() -> Router<AppState> {
    Router::new()
        .post("/auth/register", register)
        .post("/auth/login", login)
}

pub async fn register(ctx: Ctx, Json(input): Json<Credentials>) -> Result<Json<User>> {
    let user = ctx.state().auth.register(&ctx, input).await?;
    Ok(Json(user))
}

pub async fn login(ctx: Ctx, Json(input): Json<Credentials>) -> Result<Json<TokenResponse>> {
    let token = ctx.state().auth.login(&ctx, input).await?;
    Ok(Json(token))
}
