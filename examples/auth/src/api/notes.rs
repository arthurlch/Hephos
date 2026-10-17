use hephos::prelude::*;
use uuid::Uuid;

use crate::domain::note::{CreateNote, Note, UpdateNote};
use crate::state::{AppState, Ctx};

/// Per-user notes — the resource-ownership demo. Every route is behind
/// `Router::authenticated`; each read/mutation loads the note and then calls
/// `ctx.require_owner(note.owner_id)` so one user can never touch another's note.
pub fn routes() -> Router<AppState> {
    Router::new()
        .post("/notes", create)
        .get("/notes/{id}", get)
        .put("/notes/{id}", update)
        .delete("/notes/{id}", delete)
}

pub async fn create(ctx: Ctx, Json(input): Json<CreateNote>) -> Result<Json<Note>> {
    let owner_id = ctx.require_user()?.id;
    let note = ctx.state().notes.create(&ctx, owner_id, input).await?;
    Ok(Json(note))
}

pub async fn get(ctx: Ctx, Path(id): Path<Uuid>) -> Result<Json<Note>> {
    let note = ctx.state().notes.get(&ctx, id).await?;
    ctx.require_owner(note.owner_id)?;
    Ok(Json(note))
}

pub async fn update(
    ctx: Ctx,
    Path(id): Path<Uuid>,
    Json(input): Json<UpdateNote>,
) -> Result<Json<Note>> {
    let note = ctx.state().notes.get(&ctx, id).await?;
    ctx.require_owner(note.owner_id)?;
    let note = ctx.state().notes.update(&ctx, id, input).await?;
    Ok(Json(note))
}

pub async fn delete(ctx: Ctx, Path(id): Path<Uuid>) -> Result<StatusCode> {
    let note = ctx.state().notes.get(&ctx, id).await?;
    ctx.require_owner(note.owner_id)?;
    ctx.state().notes.delete(&ctx, id).await?;
    Ok(StatusCode::NO_CONTENT)
}
