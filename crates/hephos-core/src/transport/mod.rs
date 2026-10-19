//! Real-time, streaming, and connection-lifecycle transports.
//!
//! This module houses the blessed transport primitives for long-lived and streaming connections:
//! - [`ws`]: WebSocket primitives, configuration, heartbeats, and backpressure.
//! - [`sse`]: Server-Sent Events primitives, `Last-Event-ID` resumption, and retry hints.
//! - [`conn`]: Connection tracking, capacity bounds, and graceful socket draining.

pub mod conn;
pub mod sse;
pub mod ws;
