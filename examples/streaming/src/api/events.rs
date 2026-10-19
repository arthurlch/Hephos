use std::convert::Infallible;
use std::time::Duration;

use futures::Stream;
use hephos::prelude::*;
use hephos::sse::{Event, KeepAlive, LastEventId, Sse, event_with_retry};

use crate::state::{AppState, Ctx};

pub fn routes() -> Router<AppState> {
    Router::new().get("/events", stream)
}

/// Emit events up to 10 (the first immediately, then one per interval) and end.
/// Supports resuming from `Last-Event-ID` or `?last_event_id=` / `?last_id=`.
/// On server shutdown the token fires and the stream stops; on client disconnect the runtime
/// drops this stream, which also stops production.
pub async fn stream(
    ctx: Ctx,
    last_id: LastEventId,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let start_seq = last_id.parse::<u32>().map(|id| id + 1).unwrap_or(0);
    let tick_interval = ctx.state().tick_interval;

    let body = async_stream::stream! {
        let mut tick = tokio::time::interval(tick_interval);
        for n in start_seq..10u32 {
            tokio::select! {
                _ = ctx.cancel_token().cancelled() => break,
                _ = tick.tick() => {
                    yield Ok(event_with_retry(
                        n.to_string(),
                        format!("event payload {n}"),
                        Duration::from_secs(15),
                    ));
                }
            }
        }
    };

    Sse::new(body).keep_alive(KeepAlive::default())
}
