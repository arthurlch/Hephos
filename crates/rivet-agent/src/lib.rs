//! Rivet agent layer.
//!
//! The agent layer is deliberately state-agnostic: it knows about models, tools,
//! messages, and limits — not about your `AppState`. A tool captures whatever it
//! needs (a `Db`, a service, an identity) when it is constructed, so the agent
//! never has to be generic over the application.
//!
//! The pieces:
//!
//! - [`Model`]  — a provider abstraction: complete and stream.
//! - [`Tool`]   — a strongly typed capability the model may call.
//! - [`Agent`]  — a model + system prompt + tools + [`Limits`].
//!
//! Structured output ([`Agent::run_typed`]) deserializes the model's final answer
//! into a typed value whose JSON Schema is derived with `schemars`.

#![forbid(unsafe_code)]

mod agent;
mod message;
mod model;
mod tool;

pub use agent::{Agent, Limits};
pub use message::{Message, Role, TextStream};
pub use model::{Completion, Model};
pub use tool::{Tool, ToolCall, ToolResult, ToolSchema, schema_for as tool_schema};

/// Provider clients.
///
/// BOUNDARY: each provider is a thin `reqwest` client against that vendor's HTTP
/// API. The [`Model`] trait is the stable contract; provider structs implement it.
pub mod providers;
