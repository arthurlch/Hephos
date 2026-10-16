use hephos::prelude::*;
use uuid::Uuid;

use crate::domain::user::{ApiKeyCreated, CreateApiKey};
use crate::state::{AppState, Ctx};

/// Admin-only management of machine-to-machine API keys. These routes sit behind
/// `Router::authenticated`, so the caller is already a resolved identity; `require_role`
/// narrows to admins.
pub fn routes() -> Router<AppState> {
    Router::new()
        .post("/api-keys", create)
        .delete("/api-keys/{id}", revoke)
}

pub async fn create(ctx: Ctx, Json(input): Json<CreateApiKey>) -> Result<Json<ApiKeyCreated>> {
    ctx.require_role("admin")?;
    let created = ctx.state().auth.mint_api_key(&ctx, input).await?;
    Ok(Json(created))
}

pub async fn revoke(ctx: Ctx, Path(id): Path<Uuid>) -> Result<StatusCode> {
    ctx.require_role("admin")?;
    ctx.state().auth.revoke_api_key(&ctx, id).await?;
    Ok(StatusCode::NO_CONTENT)
}
