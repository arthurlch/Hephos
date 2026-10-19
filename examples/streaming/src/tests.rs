use hephos::http::{Body, Request};
use hephos::test::TestClient;

use crate::api;
use crate::state::AppState;

#[tokio::test]
async fn routes_build() {
    let _ = api::routes();
}

#[tokio::test]
async fn stream_emits_events_from_start() {
    let client = TestClient::new(api::routes(), AppState::for_test());
    let res = client.get("/events").await;
    assert_eq!(res.status(), 200);
    assert_eq!(res.header("content-type").unwrap(), "text/event-stream");

    let text = res.text();
    assert!(text.contains("id: 0"));
    assert!(text.contains("data: event payload 0"));
    assert!(text.contains("retry: 15000"));
    assert!(text.contains("id: 9"));
    assert!(text.contains("data: event payload 9"));
}

#[tokio::test]
async fn stream_resumes_from_last_event_id_header() {
    let client = TestClient::new(api::routes(), AppState::for_test());
    let req = Request::builder()
        .uri("/events")
        .header("last-event-id", "6")
        .body(Body::empty())
        .unwrap();

    let res = client.send(req).await;
    assert_eq!(res.status(), 200);

    let text = res.text();
    // Events 0..6 must not be emitted
    assert!(!text.contains("id: 0"));
    assert!(!text.contains("id: 6"));
    // Events 7..9 must be emitted
    assert!(text.contains("id: 7"));
    assert!(text.contains("data: event payload 7"));
    assert!(text.contains("id: 9"));
    assert!(text.contains("data: event payload 9"));
}

#[tokio::test]
async fn stream_resumes_from_query_token_fallback() {
    let client = TestClient::new(api::routes(), AppState::for_test());
    let res = client.get("/events?last_event_id=8").await;
    assert_eq!(res.status(), 200);

    let text = res.text();
    assert!(!text.contains("id: 8"));
    assert!(text.contains("id: 9"));
    assert!(text.contains("data: event payload 9"));
}

#[tokio::test]
async fn stream_with_all_events_acknowledged_is_empty() {
    let client = TestClient::new(api::routes(), AppState::for_test());
    let res = client.get("/events?last_event_id=9").await;
    assert_eq!(res.status(), 200);

    let text = res.text();
    assert!(!text.contains("id:"));
}
