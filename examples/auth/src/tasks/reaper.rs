use hephos::prelude::*;

use crate::state::AppState;

/// Background task that periodically purges dead security tokens (expired or revoked
/// refresh tokens and API keys) older than a configurable retention grace window.
pub struct TokenReaper {
    interval: std::time::Duration,
    grace: time::Duration,
}

impl Default for TokenReaper {
    fn default() -> Self {
        Self {
            // Wake up once per hour.
            interval: std::time::Duration::from_secs(3600),
            // Retain revoked/expired tokens for 7 days before permanent deletion.
            grace: time::Duration::days(7),
        }
    }
}

impl TokenReaper {
    #[allow(dead_code)] // Permitted constructor for tests and custom retention windows
    pub fn new(interval: std::time::Duration, grace: time::Duration) -> Self {
        Self { interval, grace }
    }
}

impl Task<AppState> for TokenReaper {
    fn name(&self) -> &'static str {
        "token_reaper"
    }

    async fn run(&self, ctx: Ctx<AppState>) -> Result<()> {
        let mut interval = tokio::time::interval(self.interval);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            tokio::select! {
                biased;
                _ = ctx.cancel_token().cancelled() => break,
                _ = interval.tick() => {
                    match ctx.state().auth.reap_dead_tokens(self.grace).await {
                        Ok(summary) => {
                            if summary.purged_refresh_tokens > 0 || summary.purged_api_keys > 0 {
                                tracing::info!(
                                    purged_refresh_tokens = summary.purged_refresh_tokens,
                                    purged_api_keys = summary.purged_api_keys,
                                    "data-retention reaper purged dead security tokens"
                                );
                            }
                        }
                        Err(err) => {
                            tracing::error!(%err, "token reaper failed to purge dead tokens");
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn reaper_cancels_cleanly() {
        let state = AppState::offline();
        let ctx = Ctx::detached(state);
        let cancel = ctx.cancel_token().clone();

        let reaper = TokenReaper::new(
            std::time::Duration::from_secs(3600),
            time::Duration::days(7),
        );

        let handle = tokio::spawn(async move { reaper.run(ctx).await });

        cancel.cancel();
        let result = tokio::time::timeout(std::time::Duration::from_secs(1), handle).await;
        assert!(
            result.is_ok(),
            "reaper timed out instead of exiting on cancel"
        );
        assert!(result.unwrap().unwrap().is_ok());
    }
}
