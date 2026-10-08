use hephos::prelude::*;

use crate::services::accounts::AccountService;

pub type Ctx = hephos::Ctx<AppState>;

#[derive(Clone)]
pub struct AppState {
    pub accounts: AccountService,
}

impl AppState {
    pub async fn init() -> Result<Self> {
        Ok(AppState {
            accounts: AccountService::new(),
        })
    }
}
