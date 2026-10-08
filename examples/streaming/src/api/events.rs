use std::convert::Infallible;
use std::time::Duration;

use futures::Stream;
use hephos::prelude::*;
use hephos::sse::{Event, KeepAlive, Sse};

use crate::state::{AppState, Ctx};

pub fn routes() -> Router<AppState> {
    Router::new().get("/events", stream)
}

/// Emit ten ticks, one per second, then end. Production stops early if the client
/// disconnects or the server shuts down (both fire the cancellation token).
pub async fn stream(ctx: Ctx) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let body = async_stream::stream! {
        let mut tick = tokio::time::interval(Duration::from_secs(1));
        for n in 0..10u32 {
            tokio::select! {
                _ = ctx.cancel_token().cancelled() => break,
                _ = tick.tick() => {
                    yield Ok(Event::default().data(n.to_string()));
                }
            }
        }
    };

    Sse::new(body).keep_alive(KeepAlive::default())
}
