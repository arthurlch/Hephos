use rivet::prelude::*;
use uuid::Uuid;

use crate::services::users::UserService;

pub type Ctx = rivet::Ctx<AppState>;

#[derive(Debug, Clone)]
pub enum AppEvent {
    UserCreated { id: Uuid },
}
impl Event for AppEvent {}

#[derive(Clone)]
pub struct AppState {
    pub db: Db,
    pub events: Events<AppEvent>,
    pub users: UserService,
}

impl AppState {
    pub async fn init() -> Result<Self> {
        let db = Db::connect_from_env().await?;
        db.migrate().await?;
        let events = Events::new(1024);
        let users = UserService::new(db.clone(), events.clone());
        Ok(AppState { db, events, users })
    }
}
