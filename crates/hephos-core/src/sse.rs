//! Server-Sent Events (SSE) primitives and streaming maturity.
//!
//! Provides the blessed Server-Sent Events surface:
//! - Re-exports of [`crate::sse::Event`], [`crate::sse::KeepAlive`], and [`crate::sse::Sse`].
//! - Automatic [`crate::sse::LastEventId`] extractor for stream resumption via the standard
//!   `Last-Event-ID` header or query fallback (`?last_event_id=` / `?last_id=`).
//! - Event constructors with IDs and reconnection retry hints ([`crate::sse::event`], [`crate::sse::event_with_id`], [`crate::sse::event_with_retry`]).

use std::convert::Infallible;
use std::str::FromStr;
use std::time::Duration;

use axum::extract::FromRequestParts;
use axum::http::request::Parts;

pub use axum::response::sse::{Event, KeepAlive, Sse};

/// Maximum accepted length of a `Last-Event-ID` string (256 bytes) to prevent memory abuse.
pub const MAX_LAST_EVENT_ID_LEN: usize = 256;

/// Extractor for the standard SSE `Last-Event-ID` header, with query parameter fallback.
///
/// When an SSE client (such as a browser `EventSource`) reconnects after a disconnect,
/// it sends the `Last-Event-ID: <id>` request header.
///
/// For environments or polyfills where custom request headers cannot be set on
/// `EventSource`, `LastEventId` also checks `?last_event_id=` or `?last_id=` in
/// the query string.
///
/// Handlers take `last_id: LastEventId` to resume event streams from the client's last
/// received cursor.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LastEventId(pub Option<String>);

impl LastEventId {
    /// Return the raw string slice of the event ID if present.
    pub fn as_deref(&self) -> Option<&str> {
        self.0.as_deref()
    }

    /// Parse the event ID into a typed identifier (such as `u64`, `u32`, or `uuid::Uuid`).
    pub fn parse<T: FromStr>(&self) -> Option<T> {
        self.0.as_deref().and_then(|s| s.parse::<T>().ok())
    }

    /// True if no event ID was supplied.
    pub fn is_none(&self) -> bool {
        self.0.is_none()
    }

    /// True if an event ID was supplied.
    pub fn is_some(&self) -> bool {
        self.0.is_some()
    }
}

impl std::ops::Deref for LastEventId {
    type Target = Option<String>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<S> FromRequestParts<S> for LastEventId
where
    S: Send + Sync,
{
    type Rejection = Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        // 1. Check `Last-Event-ID` header
        if let Some(val) = parts.headers.get("last-event-id") {
            if let Ok(s) = val.to_str() {
                let trimmed = s.trim();
                if !trimmed.is_empty() && trimmed.len() <= MAX_LAST_EVENT_ID_LEN {
                    return Ok(LastEventId(Some(trimmed.to_string())));
                }
            }
        }

        // 2. Check query string: ?last_event_id=... or ?last_id=...
        if let Some(query) = parts.uri.query() {
            for param in query.split('&') {
                if let Some((k, v)) = param.split_once('=') {
                    if k == "last_event_id" || k == "last_id" {
                        if let Some(decoded) = crate::router::percent_decode(v) {
                            let trimmed = decoded.trim();
                            if !trimmed.is_empty() && trimmed.len() <= MAX_LAST_EVENT_ID_LEN {
                                return Ok(LastEventId(Some(trimmed.to_string())));
                            }
                        }
                    }
                }
            }
        }

        Ok(LastEventId(None))
    }
}

/// Construct an SSE data event with a string payload.
pub fn event(data: impl Into<String>) -> Event {
    Event::default().data(data.into())
}

/// Construct an SSE event with an explicit ID and string payload.
pub fn event_with_id(id: impl Into<String>, data: impl Into<String>) -> Event {
    Event::default().id(id.into()).data(data.into())
}

/// Construct an SSE event with an explicit ID, string payload, and reconnect retry hint.
pub fn event_with_retry(id: impl Into<String>, data: impl Into<String>, retry: Duration) -> Event {
    Event::default()
        .id(id.into())
        .data(data.into())
        .retry(retry)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::Request;

    #[tokio::test]
    async fn extracts_last_event_id_from_header() {
        let req = Request::builder()
            .header("last-event-id", "event-1234")
            .body(axum::body::Body::empty())
            .unwrap();
        let (mut parts, _) = req.into_parts();
        let last_id = LastEventId::from_request_parts(&mut parts, &())
            .await
            .unwrap();
        assert_eq!(last_id.as_deref(), Some("event-1234"));
        assert!(last_id.is_some());
        assert!(!last_id.is_none());
    }

    #[tokio::test]
    async fn extracts_last_event_id_from_query_params() {
        // 1. ?last_event_id=42
        let req1 = Request::builder()
            .uri("/events?last_event_id=42")
            .body(axum::body::Body::empty())
            .unwrap();
        let (mut parts1, _) = req1.into_parts();
        let last_id1 = LastEventId::from_request_parts(&mut parts1, &())
            .await
            .unwrap();
        assert_eq!(last_id1.as_deref(), Some("42"));
        assert_eq!(last_id1.parse::<u64>(), Some(42));

        // 2. ?last_id=event%2099
        let req2 = Request::builder()
            .uri("/events?last_id=event%2099")
            .body(axum::body::Body::empty())
            .unwrap();
        let (mut parts2, _) = req2.into_parts();
        let last_id2 = LastEventId::from_request_parts(&mut parts2, &())
            .await
            .unwrap();
        assert_eq!(last_id2.as_deref(), Some("event 99"));
    }

    #[tokio::test]
    async fn header_takes_precedence_over_query() {
        let req = Request::builder()
            .uri("/events?last_event_id=from-query")
            .header("last-event-id", "from-header")
            .body(axum::body::Body::empty())
            .unwrap();
        let (mut parts, _) = req.into_parts();
        let last_id = LastEventId::from_request_parts(&mut parts, &())
            .await
            .unwrap();
        assert_eq!(last_id.as_deref(), Some("from-header"));
    }

    #[tokio::test]
    async fn rejects_oversized_and_empty_last_event_id() {
        // 1. Empty header
        let req_empty = Request::builder()
            .header("last-event-id", "   ")
            .body(axum::body::Body::empty())
            .unwrap();
        let (mut parts_empty, _) = req_empty.into_parts();
        let last_id_empty = LastEventId::from_request_parts(&mut parts_empty, &())
            .await
            .unwrap();
        assert!(last_id_empty.is_none());

        // 2. Oversized header
        let oversized = "x".repeat(MAX_LAST_EVENT_ID_LEN + 1);
        let req_oversized = Request::builder()
            .header("last-event-id", &oversized)
            .body(axum::body::Body::empty())
            .unwrap();
        let (mut parts_oversized, _) = req_oversized.into_parts();
        let last_id_oversized = LastEventId::from_request_parts(&mut parts_oversized, &())
            .await
            .unwrap();
        assert!(last_id_oversized.is_none());
    }

    #[test]
    fn parse_typed_id() {
        let id_num = LastEventId(Some("1042".into()));
        assert_eq!(id_num.parse::<u64>(), Some(1042));
        assert_eq!(id_num.parse::<i32>(), Some(1042));
        assert_eq!(id_num.parse::<u8>(), None); // overflow

        let id_none = LastEventId(None);
        assert_eq!(id_none.parse::<u64>(), None);
    }

    #[test]
    fn event_constructors() {
        let _e1 = event("hello");
        let _e2 = event_with_id("msg-1", "payload");
        let _e3 = event_with_retry("msg-2", "payload", Duration::from_secs(3));
    }
}
