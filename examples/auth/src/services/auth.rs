use std::sync::{Arc, OnceLock};

use argon2::password_hash::SaltString;
use argon2::password_hash::rand_core::OsRng;
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use hephos::prelude::*;
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

use crate::domain::user::{
    ApiKeyCreated, CreateApiKey, Credentials, RefreshRequest, TokenPair, User,
};
use crate::repos::api_keys::ApiKeyRepo;
use crate::repos::refresh_tokens::RefreshTokenRepo;
use crate::repos::users::UserRepo;
use crate::state::Ctx;
use crate::token;

// Refresh tokens are long-lived; rotation on every use keeps the window of any single
// leaked token short even though the family can live this long.
const REFRESH_TTL_DAYS: i64 = 30;

/// Owns registration, login, the token lifecycle, and API-key management. Holds the
/// verification keyring (read once at startup); `keyring[0]` is the current signing key.
#[derive(Clone)]
pub struct AuthService {
    db: Db,
    keyring: Arc<Vec<Vec<u8>>>,
}

impl AuthService {
    pub fn from_env(db: Db) -> Result<Self> {
        let current = std::env::var("JWT_SECRET")
            .map_err(|_| Error::invalid("JWT_SECRET is not set"))?
            .into_bytes();
        let mut keyring = vec![current];
        // A previous key, set only during a signing-key rotation, keeps tokens signed
        // with the old key verifying until it is retired.
        if let Ok(previous) = std::env::var("JWT_SECRET_PREVIOUS") {
            if !previous.is_empty() {
                keyring.push(previous.into_bytes());
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
        if input.password.len() < 12 {
            return Err(Error::invalid("password must be at least 12 characters"));
        }
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

        RefreshTokenRepo::revoke(tx.exec(), stored.id).await?;
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
}

fn hash_password(password: &str) -> Result<String> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|e| Error::internal(format!("hash password: {e}")))
}

fn verify_password(password: &str, hash: &str) -> bool {
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
