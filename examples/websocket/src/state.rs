use rivet::prelude::*;

pub type Ctx = rivet::Ctx<AppState>;

#[derive(Clone)]
pub struct AppState;

impl AppState {
    pub async fn init() -> Result<Self> {
        Ok(AppState)
    }
}
