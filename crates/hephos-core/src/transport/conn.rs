//! Connection tracking, concurrency caps, and graceful socket draining.
//!
//! Provides the blessed primitives for bounding concurrent connections and ensuring
//! long-lived sockets (WebSockets, Server-Sent Events, or long polls) drain cleanly on shutdown:
//! - [`crate::conn::ConnectionCap`]: Tracks active connections, enforces global or per-route capacity limits,
//!   and coordinates graceful draining.
//! - [`crate::conn::ConnectionGuard`]: RAII permit representing one active connection. Automatically decrements
//!   the active count on drop and unblocks drain listeners.
//! - [`crate::conn::ConnectionCapLayer`]: Tower/Axum middleware layer for per-route or global connection capping.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::{Context, Poll};
use std::time::Duration;

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::response::{IntoResponse, Response};
use tokio::sync::watch;

use crate::error::{Error, Result};

/// A tracker and concurrency cap for concurrent connections.
///
/// Thread-safe and cheap to clone (`Arc`-backed). Tracks the count of active
/// connections, rejects new connections when the configured capacity is reached,
/// and provides bounded graceful draining on shutdown via [`ConnectionCap::drain`].
#[derive(Debug, Clone)]
pub struct ConnectionCap {
    inner: Arc<ConnectionCapInner>,
}

#[derive(Debug)]
struct ConnectionCapInner {
    limit: Option<usize>,
    active: AtomicUsize,
    drain_tx: watch::Sender<usize>,
    drain_rx: watch::Receiver<usize>,
}

impl ConnectionCap {
    /// Create a new connection cap with a fixed maximum capacity.
    ///
    /// When `limit` active connections are reached, subsequent calls to [`.acquire()`]
    /// return `503 Service Unavailable` (`Error::Unavailable`).
    pub fn new(limit: usize) -> Self {
        let (drain_tx, drain_rx) = watch::channel(0);
        Self {
            inner: Arc::new(ConnectionCapInner {
                limit: Some(limit),
                active: AtomicUsize::new(0),
                drain_tx,
                drain_rx,
            }),
        }
    }

    /// Create an unlimited connection cap.
    ///
    /// Does not reject connections, but tracks active connections and enables
    /// bounded graceful draining on shutdown.
    pub fn unlimited() -> Self {
        let (drain_tx, drain_rx) = watch::channel(0);
        Self {
            inner: Arc::new(ConnectionCapInner {
                limit: None,
                active: AtomicUsize::new(0),
                drain_tx,
                drain_rx,
            }),
        }
    }

    /// Maximum concurrent connections allowed by this cap, or `None` if unlimited.
    pub fn limit(&self) -> Option<usize> {
        self.inner.limit
    }

    /// Current count of active connections.
    pub fn active(&self) -> usize {
        self.inner.active.load(Ordering::Acquire)
    }

    /// Acquire a connection permit.
    ///
    /// If capacity is exhausted, returns `Err(Error::Unavailable)` which maps to
    /// `503 Service Unavailable`. On success, returns an RAII [`ConnectionGuard`]
    /// that decrements the count when dropped.
    pub fn acquire(&self) -> Result<ConnectionGuard> {
        let mut current = self.inner.active.load(Ordering::Acquire);
        loop {
            if let Some(limit) = self.inner.limit {
                if current >= limit {
                    return Err(Error::unavailable("connection cap reached"));
                }
            }

            match self.inner.active.compare_exchange_weak(
                current,
                current + 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    let next = current + 1;
                    let _ = self.inner.drain_tx.send_replace(next);
                    let permit = ConnectionPermit {
                        cap: Arc::clone(&self.inner),
                    };
                    return Ok(ConnectionGuard {
                        inner: Some(Arc::new(permit)),
                    });
                }
                Err(actual) => current = actual,
            }
        }
    }

    /// Attempt to acquire a connection permit without returning an error on saturation.
    pub fn try_acquire(&self) -> Option<ConnectionGuard> {
        self.acquire().ok()
    }

    /// Wait until all active connections are closed, or until `timeout` expires.
    ///
    /// If all connections close within the timeout, returns `Ok(())`. If the timeout
    /// expires while connections remain active, returns `Err(Error::Unavailable)`.
    pub async fn drain(&self, timeout: Duration) -> Result<()> {
        let mut rx = self.inner.drain_rx.clone();
        if *rx.borrow_and_update() == 0 {
            return Ok(());
        }

        tokio::select! {
            _ = async {
                while *rx.borrow_and_update() > 0 {
                    if rx.changed().await.is_err() {
                        break;
                    }
                }
            } => Ok(()),
            _ = tokio::time::sleep(timeout) => {
                let remaining = self.active();
                if remaining == 0 {
                    Ok(())
                } else {
                    Err(Error::unavailable(format!(
                        "connection drain deadline exceeded ({remaining} connections remaining)"
                    )))
                }
            }
        }
    }

    /// Construct a Tower layer that applies this connection cap to incoming requests.
    pub fn layer(&self) -> ConnectionCapLayer {
        ConnectionCapLayer::new(self.clone())
    }
}

/// An RAII permit representing one active connection.
///
/// Can be cloned; all clones share the same underlying permit. When all clones
/// of a guard are dropped, the active connection count in [`ConnectionCap`] is
/// decremented and any waiters blocked on [`ConnectionCap::drain`] are notified.
#[derive(Debug, Clone)]
pub struct ConnectionGuard {
    inner: Option<Arc<ConnectionPermit>>,
}

#[derive(Debug)]
struct ConnectionPermit {
    cap: Arc<ConnectionCapInner>,
}

impl Drop for ConnectionPermit {
    fn drop(&mut self) {
        let prev = self.cap.active.fetch_sub(1, Ordering::AcqRel);
        let next = prev.saturating_sub(1);
        let _ = self.cap.drain_tx.send_replace(next);
    }
}

impl ConnectionGuard {
    /// Create a detached guard that is not bound to any cap.
    ///
    /// Does not increment or decrement connection counts; useful for tests or
    /// routes that operate without active connection limits.
    pub fn detached() -> Self {
        Self { inner: None }
    }

    /// Consume the guard without decrementing the connection count.
    pub fn forget(mut self) {
        self.inner = None;
    }
}

impl<S> FromRequestParts<S> for ConnectionGuard
where
    S: Send + Sync,
{
    type Rejection = Error;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        // 1. If a guard was already acquired by an outer layer, share ownership via clone:
        if let Some(guard) = parts.extensions.get::<ConnectionGuard>() {
            return Ok(guard.clone());
        }

        // 2. Otherwise, if a ConnectionCap is present in extensions, acquire from it:
        if let Some(cap) = parts.extensions.get::<ConnectionCap>() {
            return cap.acquire();
        }

        // 3. Fallback: detached no-op guard
        Ok(ConnectionGuard::detached())
    }
}

/// Tower/Axum middleware layer that enforces a [`ConnectionCap`].
#[derive(Debug, Clone)]
pub struct ConnectionCapLayer {
    cap: ConnectionCap,
}

impl ConnectionCapLayer {
    pub fn new(cap: ConnectionCap) -> Self {
        Self { cap }
    }
}

impl<S> tower::Layer<S> for ConnectionCapLayer {
    type Service = ConnectionCapService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        ConnectionCapService {
            inner,
            cap: self.cap.clone(),
        }
    }
}

/// Service generated by [`ConnectionCapLayer`].
#[derive(Debug, Clone)]
pub struct ConnectionCapService<S> {
    inner: S,
    cap: ConnectionCap,
}

impl<S, B> tower::Service<axum::http::Request<B>> for ConnectionCapService<S>
where
    S: tower::Service<axum::http::Request<B>, Response = Response> + Clone + Send + 'static,
    S::Future: Send + 'static,
    B: Send + 'static,
{
    type Response = Response;
    type Error = S::Error;
    type Future = std::pin::Pin<
        Box<
            dyn std::future::Future<Output = std::result::Result<Self::Response, Self::Error>>
                + Send,
        >,
    >;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<std::result::Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, mut req: axum::http::Request<B>) -> Self::Future {
        let cap = self.cap.clone();
        let mut inner = self.inner.clone();
        std::mem::swap(&mut self.inner, &mut inner);

        Box::pin(async move {
            match cap.acquire() {
                Ok(guard) => {
                    req.extensions_mut().insert(guard.clone());
                    req.extensions_mut().insert(cap);
                    let mut res = inner.call(req).await?;
                    res.extensions_mut().insert(guard);
                    Ok(res)
                }
                Err(err) => Ok(err.into_response()),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::response::IntoResponse;
    use axum::routing::get;
    use tower::ServiceExt;

    #[tokio::test]
    async fn acquire_and_release_decrements_count() {
        let cap = ConnectionCap::new(2);
        assert_eq!(cap.active(), 0);
        assert_eq!(cap.limit(), Some(2));

        let guard1 = cap.acquire().unwrap();
        assert_eq!(cap.active(), 1);

        let guard2 = cap.acquire().unwrap();
        assert_eq!(cap.active(), 2);

        // 3rd attempt exceeds limit:
        let err = cap.acquire().unwrap_err();
        assert_eq!(
            err.into_response().status(),
            StatusCode::SERVICE_UNAVAILABLE
        );

        drop(guard1);
        assert_eq!(cap.active(), 1);

        let guard3 = cap.acquire().unwrap();
        assert_eq!(cap.active(), 2);

        drop(guard2);
        drop(guard3);
        assert_eq!(cap.active(), 0);
    }

    #[tokio::test]
    async fn drain_succeeds_when_connections_close() {
        let cap = ConnectionCap::new(5);
        let guard = cap.acquire().unwrap();

        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(10)).await;
            drop(guard);
        });

        let drain_res = cap.drain(Duration::from_millis(500)).await;
        assert!(drain_res.is_ok());
        assert_eq!(cap.active(), 0);
    }

    #[tokio::test]
    async fn drain_times_out_when_connection_held() {
        let cap = ConnectionCap::new(5);
        let _guard = cap.acquire().unwrap();

        let drain_res = cap.drain(Duration::from_millis(20)).await;
        assert!(drain_res.is_err());
        assert_eq!(cap.active(), 1);
    }

    #[tokio::test]
    async fn unlimited_cap_tracks_without_rejection() {
        let cap = ConnectionCap::unlimited();
        assert_eq!(cap.limit(), None);

        let mut guards = Vec::new();
        for _ in 0..100 {
            guards.push(cap.acquire().unwrap());
        }
        assert_eq!(cap.active(), 100);

        drop(guards);
        assert_eq!(cap.active(), 0);
    }

    #[tokio::test]
    async fn extractor_from_request_parts() {
        let cap = ConnectionCap::new(1);
        let req = Request::builder()
            .extension(cap.clone())
            .body(Body::empty())
            .unwrap();
        let (mut parts, _) = req.into_parts();

        let guard = ConnectionGuard::from_request_parts(&mut parts, &())
            .await
            .unwrap();
        assert_eq!(cap.active(), 1);

        // Second extraction from parts with same cap fails
        let mut parts2 = parts;
        parts2.extensions.insert(cap.clone());
        let err = ConnectionGuard::from_request_parts(&mut parts2, &())
            .await
            .unwrap_err();
        assert_eq!(
            err.into_response().status(),
            StatusCode::SERVICE_UNAVAILABLE
        );

        drop(guard);
        assert_eq!(cap.active(), 0);
    }

    #[tokio::test]
    async fn layer_enforces_cap_on_router() {
        let cap = ConnectionCap::new(1);
        let router = axum::Router::new()
            .route(
                "/test",
                get(|guard: ConnectionGuard| async move {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                    drop(guard);
                    "ok".into_response()
                }),
            )
            .layer(cap.layer());

        let req1 = Request::builder().uri("/test").body(Body::empty()).unwrap();
        let req2 = Request::builder().uri("/test").body(Body::empty()).unwrap();

        let router1 = router.clone();
        let task1 = tokio::spawn(async move { router1.oneshot(req1).await.unwrap() });

        tokio::time::sleep(Duration::from_millis(5)).await;

        let router2 = router.clone();
        let res2 = router2.oneshot(req2).await.unwrap();
        // Since capacity is 1 and req1 is running, req2 is immediately rejected with 503:
        assert_eq!(res2.status(), StatusCode::SERVICE_UNAVAILABLE);

        let res1 = task1.await.unwrap();
        assert_eq!(res1.status(), StatusCode::OK);
        drop(res1);
        assert_eq!(cap.active(), 0);
    }
}
