use hephos::prelude::*;
use uuid::Uuid;

use crate::domain::user::{CreateUser, User};
use crate::repos::users::UserRepo;
use crate::state::{AppEvent, Ctx};

#[derive(Clone)]
pub struct UserService {
    db: Db,
    events: Events<AppEvent>,
}

impl UserService {
    pub fn new(db: Db, events: Events<AppEvent>) -> Self {
        UserService { db, events }
    }

    pub async fn get(&self, _ctx: &Ctx, id: Uuid) -> Result<User> {
        UserRepo::find(self.db.pool(), id)
            .await?
            .ok_or_else(|| Error::not_found("user"))
    }

    /// Validate, write inside a transaction, commit, then emit the event. The
    /// event is emitted only after the commit succeeds, so subscribers never see
    /// a user that was rolled back.
    pub async fn create(&self, _ctx: &Ctx, input: CreateUser) -> Result<User> {
        if input.email.trim().is_empty() {
            return Err(Error::invalid("email is required"));
        }

        let mut tx = self.db.begin().await?;
        if UserRepo::find_by_email(tx.exec(), &input.email)
            .await?
            .is_some()
        {
            return Err(Error::conflict("email already registered"));
        }
        let user = UserRepo::create(tx.exec(), &input).await?;
        tx.commit().await?;

        self.events.emit(AppEvent::UserCreated { id: user.id });
        Ok(user)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::AppState;

    async fn state() -> AppState {
        AppState::init()
            .await
            .expect("test database (DATABASE_URL) must be available")
    }

    #[tokio::test]
    async fn create_rejects_empty_email() {
        let state = state().await;
        let ctx = Ctx::detached(state.clone());
        let err = state
            .users
            .create(&ctx, CreateUser { email: "  ".into() })
            .await
            .unwrap_err();
        assert!(matches!(err, Error::Invalid(_)));
    }

    #[tokio::test]
    async fn get_missing_returns_not_found() {
        let state = state().await;
        let ctx = Ctx::detached(state.clone());
        let err = state.users.get(&ctx, Uuid::nil()).await.unwrap_err();
        assert!(matches!(err, Error::NotFound(_)));
    }

    // The §24 simulation-1 chain, end-to-end: a successful write emits `UserCreated`,
    // which a subscriber (the `AuditLog` task in production) observes. Emission happens
    // only after commit, so this also proves subscribers never see a rolled-back user.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn create_emits_user_created_after_commit() {
        let state = state().await;
        let ctx = Ctx::detached(state.clone());
        let mut rx = state.events.subscribe();

        let user = state
            .users
            .create(
                &ctx,
                CreateUser {
                    email: "evt@example.com".into(),
                },
            )
            .await
            .unwrap();

        let event = tokio::time::timeout(std::time::Duration::from_secs(1), rx.recv())
            .await
            .expect("event emitted within 1s")
            .expect("event bus open");
        assert!(matches!(event, AppEvent::UserCreated { id } if id == user.id));
    }
}
