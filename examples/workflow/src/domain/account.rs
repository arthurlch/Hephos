use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Deserialize)]
pub struct NewAccount {
    pub email: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Account {
    pub id: Uuid,
    pub email: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Onboarded {
    pub account: Account,
    pub welcome: String,
}
