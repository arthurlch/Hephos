use std::sync::Arc;

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
        let secret = std::env::var("JWT_SECRET")
            .unwrap_or_else(|_| "reference-app-default-secret-at-least-32-bytes".to_string());
        Ok(Self::new(repo, secret))
    }

    pub async fn register(&self, ctx: &Ctx, creds: AuthCredentials) -> Result<User> {
        if creds.password.len() < 8 {
            return Err(Error::invalid("password must be at least 8 characters"));
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
        let stored = self
            .repo
            .find_by_email(&creds.email)
            .await?
            .ok_or(Error::Unauthorized)?;

        let parsed_hash = PasswordHash::new(&stored.password_hash)
            .map_err(|_| Error::internal("malformed password hash"))?;

        Argon2::default()
            .verify_password(creds.password.as_bytes(), &parsed_hash)
            .map_err(|_| Error::Unauthorized)?;

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
