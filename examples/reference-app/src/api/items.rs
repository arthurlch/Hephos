use hephos::prelude::*;
use uuid::Uuid;

use crate::domain::item::{CreateItem, Item, UpdateItem};
use crate::state::{AppState, Ctx};

pub fn routes() -> Router<AppState> {
    Router::new()
        .post("/items", create)
        .get("/items", list)
        .get("/items/{id}", get)
        .patch("/items/{id}", update)
        .delete("/items/{id}", delete)
}

pub async fn create(ctx: Ctx, Json(input): Json<CreateItem>) -> Result<Json<Item>> {
    let item = ctx.state().items.create(&ctx, input).await?;
    Ok(Json(item))
}

pub async fn list(ctx: Ctx) -> Result<Json<Vec<Item>>> {
    let items = ctx.state().items.list(&ctx).await?;
    Ok(Json(items))
}

pub async fn get(ctx: Ctx, Path(id): Path<Uuid>) -> Result<Json<Item>> {
    let item = ctx.state().items.get(&ctx, id).await?;
    Ok(Json(item))
}

pub async fn update(
    ctx: Ctx,
    Path(id): Path<Uuid>,
    Json(input): Json<UpdateItem>,
) -> Result<Json<Item>> {
    let item = ctx.state().items.update(&ctx, id, input).await?;
    Ok(Json(item))
}

pub async fn delete(ctx: Ctx, Path(id): Path<Uuid>) -> Result<StatusCode> {
    ctx.state().items.delete(&ctx, id).await?;
    Ok(StatusCode::NO_CONTENT)
}
