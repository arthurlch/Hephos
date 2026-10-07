use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize)]
pub struct UserSummary {
    pub id: Uuid,
    pub name: String,
    pub email: String,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct SearchQuery {
    /// Free-text match on the user's name or email. Must not be empty.
    pub query: String,
}
