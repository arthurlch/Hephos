use std::sync::{Arc, OnceLock};

use argon2::password_hash::SaltString;
use argon2::password_hash::rand_core::OsRng;
use argon2::{Algorithm, Argon2, Params, PasswordHash, PasswordHasher, PasswordVerifier, Version};
use hephos::prelude::*;
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

use crate::domain::user::{
    ApiKeyCreated, CreateApiKey, Credentials, ReapSummary, RefreshRequest, TokenPair, User,
};
use crate::repos::api_keys::ApiKeyRepo;
use crate::repos::refresh_tokens::RefreshTokenRepo;
use crate::repos::users::UserRepo;
use crate::services::token;
use crate::state::Ctx;

// Refresh tokens are long-lived; rotation on every use keeps the window of any single
// leaked token short even though the family can live this long.
const REFRESH_TTL_DAYS: i64 = 30;

// Password policy (NIST SP 800-63B: favor length, avoid composition rules). The upper
// bound caps the work argon2 does, so a multi-megabyte "password" can't become a DoS.
const MIN_PASSWORD_LEN: usize = 12;
const MAX_PASSWORD_LEN: usize = 1024;

/// Owns registration, login, the token lifecycle, and API-key management. Holds the
/// verification keyring (read once at startup); `keyring[0]` is the current signing key.
#[derive(Clone)]
pub struct AuthService {
    db: Db,
    keyring: Arc<Vec<Vec<u8>>>,
}

impl AuthService {
    pub fn from_env(db: Db) -> Result<Self> {
        let current =
            std::env::var("JWT_SECRET").map_err(|_| Error::invalid("JWT_SECRET is not set"))?;
        // Reject an empty/short secret: HMAC accepts a zero-length key, so an unset-but-
        // exported `JWT_SECRET=""` would otherwise become a predictable signing key that
        // lets anyone forge an admin token. Require real entropy (≥ 32 bytes).
        let mut keyring = vec![require_secret("JWT_SECRET", current)?];
        // A previous key, set only during a signing-key rotation, keeps tokens signed
        // with the old key verifying until it is retired.
        if let Ok(previous) = std::env::var("JWT_SECRET_PREVIOUS") {
            if !previous.is_empty() {
                keyring.push(require_secret("JWT_SECRET_PREVIOUS", previous)?);
            }
        }
        Ok(AuthService {
            db,
            keyring: Arc::new(keyring),
        })
    }

    /// Offline constructor for the `routes_build` canary: a lazily-connected pool and
    /// a dummy keyring, so the router can be assembled without a live database or
    /// `JWT_SECRET`. The verify closure it feeds is never called in that test.
    #[cfg(test)]
    pub fn offline() -> Self {
        let db =
            Db::connect_lazy("postgres://offline@localhost/offline").expect("offline url parses");
        AuthService {
            db,
            keyring: Arc::new(vec![b"offline".to_vec()]),
        }
    }

    /// Resolve a presented bearer token to an [`Identity`] — the resolver the auth
    /// seam calls on every protected request. A prefixed value is an API key, looked
    /// up in the key store; anything else is verified as a JWT against the keyring.
    pub async fn authenticate(&self, token: &str) -> Result<Identity> {
        if token.starts_with(token::API_KEY_PREFIX) {
            self.authenticate_api_key(token).await
        } else {
            token::verify(&self.keyring, token)
        }
    }

    async fn authenticate_api_key(&self, token: &str) -> Result<Identity> {
        // No decoy/constant-time dance like `login`: the key is high-entropy random and
        // the lookup is a single indexed equality on its hash, so a missing key leaks no
        // useful timing signal (nothing an attacker can steer toward a valid key).
        let hash = token::hash_opaque_token(token);
        let key = ApiKeyRepo::find_active_by_hash(self.db.pool(), &hash)
            .await?
            .ok_or(Error::Unauthorized)?;
        if let Some(expires_at) = key.expires_at {
            if expires_at <= OffsetDateTime::now_utc() {
                return Err(Error::Unauthorized);
            }
        }
        Ok(Identity::User(Principal {
            id: key.id,
            roles: key.roles,
        }))
    }

    /// Mint a machine-to-machine API key. The raw `hephos_sk_…` value is returned once;
    /// only its hash is stored.
    pub async fn mint_api_key(&self, _ctx: &Ctx, input: CreateApiKey) -> Result<ApiKeyCreated> {
        if input.name.trim().is_empty() {
            return Err(Error::invalid("api key name is required"));
        }
        let (raw, hash) = token::new_api_key();
        let id = ApiKeyRepo::insert(self.db.pool(), &input.name, &hash, &input.roles, None).await?;
        Ok(ApiKeyCreated { id, api_key: raw })
    }

    /// Revoke an API key by id. An unknown or already-revoked id is a `404`.
    pub async fn revoke_api_key(&self, _ctx: &Ctx, id: Uuid) -> Result<()> {
        if ApiKeyRepo::revoke(self.db.pool(), id).await? {
            Ok(())
        } else {
            Err(Error::not_found("api key"))
        }
    }

    pub async fn register(&self, _ctx: &Ctx, input: Credentials) -> Result<User> {
        validate_password(&input.password)?;
        let hash = hash_password(&input.password)?;

        let mut tx = self.db.begin().await?;
        if UserRepo::find_auth_by_email(tx.exec(), &input.email)
            .await?
            .is_some()
        {
            return Err(Error::conflict("email already registered"));
        }
        let user = UserRepo::create(tx.exec(), &input.email, &hash).await?;
        tx.commit().await?;
        Ok(user)
    }

    pub async fn login(&self, _ctx: &Ctx, input: Credentials) -> Result<TokenPair> {
        // Reject over-length input before hashing: no registered password can exceed
        // the policy bound, so this is never a valid login, and it stops an attacker
        // forcing argon2 work on a multi-megabyte body. `Unauthorized` (not `invalid`)
        // keeps login's existence-opacity — the same answer a wrong password gets.
        if input.password.len() > MAX_PASSWORD_LEN {
            return Err(Error::Unauthorized);
        }
        let row = UserRepo::find_auth_by_email(self.db.pool(), &input.email).await?;

        // Verify even when the user is missing, against a known-valid decoy hash,
        // so response time does not reveal whether the email exists.
        let authenticated = match &row {
            Some(row) => verify_password(&input.password, &row.password_hash),
            None => {
                let _ = verify_password(&input.password, decoy_hash());
                false
            }
        };

        let row = row.filter(|_| authenticated).ok_or(Error::Unauthorized)?;
        self.mint_pair(self.db.pool(), row.id, row.roles).await
    }

    /// Exchange a valid refresh token for a new access + refresh pair, rotating the
    /// presented token. If an *already-revoked* token is presented the token has been
    /// stolen (the legitimate client rotated it away), so the whole family is revoked.
    pub async fn refresh(&self, _ctx: &Ctx, input: RefreshRequest) -> Result<TokenPair> {
        let hash = token::hash_opaque_token(&input.refresh_token);

        let mut tx = self.db.begin().await?;
        let stored = RefreshTokenRepo::find_by_hash(tx.exec(), &hash)
            .await?
            .ok_or(Error::Unauthorized)?;

        if stored.revoked_at.is_some() {
            RefreshTokenRepo::revoke_all_for_user(tx.exec(), stored.user_id).await?;
            tx.commit().await?;
            return Err(Error::Unauthorized);
        }
        if stored.expires_at <= OffsetDateTime::now_utc() {
            return Err(Error::Unauthorized);
        }

        let user = UserRepo::find(tx.exec(), stored.user_id)
            .await?
            .ok_or(Error::Unauthorized)?;

        // Atomic single-use check: `revoke`'s `where revoked_at is null` means only one
        // of two concurrent rotations of this token wins. If we lost the race the token
        // was already rotated away — treat that like reuse and revoke the whole family.
        if !RefreshTokenRepo::revoke(tx.exec(), stored.id).await? {
            RefreshTokenRepo::revoke_all_for_user(tx.exec(), stored.user_id).await?;
            tx.commit().await?;
            return Err(Error::Unauthorized);
        }
        let pair = self.mint_pair(tx.exec(), user.id, user.roles).await?;
        tx.commit().await?;
        Ok(pair)
    }

    /// Revoke a refresh token. Idempotent and existence-opaque: an unknown token still
    /// returns success, so logout never reveals whether the token was real.
    pub async fn logout(&self, _ctx: &Ctx, input: RefreshRequest) -> Result<()> {
        let hash = token::hash_opaque_token(&input.refresh_token);
        RefreshTokenRepo::revoke_by_hash(self.db.pool(), &hash).await?;
        Ok(())
    }

    /// Issue an access token and a freshly-stored refresh token in one step.
    async fn mint_pair<'e, E: sqlx::PgExecutor<'e>>(
        &self,
        exec: E,
        user_id: Uuid,
        roles: Vec<String>,
    ) -> Result<TokenPair> {
        let access_token = token::issue(&self.keyring[0], user_id, roles)?;
        let (raw, hash) = token::new_refresh_token();
        let expires_at = OffsetDateTime::now_utc() + Duration::days(REFRESH_TTL_DAYS);
        RefreshTokenRepo::insert(exec, user_id, &hash, expires_at).await?;
        Ok(TokenPair {
            access_token,
            refresh_token: raw,
        })
    }

    /// Purge expired or revoked refresh tokens and API keys older than the `grace` window.
    ///
    /// The grace window keeps recently revoked/expired tokens visible for audit logs and
    /// token-reuse detection before permanent deletion.
    pub async fn reap_dead_tokens(&self, grace: Duration) -> Result<ReapSummary> {
        let cutoff = OffsetDateTime::now_utc() - grace;
        let mut tx = self.db.begin().await?;
        let purged_refresh_tokens = RefreshTokenRepo::purge_dead(tx.exec(), cutoff).await?;
        let purged_api_keys = ApiKeyRepo::purge_dead(tx.exec(), cutoff).await?;
        tx.commit().await?;
        Ok(ReapSummary {
            purged_refresh_tokens,
            purged_api_keys,
        })
    }
}

// Minimum signing-secret length. A JWT HMAC key shorter than its 256-bit output adds
// no security; below this is almost always a misconfiguration (empty/placeholder).
const MIN_SECRET_LEN: usize = 32;

fn require_secret(name: &str, value: String) -> Result<Vec<u8>> {
    if value.len() < MIN_SECRET_LEN {
        return Err(Error::invalid(format!(
            "{name} must be at least {MIN_SECRET_LEN} bytes"
        )));
    }
    Ok(value.into_bytes())
}

/// The canonical place for password rules. Checked in the service (business rule),
/// not at the edge (serde only validates shape). Returns [`Error::invalid`].
fn validate_password(password: &str) -> Result<()> {
    if password.len() < MIN_PASSWORD_LEN {
        return Err(Error::invalid(format!(
            "password must be at least {MIN_PASSWORD_LEN} characters"
        )));
    }
    if password.len() > MAX_PASSWORD_LEN {
        return Err(Error::invalid(format!(
            "password must be at most {MAX_PASSWORD_LEN} characters"
        )));
    }
    Ok(())
}

// OWASP-recommended Argon2id parameters (2024): 19 MiB memory, 2 iterations, 1 lane.
// They are written into the PHC hash string, so verification reads them back — raising
// these later strengthens new hashes without invalidating existing ones.
fn hasher() -> Argon2<'static> {
    let params = Params::new(19 * 1024, 2, 1, None).expect("argon2 params are valid");
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
}

fn hash_password(password: &str) -> Result<String> {
    let salt = SaltString::generate(&mut OsRng);
    hasher()
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|e| Error::internal(format!("hash password: {e}")))
}

fn verify_password(password: &str, hash: &str) -> bool {
    // Verification uses the parameters embedded in the stored hash, so a default
    // instance is correct regardless of what `hasher()` currently produces.
    match PasswordHash::new(hash) {
        Ok(parsed) => Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .is_ok(),
        Err(_) => false,
    }
}

// A guaranteed-valid decoy hash, computed once. Computing it (rather than
// hardcoding a literal that might fail to parse) ensures the user-not-found path
// performs the same argon2 work as a real verification, closing the timing side
// channel.
fn decoy_hash() -> &'static str {
    static DECOY: OnceLock<String> = OnceLock::new();
    DECOY
        .get_or_init(|| hash_password("hephos-timing-equalizer").expect("decoy hash is valid"))
        .as_str()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_policy_enforces_length_bounds() {
        assert!(validate_password(&"a".repeat(MIN_PASSWORD_LEN - 1)).is_err());
        assert!(validate_password(&"a".repeat(MIN_PASSWORD_LEN)).is_ok());
        assert!(validate_password(&"a".repeat(MAX_PASSWORD_LEN)).is_ok());
        assert!(validate_password(&"a".repeat(MAX_PASSWORD_LEN + 1)).is_err());
    }

    #[test]
    fn hash_roundtrips_and_uses_tuned_argon2id() {
        let hash = hash_password("correct horse battery").unwrap();
        // PHC string identifies the algorithm and the parameters we configured.
        assert!(hash.starts_with("$argon2id$"));
        assert!(hash.contains("m=19456,t=2,p=1"));
        assert!(verify_password("correct horse battery", &hash));
        assert!(!verify_password("wrong password", &hash));
    }

    #[test]
    fn verify_rejects_a_malformed_hash() {
        assert!(!verify_password("whatever", "not-a-phc-hash"));
    }

    #[test]
    fn require_secret_rejects_short_and_empty_keys() {
        assert!(require_secret("JWT_SECRET", String::new()).is_err());
        assert!(require_secret("JWT_SECRET", "a".repeat(MIN_SECRET_LEN - 1)).is_err());
        assert!(require_secret("JWT_SECRET", "a".repeat(MIN_SECRET_LEN)).is_ok());
    }
}
