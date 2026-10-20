use std::time::Duration;

use hephos::prelude::*;

use crate::state::AppState;

pub struct DataReaper {
    interval: Duration,
}

impl Default for DataReaper {
    fn default() -> Self {
        Self {
            interval: Duration::from_secs(3600),
        }
    }
}

impl DataReaper {
    #[allow(dead_code)]
    pub fn new(interval: Duration) -> Self {
        Self { interval }
    }
}

impl Task<AppState> for DataReaper {
    fn name(&self) -> &'static str {
        "data_reaper"
    }

    async fn run(&self, ctx: Ctx<AppState>) -> Result<()> {
        let mut interval = tokio::time::interval(self.interval);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            tokio::select! {
                _ = ctx.cancel_token().cancelled() => {
                    tracing::info!("data_reaper received cancellation");
                    break Ok(());
                }
                _ = interval.tick() => {
                    tracing::debug!("data_reaper maintenance cycle ticked");
                }
            }
        }
    }
}
