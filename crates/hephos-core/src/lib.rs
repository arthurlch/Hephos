//! Hephos core.
//!
//! This crate defines the stable application surface every Hephos app is built
//! from: [`App`], [`Ctx`], [`Error`], [`Router`], the extractors, [`Config`],
//! [`Task`], and the event bus.
//!
//! There is exactly one canonical way to use each of these. If you find
//! yourself reaching for a second way, the convention is wrong, not the code.

#![forbid(unsafe_code)]

mod app;
mod config;
mod ctx;
mod error;
mod event;
mod extract;
mod identity;
mod router;
mod stack;
mod task;
pub mod telemetry;
pub mod test;

pub use app::{App, ProbeResponse};
pub use config::{Config, LogFormat, RateLimitConfig};
pub use ctx::Ctx;
pub use error::{Error, Result};
pub use event::{Event, Events};
pub use extract::{Json, Path, Query};
pub use identity::{Identity, Principal};
pub use router::Router;
pub use task::Task;
pub use telemetry::RedMetrics;

/// Re-export so handlers can return a bare status (the canonical `204` shape:
/// `Result<StatusCode>`) without importing `axum` directly.
pub use axum::http::StatusCode;

/// Re-export the cancellation primitive so apps never pull it in directly.
pub use tokio_util::sync::CancellationToken;

/// Real-time, streaming, and connection-lifecycle transports.
pub mod transport;

/// The blessed WebSocket primitives. Handlers use these; there is no other
/// WebSocket surface.
pub use transport::ws;

/// The blessed server-sent-events / streaming primitives.
pub use transport::sse;

pub use conn::{ConnectionCap, ConnectionGuard};
/// The blessed connection tracking, concurrency caps, and graceful draining primitives.
pub use transport::conn;

/// Lower-level HTTP primitives for the handler escape hatches (raw bodies, request
/// headers, custom responses) — so an app reaches for these, not `axum` directly. See
/// `CONVENTIONS.md §2.13`. The canonical `Json`/`Path`/`Query` surface stays in the
/// prelude; these are the deliberate 20%.
pub mod http {
    pub use axum::body::{Body, Bytes};
    pub use axum::http::{HeaderMap, HeaderName, HeaderValue, Request};
    pub use axum::response::{IntoResponse, Response};
}
