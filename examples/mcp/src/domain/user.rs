use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UserSummary {
    pub id: Uuid,
    pub name: String,
    pub email: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct SearchQuery {
    /// Free-text match on the user's name or email. Must not be empty.
    pub query: String,
}
