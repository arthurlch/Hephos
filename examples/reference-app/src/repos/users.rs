use hephos::db::Db;
use hephos::prelude::*;
use uuid::Uuid;

use crate::domain::user::User;

#[derive(Clone)]
pub struct UsersRepo {
    db: Db,
}

#[derive(sqlx::FromRow)]
pub struct StoredUser {
    pub id: Uuid,
    pub email: String,
    pub password_hash: String,
    pub roles: Vec<String>,
    pub created_at: time::OffsetDateTime,
}

impl UsersRepo {
    pub fn new(db: Db) -> Self {
        Self { db }
    }

    pub async fn create(&self, email: &str, password_hash: &str) -> Result<User> {
        let user = sqlx::query_as::<_, User>(
            "INSERT INTO users (email, password_hash) \
             VALUES ($1, $2) \
             RETURNING id, email, roles, created_at",
        )
        .bind(email)
        .bind(password_hash)
        .fetch_one(self.db.pool())
        .await
        .map_err(|e| {
            if hephos::db::is_unique_violation(&e) {
                Error::conflict("email already registered")
            } else {
                e.into()
            }
        })?;

        Ok(user)
    }

    pub async fn find_by_email(&self, email: &str) -> Result<Option<StoredUser>> {
        let user = sqlx::query_as::<_, StoredUser>(
            "SELECT id, email, password_hash, roles, created_at FROM users WHERE email = $1",
        )
        .bind(email)
        .fetch_optional(self.db.pool())
        .await?;

        Ok(user)
    }

    #[allow(dead_code)]
    pub async fn find_by_id(&self, id: Uuid) -> Result<Option<User>> {
        let user = sqlx::query_as::<_, User>(
            "SELECT id, email, roles, created_at FROM users WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(self.db.pool())
        .await?;

        Ok(user)
    }
}
