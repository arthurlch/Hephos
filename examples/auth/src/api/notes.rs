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

/// Enforce note ownership, recording an audit event on a cross-owner denial (the IDOR
/// attempt) before returning `403`.
fn require_note_owner(ctx: &Ctx, note: &Note) -> Result<()> {
    if let Err(e) = ctx.require_owner(note.owner_id) {
        ctx.audit(
            "access_note",
            "note",
            Some(note.id.to_string()),
            AuditOutcome::Denied {
                reason: "requester is not the note owner".into(),
            },
            serde_json::json!({}),
        );
        return Err(e);
    }
    Ok(())
}

pub async fn get(ctx: Ctx, Path(id): Path<Uuid>) -> Result<Json<Note>> {
    let note = ctx.state().notes.get(&ctx, id).await?;
    require_note_owner(&ctx, &note)?;
    Ok(Json(note))
}

pub async fn update(
    ctx: Ctx,
    Path(id): Path<Uuid>,
    Json(input): Json<UpdateNote>,
) -> Result<Json<Note>> {
    let note = ctx.state().notes.get(&ctx, id).await?;
    require_note_owner(&ctx, &note)?;
    let note = ctx.state().notes.update(&ctx, id, input).await?;
    Ok(Json(note))
}

pub async fn delete(ctx: Ctx, Path(id): Path<Uuid>) -> Result<StatusCode> {
    let note = ctx.state().notes.get(&ctx, id).await?;
    require_note_owner(&ctx, &note)?;
    ctx.state().notes.delete(&ctx, id).await?;
    Ok(StatusCode::NO_CONTENT)
}
