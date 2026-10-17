use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

/// A note owned by a single user. `owner_id` is the authorization anchor: every read
/// or mutation is gated by `ctx.require_owner(note.owner_id)`.
#[derive(Debug, Clone, Serialize)]
pub struct Note {
    pub id: Uuid,
    pub owner_id: Uuid,
    pub body: String,
    pub created_at: OffsetDateTime,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreateNote {
    pub body: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateNote {
    pub body: String,
}
