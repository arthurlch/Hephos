use hephos::prelude::*;
use sqlx::PgExecutor;
use uuid::Uuid;

use crate::domain::note::Note;

pub struct NoteRepo;

impl NoteRepo {
    pub async fn create<'e, E: PgExecutor<'e>>(
        exec: E,
        owner_id: Uuid,
        body: &str,
    ) -> Result<Note> {
        let note = sqlx::query_as!(
            Note,
            "insert into notes (owner_id, body) values ($1, $2) \
             returning id, owner_id, body, created_at",
            owner_id,
            body
        )
        .fetch_one(exec)
        .await?;
        Ok(note)
    }

    pub async fn find<'e, E: PgExecutor<'e>>(exec: E, id: Uuid) -> Result<Option<Note>> {
        let note = sqlx::query_as!(
            Note,
            "select id, owner_id, body, created_at from notes where id = $1",
            id
        )
        .fetch_optional(exec)
        .await?;
        Ok(note)
    }

    pub async fn update<'e, E: PgExecutor<'e>>(
        exec: E,
        id: Uuid,
        body: &str,
    ) -> Result<Option<Note>> {
        let note = sqlx::query_as!(
            Note,
            "update notes set body = $2 where id = $1 \
             returning id, owner_id, body, created_at",
            id,
            body
        )
        .fetch_optional(exec)
        .await?;
        Ok(note)
    }

    /// Returns whether a row was deleted, so the service can map a missing id to `404`.
    pub async fn delete<'e, E: PgExecutor<'e>>(exec: E, id: Uuid) -> Result<bool> {
        let result = sqlx::query!("delete from notes where id = $1", id)
            .execute(exec)
            .await?;
        Ok(result.rows_affected() > 0)
    }
}
