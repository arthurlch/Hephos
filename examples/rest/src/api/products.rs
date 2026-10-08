use hephos::prelude::*;
use uuid::Uuid;

use crate::domain::product::{ListQuery, Product, SearchRequest};
use crate::state::{AppState, Ctx};

pub fn routes() -> Router<AppState> {
    Router::new()
        .get("/products", list)
        .get("/products/{id}", get)
        .post("/products/search", search)
}

pub async fn list(ctx: Ctx, Query(query): Query<ListQuery>) -> Result<Json<Vec<Product>>> {
    let products = ctx.state().catalog.list(&ctx, query).await?;
    Ok(Json(products))
}

pub async fn get(ctx: Ctx, Path(id): Path<Uuid>) -> Result<Json<Product>> {
    let product = ctx.state().catalog.get(&ctx, id).await?;
    Ok(Json(product))
}

pub async fn search(ctx: Ctx, Json(request): Json<SearchRequest>) -> Result<Json<Vec<Product>>> {
    let products = ctx.state().catalog.search(&ctx, request).await?;
    Ok(Json(products))
}
