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

/// Request to mint a machine-to-machine API key. `roles` are the scopes the key
/// carries when it authenticates.
#[derive(Debug, Clone, Deserialize)]
pub struct CreateApiKey {
    pub name: String,
    #[serde(default)]
    pub roles: Vec<String>,
}

/// The one-time response when an API key is created: the raw `hephos_sk_…` value is
/// shown here and never again (only its hash is stored).
#[derive(Debug, Clone, Serialize)]
pub struct ApiKeyCreated {
    pub id: Uuid,
    pub api_key: String,
}

/// JWT claims. `sub` is the user id; `roles` drives authorization; `exp` is the
/// expiry (seconds since epoch).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    pub sub: Uuid,
    pub roles: Vec<String>,
    pub exp: u64,
}

/// Summary of dead security rows purged by the data-retention reaper.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ReapSummary {
    pub purged_refresh_tokens: u64,
    pub purged_api_keys: u64,
}
