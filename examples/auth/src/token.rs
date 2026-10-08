use hephos::prelude::*;
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation, decode, encode};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::domain::user::Claims;

const TTL_SECONDS: i64 = 3600;

/// Sign a short-lived access token for an authenticated user.
pub fn issue(secret: &[u8], id: Uuid, roles: Vec<String>) -> Result<String> {
    let exp = (OffsetDateTime::now_utc().unix_timestamp() + TTL_SECONDS) as usize;
    let claims = Claims {
        sub: id,
        roles,
        exp,
    };
    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(secret),
    )
    .map_err(|e| Error::internal(format!("sign token: {e}")))
}

/// Verify a token's signature and expiry, returning the caller's [`Identity`].
/// An invalid or expired token is [`Error::Unauthorized`], never a 500.
pub fn verify(secret: &[u8], token: &str) -> Result<Identity> {
    let data = decode::<Claims>(
        token,
        &DecodingKey::from_secret(secret),
        &Validation::new(Algorithm::HS256),
    )
    .map_err(|_| Error::Unauthorized)?;

    Ok(Identity::User(Principal {
        id: data.claims.sub,
        roles: data.claims.roles,
    }))
}
