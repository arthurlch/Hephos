use std::time::Duration;

use hephos::prelude::*;

pub type Ctx = hephos::Ctx<AppState>;

#[derive(Clone)]
pub struct AppState {
    pub tick_interval: Duration,
}

impl AppState {
    pub async fn init() -> Result<Self> {
        Ok(AppState {
            tick_interval: Duration::from_secs(1),
        })
    }

    #[cfg(test)]
    pub fn for_test() -> Self {
        AppState {
            tick_interval: Duration::from_millis(1),
        }
    }
}
