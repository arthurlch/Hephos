use argon2::password_hash::rand_core::{OsRng, RngCore};
use hephos::prelude::*;
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation, decode, encode};
use sha2::{Digest, Sha256};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::domain::user::Claims;

// Access tokens are short-lived: the refresh token is how a session is kept alive, so
// a leaked access token is only useful for a few minutes.
const TTL_SECONDS: i64 = 900;

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

/// Mint a new opaque refresh token. Returns `(raw, hash)`: the raw token is returned
/// to the client once; only the hash is persisted, so a database dump cannot be
/// replayed as a session.
pub fn new_refresh_token() -> (String, String) {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    let raw = to_hex(&bytes);
    let hash = hash_refresh_token(&raw);
    (raw, hash)
}

/// Hash a presented refresh token for a constant-shape lookup against the stored hash.
/// The token is high-entropy random, so a fast digest (not a password KDF) is correct.
pub fn hash_refresh_token(raw: &str) -> String {
    to_hex(&Sha256::digest(raw.as_bytes()))
}

fn to_hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(s, "{b:02x}");
    }
    s
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
    fn refresh_token_hash_is_stable_and_not_the_raw_token() {
        let (raw, hash) = new_refresh_token();
        assert_ne!(raw, hash);
        assert_eq!(hash, hash_refresh_token(&raw));
        // 32 random bytes and a SHA-256 digest both hex-encode to 64 chars.
        assert_eq!(raw.len(), 64);
        assert_eq!(hash.len(), 64);
    }

    #[test]
    fn refresh_tokens_are_unique() {
        let (a, _) = new_refresh_token();
        let (b, _) = new_refresh_token();
        assert_ne!(a, b);
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
