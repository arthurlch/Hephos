use hephos::prelude::*;
use uuid::Uuid;

use crate::domain::user::User;
use crate::repos::users::UserRepo;
use crate::state::Ctx;

#[derive(Clone)]
pub struct UserService {
    db: Db,
}

impl UserService {
    pub fn new(db: Db) -> Self {
        UserService { db }
    }

    pub async fn get(&self, _ctx: &Ctx, id: Uuid) -> Result<User> {
        UserRepo::find(self.db.pool(), id)
            .await?
            .ok_or_else(|| Error::not_found("user"))
    }
}
