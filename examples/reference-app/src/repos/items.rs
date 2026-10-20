use hephos::db::Db;
use hephos::prelude::*;
use uuid::Uuid;

use crate::domain::item::{CreateItem, Item, UpdateItem};

#[derive(Clone)]
pub struct ItemsRepo {
    db: Db,
}

impl ItemsRepo {
    pub fn new(db: Db) -> Self {
        Self { db }
    }

    pub async fn create(&self, owner_id: Uuid, input: CreateItem) -> Result<Item> {
        let desc = input.description.unwrap_or_default();
        let item = sqlx::query_as::<_, Item>(
            "INSERT INTO items (owner_id, title, description, price_cents) \
             VALUES ($1, $2, $3, $4) \
             RETURNING id, owner_id, title, description, price_cents, created_at, updated_at",
        )
        .bind(owner_id)
        .bind(input.title)
        .bind(desc)
        .bind(input.price_cents)
        .fetch_one(self.db.pool())
        .await?;

        Ok(item)
    }

    pub async fn find_by_id(&self, id: Uuid) -> Result<Option<Item>> {
        let item = sqlx::query_as::<_, Item>(
            "SELECT id, owner_id, title, description, price_cents, created_at, updated_at \
             FROM items WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(self.db.pool())
        .await?;

        Ok(item)
    }

    pub async fn list_by_owner(&self, owner_id: Uuid) -> Result<Vec<Item>> {
        let items = sqlx::query_as::<_, Item>(
            "SELECT id, owner_id, title, description, price_cents, created_at, updated_at \
             FROM items WHERE owner_id = $1 ORDER BY created_at DESC",
        )
        .bind(owner_id)
        .fetch_all(self.db.pool())
        .await?;

        Ok(items)
    }

    pub async fn update(&self, id: Uuid, input: UpdateItem) -> Result<Option<Item>> {
        let current = match self.find_by_id(id).await? {
            Some(i) => i,
            None => return Ok(None),
        };

        let title = input.title.unwrap_or(current.title);
        let desc = input.description.unwrap_or(current.description);
        let price = input.price_cents.unwrap_or(current.price_cents);

        let updated = sqlx::query_as::<_, Item>(
            "UPDATE items SET title = $2, description = $3, price_cents = $4, updated_at = now() \
             WHERE id = $1 \
             RETURNING id, owner_id, title, description, price_cents, created_at, updated_at",
        )
        .bind(id)
        .bind(title)
        .bind(desc)
        .bind(price)
        .fetch_optional(self.db.pool())
        .await?;

        Ok(updated)
    }

    pub async fn delete(&self, id: Uuid) -> Result<bool> {
        let res = sqlx::query("DELETE FROM items WHERE id = $1")
            .bind(id)
            .execute(self.db.pool())
            .await?;

        Ok(res.rows_affected() > 0)
    }
}
