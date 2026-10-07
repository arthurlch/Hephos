use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

/// Public profile. Never carries the password hash.
#[derive(Debug, Clone, Serialize)]
pub struct User {
    pub id: Uuid,
    pub email: String,
    pub roles: Vec<String>,
    pub created_at: OffsetDateTime,
}

/// Login/registration input.
#[derive(Debug, Clone, Deserialize)]
pub struct Credentials {
    pub email: String,
    pub password: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct TokenResponse {
    pub token: String,
}

/// JWT claims. `sub` is the user id; `roles` drives authorization; `exp` is the
/// expiry (seconds since epoch).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    pub sub: Uuid,
    pub roles: Vec<String>,
    pub exp: usize,
}
