use hephos::prelude::*;
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation, decode, encode};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::domain::user::Claims;

const TTL_SECONDS: i64 = 3600;

/// Sign a short-lived access token for an authenticated user.
pub fn issue(secret: &[u8], id: Uuid, roles: Vec<String>) -> Result<String> {
    let exp = (OffsetDateTime::now_utc().unix_timestamp() + TTL_SECONDS) as u64;
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

/// Verify a token against a keyring, returning the caller's [`Identity`]. The token
/// is accepted if it validates under **any** key — so during a signing-key rotation
/// the keyring holds `[current, previous]` and tokens signed with either key keep
/// working until the previous key is retired. An invalid or expired token is
/// [`Error::Unauthorized`], never a `500`.
pub fn verify(keyring: &[Vec<u8>], token: &str) -> Result<Identity> {
    let validation = Validation::new(Algorithm::HS256);
    for secret in keyring {
        if let Ok(data) = decode::<Claims>(token, &DecodingKey::from_secret(secret), &validation) {
            return Ok(Identity::User(Principal {
                id: data.claims.sub,
                roles: data.claims.roles,
            }));
        }
    }
    Err(Error::Unauthorized)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotation_accepts_tokens_from_current_and_previous_keys() {
        let current = b"current-signing-secret".to_vec();
        let previous = b"previous-signing-secret".to_vec();
        let id = Uuid::nil();

        let signed_with_previous = issue(&previous, id, vec![]).unwrap();
        let signed_with_current = issue(&current, id, vec![]).unwrap();
        let keyring = vec![current.clone(), previous.clone()];

        assert!(verify(&keyring, &signed_with_previous).is_ok());
        assert!(verify(&keyring, &signed_with_current).is_ok());
    }

    #[test]
    fn unknown_key_is_rejected() {
        let token = issue(b"the-real-secret", Uuid::nil(), vec![]).unwrap();
        assert!(matches!(
            verify(&[b"a-different-secret".to_vec()], &token),
            Err(Error::Unauthorized)
        ));
    }
}
