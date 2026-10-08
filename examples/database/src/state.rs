use hephos::prelude::*;
use uuid::Uuid;

use crate::services::users::UserService;

pub type Ctx = hephos::Ctx<AppState>;

#[derive(Debug, Clone)]
pub enum AppEvent {
    UserCreated { id: Uuid },
}
impl Event for AppEvent {}

// AppState holds the shared event bus and the services. The services own their own
// `Db` handle, so AppState does not separately store one (an unread field).
#[derive(Clone)]
pub struct AppState {
    pub events: Events<AppEvent>,
    pub users: UserService,
}

impl AppState {
    pub async fn init() -> Result<Self> {
        let db = Db::connect_from_env().await?;
        db.migrate().await?;
        let events = Events::new(1024);
        let users = UserService::new(db, events.clone());
        Ok(AppState { events, users })
    }
}
