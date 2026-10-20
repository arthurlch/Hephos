use hephos::prelude::*;
use uuid::Uuid;

use crate::domain::item::{CreateItem, Item, UpdateItem};
use crate::repos::items::ItemsRepo;
use crate::state::Ctx;

#[derive(Clone)]
pub struct ItemsService {
    repo: ItemsRepo,
}

impl ItemsService {
    pub fn new(repo: ItemsRepo) -> Self {
        Self { repo }
    }

    pub async fn create(&self, ctx: &Ctx, input: CreateItem) -> Result<Item> {
        let caller = ctx.require_user()?;
        if input.title.trim().is_empty() {
            return Err(Error::invalid("title must not be empty"));
        }
        if input.price_cents < 0 {
            return Err(Error::invalid("price_cents must be non-negative"));
        }

        let item = self.repo.create(caller.id, input).await?;
        ctx.state()
            .events
            .emit(crate::state::AppEvent::ItemCreated {
                item_id: item.id,
                owner_id: item.owner_id,
            });

        Ok(item)
    }

    pub async fn get(&self, ctx: &Ctx, id: Uuid) -> Result<Item> {
        let _caller = ctx.require_user()?;
        let item = self
            .repo
            .find_by_id(id)
            .await?
            .ok_or_else(|| Error::not_found("item"))?;

        ctx.require_owner(item.owner_id)?;
        Ok(item)
    }

    pub async fn list(&self, ctx: &Ctx) -> Result<Vec<Item>> {
        let caller = ctx.require_user()?;
        self.repo.list_by_owner(caller.id).await
    }

    pub async fn update(&self, ctx: &Ctx, id: Uuid, input: UpdateItem) -> Result<Item> {
        let _caller = ctx.require_user()?;
        let current = self
            .repo
            .find_by_id(id)
            .await?
            .ok_or_else(|| Error::not_found("item"))?;

        ctx.require_owner(current.owner_id)?;

        let updated = self
            .repo
            .update(id, input)
            .await?
            .ok_or_else(|| Error::not_found("item"))?;

        Ok(updated)
    }

    pub async fn delete(&self, ctx: &Ctx, id: Uuid) -> Result<()> {
        let _caller = ctx.require_user()?;
        let current = self
            .repo
            .find_by_id(id)
            .await?
            .ok_or_else(|| Error::not_found("item"))?;

        ctx.require_owner(current.owner_id)?;

        self.repo.delete(id).await?;
        Ok(())
    }
}
