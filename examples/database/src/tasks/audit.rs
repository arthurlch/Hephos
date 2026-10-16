use hephos::prelude::*;
use tokio::sync::broadcast::error::RecvError;

use crate::state::{AppEvent, AppState};

/// Consumes AppEvent in the background. A lagged receiver resyncs from the source of
/// truth rather than panicking — the bus is best-effort by design.
pub struct AuditLog;

impl Task<AppState> for AuditLog {
    fn name(&self) -> &'static str {
        "audit_log"
    }

    async fn run(&self, ctx: Ctx<AppState>) -> Result<()> {
        let mut rx = ctx.state().events.subscribe();
        loop {
            tokio::select! {
                _ = ctx.cancel_token().cancelled() => break,
                event = rx.recv() => match event {
                    Ok(AppEvent::UserCreated { id }) => tracing::info!(%id, "user created"),
                    // Lagged is recoverable (resync from source of truth); Closed is
                    // terminal — break, or `recv()` returns immediately and busy-loops.
                    Err(RecvError::Lagged(skipped)) => tracing::warn!(%skipped, "audit lagged"),
                    Err(RecvError::Closed) => break,
                }
            }
        }
        Ok(())
    }
}
