use std::sync::Arc;

use argon2::password_hash::rand_core::OsRng;
use argon2::password_hash::SaltString;
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use rivet::prelude::*;

use crate::domain::user::{Credentials, TokenResponse, User};
use crate::repos::users::UserRepo;
use crate::state::Ctx;
use crate::token;

/// Owns registration and login. Holds the signing secret (read once at startup).
#[derive(Clone)]
pub struct AuthService {
    db: Db,
    secret: Arc<Vec<u8>>,
}

impl AuthService {
    pub fn from_env(db: Db) -> Result<Self> {
        let secret = std::env::var("JWT_SECRET")
            .map_err(|_| Error::invalid("JWT_SECRET is not set"))?
            .into_bytes();
        Ok(AuthService { db, secret: Arc::new(secret) })
    }

    pub async fn register(&self, _ctx: &Ctx, input: Credentials) -> Result<User> {
        if input.password.len() < 12 {
            return Err(Error::invalid("password must be at least 12 characters"));
        }
        let hash = hash_password(&input.password)?;

        let mut tx = self.db.begin().await?;
        if UserRepo::find_auth_by_email(tx.exec(), &input.email).await?.is_some() {
            return Err(Error::conflict("email already registered"));
        }
        let user = UserRepo::create(tx.exec(), &input.email, &hash).await?;
        tx.commit().await?;
        Ok(user)
    }

    pub async fn login(&self, _ctx: &Ctx, input: Credentials) -> Result<TokenResponse> {
        let row = UserRepo::find_auth_by_email(self.db.pool(), &input.email).await?;

        // Verify even when the user is missing, against a fixed hash, so response
        // time does not reveal whether the email exists.
        let authenticated = match &row {
            Some(row) => verify_password(&input.password, &row.password_hash),
            None => {
                let _ = verify_password(&input.password, DUMMY_HASH);
                false
            }
        };

        let row = row.filter(|_| authenticated).ok_or(Error::Unauthorized)?;
        let token = token::issue(&self.secret, row.id, row.roles)?;
        Ok(TokenResponse { token })
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

// A valid argon2 hash of a throwaway value, used to equalize timing on the
// user-not-found path.
const DUMMY_HASH: &str = "$argon2id$v=19$m=19456,t=2,p=1$c2FsdHNhbHRzYWx0$RdescudvJCsgt3ub+b+dWRWJTmaaJObG";
