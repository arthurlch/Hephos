//! Rivet core.
//!
//! This crate defines the stable application surface every Rivet app is built
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
pub mod test;

pub use app::App;
pub use config::{Config, LogFormat};
pub use ctx::Ctx;
pub use error::{Error, Result};
pub use event::{Event, Events};
pub use extract::{Json, Path, Query};
pub use identity::{Identity, Principal};
pub use router::Router;
pub use task::Task;

/// Re-export the cancellation primitive so apps never pull it in directly.
pub use tokio_util::sync::CancellationToken;

/// The blessed WebSocket primitives. Handlers use these; there is no other
/// WebSocket surface.
pub mod ws {
    pub use axum::extract::ws::{Message, Utf8Bytes, WebSocket, WebSocketUpgrade};
    pub use axum::response::Response;
}

/// The blessed server-sent-events / streaming primitives.
pub mod sse {
    pub use axum::response::sse::{Event, KeepAlive, Sse};
}
