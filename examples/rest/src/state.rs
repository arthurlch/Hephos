use rivet::prelude::*;

use crate::services::catalog::CatalogService;

pub type Ctx = rivet::Ctx<AppState>;

#[derive(Clone)]
pub struct AppState {
    pub catalog: CatalogService,
}

impl AppState {
    pub async fn init() -> Result<Self> {
        let catalog = CatalogService::load();
        Ok(AppState { catalog })
    }
}
