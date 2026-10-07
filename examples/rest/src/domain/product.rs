use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize)]
pub struct Product {
    pub id: Uuid,
    pub name: String,
    pub category: String,
    pub price_cents: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SearchRequest {
    pub category: String,
    pub max_price_cents: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ListQuery {
    pub category: Option<String>,
}
