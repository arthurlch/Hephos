use hephos::prelude::*;

use crate::services::auth::AuthService;
use crate::services::notes::NoteService;
use crate::services::users::UserService;

pub type Ctx = hephos::Ctx<AppState>;

#[derive(Clone)]
pub struct AppState {
    pub db: Db,
    pub auth: AuthService,
    pub users: UserService,
    pub notes: NoteService,
}

impl AppState {
    pub async fn init() -> Result<Self> {
        let db = Db::connect_from_env().await?;
        db.migrate(&sqlx::migrate!("./migrations")).await?;
        let auth = AuthService::from_env(db.clone())?;
        let users = UserService::new(db.clone());
        let notes = NoteService::new(db.clone());
        Ok(AppState {
            db,
            auth,
            users,
            notes,
        })
    }

    #[cfg(test)]
    pub fn offline() -> Self {
        let db =
            Db::connect_lazy("postgres://offline@localhost/offline").expect("offline url parses");
        let auth = AuthService::offline();
        let users = UserService::new(db.clone());
        let notes = NoteService::new(db.clone());
        AppState {
            db,
            auth,
            users,
            notes,
        }
    }
}
