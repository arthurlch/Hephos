use std::time::Duration;

use hephos::agent::{Agent, Limits, Model};

/// Build the reporting agent. Generic over `Model` so the same agent runs against
/// the scripted model in tests and a real provider in production.
pub fn build<M: Model>(model: M) -> Agent<M> {
    Agent::new(model)
        .system("You summarize account activity for support staff. Be concise.")
        .limits(Limits {
            max_turns: 4,
            max_tokens: 1024,
            timeout: Duration::from_secs(30),
        })
}
