use hephos::db::Db;
use hephos::prelude::*;
use uuid::Uuid;

use crate::repos::items::ItemsRepo;
use crate::repos::users::UsersRepo;
use crate::services::assistant::AssistantService;
use crate::services::auth::AuthService;
use crate::services::items::ItemsService;

pub type Ctx = hephos::Ctx<AppState>;

#[derive(Debug, Clone)]
pub enum AppEvent {
    ItemCreated { item_id: Uuid, owner_id: Uuid },
    UserRegistered { user_id: Uuid, email: String },
}

impl Event for AppEvent {}

#[derive(Clone)]
pub struct AppState {
    pub db: Db,
    pub items: ItemsService,
    pub auth: AuthService,
    pub assistant: AssistantService,
    pub workflows: Workflows,
    pub events: Events<AppEvent>,
}

impl AppState {
    pub async fn init() -> Result<Self> {
        let db_url = std::env::var("DATABASE_URL")
            .unwrap_or_else(|_| "postgres://postgres:postgres@localhost:5432/hephos_ref".into());
        let db = Db::connect_lazy(&db_url)?;
        Self::with_db(db).await
    }

    pub async fn with_db(db: Db) -> Result<Self> {
        let items_repo = ItemsRepo::new(db.clone());
        let users_repo = UsersRepo::new(db.clone());
        let items = ItemsService::new(items_repo);
        let auth = AuthService::from_env(users_repo)?;
        let assistant = AssistantService::new();
        let workflows = Workflows::in_memory();
        let events = Events::new(1024);

        Ok(AppState {
            db,
            items,
            auth,
            assistant,
            workflows,
            events,
        })
    }
}
