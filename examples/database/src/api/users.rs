use rivet::prelude::*;
use uuid::Uuid;

use crate::domain::user::{CreateUser, User};
use crate::state::{AppState, Ctx};

pub fn routes() -> Router<AppState> {
    Router::new()
        .get("/users/{id}", get)
        .post("/users", create)
}

pub async fn get(ctx: Ctx, Path(id): Path<Uuid>) -> Result<Json<User>> {
    let user = ctx.state().users.get(&ctx, id).await?;
    Ok(Json(user))
}

pub async fn create(ctx: Ctx, Json(input): Json<CreateUser>) -> Result<Json<User>> {
    let user = ctx.state().users.create(&ctx, input).await?;
    Ok(Json(user))
}
