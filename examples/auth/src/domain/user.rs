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

/// What a successful login or refresh returns: a short-lived access token (a JWT)
/// and a long-lived opaque refresh token. The refresh token is shown to the client
/// exactly once — only its hash is stored.
#[derive(Debug, Clone, Serialize)]
pub struct TokenPair {
    pub access_token: String,
    pub refresh_token: String,
}

/// Body for `/auth/refresh` and `/auth/logout`.
#[derive(Debug, Clone, Deserialize)]
pub struct RefreshRequest {
    pub refresh_token: String,
}

/// JWT claims. `sub` is the user id; `roles` drives authorization; `exp` is the
/// expiry (seconds since epoch).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    pub sub: Uuid,
    pub roles: Vec<String>,
    pub exp: u64,
}
