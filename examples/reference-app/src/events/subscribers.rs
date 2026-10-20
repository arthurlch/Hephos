use hephos::prelude::*;

use crate::state::{AppEvent, AppState};

pub fn handle_event(event: &AppEvent) {
    match event {
        AppEvent::ItemCreated { item_id, owner_id } => {
            tracing::info!(%item_id, %owner_id, "AppEvent: item created");
        }
        AppEvent::UserRegistered { user_id, email } => {
            tracing::info!(%user_id, %email, "AppEvent: user registered");
        }
    }
}

pub struct EventSubscriberTask;

impl Task<AppState> for EventSubscriberTask {
    fn name(&self) -> &'static str {
        "event_subscriber"
    }

    async fn run(&self, ctx: Ctx<AppState>) -> Result<()> {
        let mut rx = ctx.state().events.subscribe();
        loop {
            tokio::select! {
                _ = ctx.cancel_token().cancelled() => {
                    break Ok(());
                }
                res = rx.recv() => {
                    if let Ok(event) = res {
                        handle_event(&event);
                    }
                }
            }
        }
    }
}
