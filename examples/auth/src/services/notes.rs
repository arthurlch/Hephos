use hephos::prelude::*;
use uuid::Uuid;

use crate::domain::note::{CreateNote, Note, UpdateNote};
use crate::repos::notes::NoteRepo;
use crate::state::Ctx;

/// Notes CRUD. Ownership is *not* enforced here — it is the handler's job via
/// `ctx.require_owner(note.owner_id)` (CONVENTIONS §2.6a), so the service stays a plain
/// data operation and the authorization decision is visible at the route.
#[derive(Clone)]
pub struct NoteService {
    db: Db,
}

impl NoteService {
    pub fn new(db: Db) -> Self {
        NoteService { db }
    }

    pub async fn create(&self, _ctx: &Ctx, owner_id: Uuid, input: CreateNote) -> Result<Note> {
        if input.body.trim().is_empty() {
            return Err(Error::invalid("note body is required"));
        }
        NoteRepo::create(self.db.pool(), owner_id, &input.body).await
    }

    pub async fn get(&self, _ctx: &Ctx, id: Uuid) -> Result<Note> {
        NoteRepo::find(self.db.pool(), id)
            .await?
            .ok_or_else(|| Error::not_found("note"))
    }

    pub async fn update(&self, _ctx: &Ctx, id: Uuid, input: UpdateNote) -> Result<Note> {
        if input.body.trim().is_empty() {
            return Err(Error::invalid("note body is required"));
        }
        NoteRepo::update(self.db.pool(), id, &input.body)
            .await?
            .ok_or_else(|| Error::not_found("note"))
    }

    pub async fn delete(&self, _ctx: &Ctx, id: Uuid) -> Result<()> {
        if NoteRepo::delete(self.db.pool(), id).await? {
            Ok(())
        } else {
            Err(Error::not_found("note"))
        }
    }
}
