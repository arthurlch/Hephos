use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Item {
    pub id: Uuid,
    pub owner_id: Uuid,
    pub title: String,
    pub description: String,
    pub price_cents: i32,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateItem {
    pub title: String,
    pub description: Option<String>,
    pub price_cents: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateItem {
    pub title: Option<String>,
    pub description: Option<String>,
    pub price_cents: Option<i32>,
}
