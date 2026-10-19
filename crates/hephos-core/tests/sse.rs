//! Server-Sent Events integration tests.

use std::convert::Infallible;
use std::time::Duration;

use futures::stream;
use hephos_core::sse::{Event, KeepAlive, LastEventId, Sse, event, event_with_retry};
use hephos_core::test::TestClient;
use hephos_core::{Ctx, Json, Result, Router};

#[derive(Clone)]
struct S;

async fn sse_endpoint(
    _ctx: Ctx<S>,
    last_id: LastEventId,
) -> Sse<impl futures::Stream<Item = std::result::Result<Event, Infallible>>> {
    let id_str = last_id.as_deref().unwrap_or("none");
    let stream = stream::iter(vec![
        Ok(event("msg1")),
        Ok(event_with_retry(
            "evt-2",
            format!("msg2-from-{id_str}"),
            Duration::from_millis(500),
        )),
    ]);
    Sse::new(stream).keep_alive(KeepAlive::default())
}

async fn parse_id_endpoint(_ctx: Ctx<S>, last_id: LastEventId) -> Result<Json<Option<u64>>> {
    Ok(Json(last_id.parse::<u64>()))
}

fn routes() -> Router<S> {
    Router::new()
        .get("/events", sse_endpoint)
        .get("/last-id", parse_id_endpoint)
}

#[tokio::test]
async fn sse_response_headers_and_status() {
    let client = TestClient::new(routes(), S);
    let res = client.get("/events").await;
    assert_eq!(res.status(), 200);
    assert_eq!(
        res.header("content-type").as_deref(),
        Some("text/event-stream")
    );
}

#[tokio::test]
async fn last_event_id_extractor_query_and_header() {
    let client = TestClient::new(routes(), S);

    // No last ID
    let res = client.get("/last-id").await;
    assert_eq!(res.status(), 200);
    assert_eq!(res.json::<Option<u64>>(), None);

    // Query parameter ?last_event_id=42
    let res = client.get("/last-id?last_event_id=42").await;
    assert_eq!(res.status(), 200);
    assert_eq!(res.json::<Option<u64>>(), Some(42));

    // Query parameter ?last_id=99
    let res = client.get("/last-id?last_id=99").await;
    assert_eq!(res.status(), 200);
    assert_eq!(res.json::<Option<u64>>(), Some(99));
}
