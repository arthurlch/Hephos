use hephos::prelude::*;
use uuid::Uuid;

use crate::domain::account::{Account, NewAccount};
use crate::state::Ctx;

#[derive(Clone)]
pub struct AccountService;

impl AccountService {
    pub fn new() -> Self {
        AccountService
    }

    pub async fn register(&self, _ctx: &Ctx, input: NewAccount) -> Result<Account> {
        if !input.email.contains('@') {
            return Err(Error::invalid("email is not valid"));
        }
        Ok(Account {
            id: Uuid::new_v4(),
            email: input.email,
        })
    }
}
