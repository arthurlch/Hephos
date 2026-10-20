use hephos::prelude::*;

use crate::domain::user::{AuthCredentials, AuthTokenResponse, User};
use crate::state::{AppState, Ctx};

pub fn routes() -> Router<AppState> {
    Router::new()
        .post("/auth/register", register)
        .post("/auth/login", login)
}

pub async fn register(ctx: Ctx, Json(input): Json<AuthCredentials>) -> Result<Json<User>> {
    let user = ctx.state().auth.register(&ctx, input).await?;
    Ok(Json(user))
}

pub async fn login(
    ctx: Ctx,
    Json(input): Json<AuthCredentials>,
) -> Result<Json<AuthTokenResponse>> {
    let res = ctx.state().auth.login(&ctx, input).await?;
    Ok(Json(res))
}
