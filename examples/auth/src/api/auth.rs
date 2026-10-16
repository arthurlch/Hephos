use hephos::prelude::*;

use crate::domain::user::{Credentials, RefreshRequest, TokenPair, User};
use crate::state::{AppState, Ctx};

pub fn routes() -> Router<AppState> {
    Router::new()
        .post("/auth/register", register)
        .post("/auth/login", login)
        .post("/auth/refresh", refresh)
        .post("/auth/logout", logout)
}

pub async fn register(ctx: Ctx, Json(input): Json<Credentials>) -> Result<Json<User>> {
    let user = ctx.state().auth.register(&ctx, input).await?;
    Ok(Json(user))
}

pub async fn login(ctx: Ctx, Json(input): Json<Credentials>) -> Result<Json<TokenPair>> {
    let pair = ctx.state().auth.login(&ctx, input).await?;
    Ok(Json(pair))
}

pub async fn refresh(ctx: Ctx, Json(input): Json<RefreshRequest>) -> Result<Json<TokenPair>> {
    let pair = ctx.state().auth.refresh(&ctx, input).await?;
    Ok(Json(pair))
}

pub async fn logout(ctx: Ctx, Json(input): Json<RefreshRequest>) -> Result<StatusCode> {
    ctx.state().auth.logout(&ctx, input).await?;
    Ok(StatusCode::NO_CONTENT)
}
