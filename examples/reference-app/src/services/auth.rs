use std::sync::{Arc, OnceLock};

use argon2::password_hash::SaltString;
use argon2::password_hash::rand_core::OsRng;
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use hephos::prelude::*;
use jsonwebtoken::{DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

use crate::domain::user::{AuthCredentials, AuthTokenResponse, User};
use crate::repos::users::UsersRepo;
use crate::state::Ctx;

const MIN_PASSWORD_LEN: usize = 8;
const MAX_PASSWORD_LEN: usize = 1024;
const MIN_SECRET_LEN: usize = 32;

#[derive(Debug, Serialize, Deserialize)]
struct Claims {
    sub: String,
    roles: Vec<String>,
    exp: i64,
}

#[derive(Clone)]
pub struct AuthService {
    repo: UsersRepo,
    jwt_secret: Arc<String>,
}

impl AuthService {
    pub fn new(repo: UsersRepo, jwt_secret: String) -> Self {
        Self {
            repo,
            jwt_secret: Arc::new(jwt_secret),
        }
    }

    pub fn from_env(repo: UsersRepo) -> Result<Self> {
        let secret = match std::env::var("JWT_SECRET") {
            Ok(val) => {
                if val.len() < MIN_SECRET_LEN {
                    return Err(Error::invalid(format!(
                        "JWT_SECRET must be at least {MIN_SECRET_LEN} bytes"
                    )));
                }
                val
            }
            Err(_) => {
                #[cfg(any(test, debug_assertions))]
                {
                    "reference-app-default-secret-at-least-32-bytes".to_string()
                }
                #[cfg(not(any(test, debug_assertions)))]
                {
                    return Err(Error::invalid(
                        "JWT_SECRET environment variable is required and must be at least 32 bytes",
                    ));
                }
            }
        };
        Ok(Self::new(repo, secret))
    }

    pub async fn register(&self, ctx: &Ctx, creds: AuthCredentials) -> Result<User> {
        if creds.password.len() < MIN_PASSWORD_LEN {
            return Err(Error::invalid(format!(
                "password must be at least {MIN_PASSWORD_LEN} characters"
            )));
        }
        if creds.password.len() > MAX_PASSWORD_LEN {
            return Err(Error::invalid(format!(
                "password must be at most {MAX_PASSWORD_LEN} characters"
            )));
        }
        if !creds.email.contains('@') {
            return Err(Error::invalid("invalid email address"));
        }

        let salt = SaltString::generate(&mut OsRng);
        let password_hash = Argon2::default()
            .hash_password(creds.password.as_bytes(), &salt)
            .map_err(|_| Error::internal("failed to hash password"))?
            .to_string();

        let user = self.repo.create(&creds.email, &password_hash).await?;
        ctx.state()
            .events
            .emit(crate::state::AppEvent::UserRegistered {
                user_id: user.id,
                email: user.email.clone(),
            });

        Ok(user)
    }

    pub async fn login(&self, _ctx: &Ctx, creds: AuthCredentials) -> Result<AuthTokenResponse> {
        if creds.password.len() > MAX_PASSWORD_LEN {
            return Err(Error::Unauthorized);
        }

        let stored = self.repo.find_by_email(&creds.email).await?;

        // Verify even when the user is missing, against a known-valid decoy hash,
        // so response time does not reveal whether the email exists.
        let authenticated = match &stored {
            Some(stored) => verify_password(&creds.password, &stored.password_hash),
            None => {
                let _ = verify_password(&creds.password, decoy_hash());
                false
            }
        };

        let stored = stored
            .filter(|_| authenticated)
            .ok_or(Error::Unauthorized)?;

        let exp = (OffsetDateTime::now_utc() + Duration::hours(24)).unix_timestamp();
        let claims = Claims {
            sub: stored.id.to_string(),
            roles: stored.roles.clone(),
            exp,
        };

        let token = jsonwebtoken::encode(
            &Header::default(),
            &claims,
            &EncodingKey::from_secret(self.jwt_secret.as_bytes()),
        )
        .map_err(|_| Error::internal("token creation failed"))?;

        let user = User {
            id: stored.id,
            email: stored.email,
            roles: stored.roles,
            created_at: stored.created_at,
        };

        Ok(AuthTokenResponse { token, user })
    }

    pub async fn verify(&self, token: &str) -> Result<Identity> {
        let mut validation = Validation::default();
        validation.validate_exp = true;

        let token_data = jsonwebtoken::decode::<Claims>(
            token,
            &DecodingKey::from_secret(self.jwt_secret.as_bytes()),
            &validation,
        )
        .map_err(|_| Error::Unauthorized)?;

        let id = Uuid::parse_str(&token_data.claims.sub).map_err(|_| Error::Unauthorized)?;

        Ok(Identity::User(Principal {
            id,
            roles: token_data.claims.roles,
        }))
    }
}

fn verify_password(password: &str, hash: &str) -> bool {
    match PasswordHash::new(hash) {
        Ok(parsed) => Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .is_ok(),
        Err(_) => false,
    }
}

fn decoy_hash() -> &'static str {
    static DECOY: OnceLock<String> = OnceLock::new();
    DECOY
        .get_or_init(|| {
            let salt = SaltString::generate(&mut OsRng);
            Argon2::default()
                .hash_password(b"hephos-timing-equalizer", &salt)
                .expect("decoy hash is valid")
                .to_string()
        })
        .as_str()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_bounds_constants() {
        assert_eq!(MIN_PASSWORD_LEN, 8);
        assert_eq!(MAX_PASSWORD_LEN, 1024);
        assert_eq!(MIN_SECRET_LEN, 32);
    }

    #[test]
    fn decoy_hash_is_valid_and_consistent() {
        let h1 = decoy_hash();
        let h2 = decoy_hash();
        assert_eq!(h1, h2);
        assert!(verify_password("hephos-timing-equalizer", h1));
        assert!(!verify_password("wrong-password", h1));
    }
}
