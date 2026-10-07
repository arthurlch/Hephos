use rivet::prelude::*;
use sqlx::PgExecutor;
use uuid::Uuid;

use crate::domain::user::User;

/// The subset needed to verify a login. Kept separate from `User` so the hash
/// never travels with the public profile.
pub struct AuthRow {
    pub id: Uuid,
    pub password_hash: String,
    pub roles: Vec<String>,
}

pub struct UserRepo;

impl UserRepo {
    pub async fn find<'e, E: PgExecutor<'e>>(exec: E, id: Uuid) -> Result<Option<User>> {
        let user = sqlx::query_as!(
            User,
            "select id, email, roles, created_at from users where id = $1",
            id
        )
        .fetch_optional(exec)
        .await?;
        Ok(user)
    }

    pub async fn find_auth_by_email<'e, E: PgExecutor<'e>>(
        exec: E,
        email: &str,
    ) -> Result<Option<AuthRow>> {
        let row = sqlx::query_as!(
            AuthRow,
            "select id, password_hash, roles from users where email = $1",
            email
        )
        .fetch_optional(exec)
        .await?;
        Ok(row)
    }

    pub async fn create<'e, E: PgExecutor<'e>>(
        exec: E,
        email: &str,
        password_hash: &str,
    ) -> Result<User> {
        let user = sqlx::query_as!(
            User,
            "insert into users (email, password_hash) values ($1, $2) \
             returning id, email, roles, created_at",
            email,
            password_hash
        )
        .fetch_one(exec)
        .await?;
        Ok(user)
    }
}
