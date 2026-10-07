use rivet::prelude::*;

use crate::services::accounts::AccountService;

pub type Ctx = rivet::Ctx<AppState>;

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
