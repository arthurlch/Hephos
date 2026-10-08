use hephos::prelude::*;
use uuid::Uuid;

use crate::domain::user::User;
use crate::state::{AppState, Ctx};

/// Every route here is behind `Router::authenticated`, so an unauthenticated
/// request never reaches these handlers.
pub fn routes() -> Router<AppState> {
    Router::new().get("/me", me).get("/users/{id}", get)
}

/// Any authenticated user can read their own profile.
pub async fn me(ctx: Ctx) -> Result<Json<User>> {
    let id = ctx.require_user()?.id;
    let user = ctx.state().users.get(&ctx, id).await?;
    Ok(Json(user))
}

/// Only an admin can look up an arbitrary user.
pub async fn get(ctx: Ctx, Path(id): Path<Uuid>) -> Result<Json<User>> {
    ctx.require_role("admin")?;
    let user = ctx.state().users.get(&ctx, id).await?;
    Ok(Json(user))
}
