use rivet::prelude::*;

use crate::services::auth::AuthService;
use crate::services::users::UserService;

pub type Ctx = rivet::Ctx<AppState>;

// AppState holds the services; each owns its own `Db` handle, so AppState does not
// separately store one (an unread field).
#[derive(Clone)]
pub struct AppState {
    pub auth: AuthService,
    pub users: UserService,
}

impl AppState {
    pub async fn init() -> Result<Self> {
        let db = Db::connect_from_env().await?;
        db.migrate().await?;
        let auth = AuthService::from_env(db.clone())?;
        let users = UserService::new(db);
        Ok(AppState { auth, users })
    }
}
