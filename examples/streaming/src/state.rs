use hephos::prelude::*;

pub type Ctx = hephos::Ctx<AppState>;

#[derive(Clone)]
pub struct AppState;

impl AppState {
    pub async fn init() -> Result<Self> {
        Ok(AppState)
    }
}
