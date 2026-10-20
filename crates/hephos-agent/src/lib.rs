//! Hephos agent layer.
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

pub use agent::{Agent, AgentCheckpoint, InMemoryAgentCheckpoint, Limits, parse_tool_calls};
pub use message::{Message, Role, TextStream};
pub use model::{Completion, Model};
pub use providers::{Anthropic, OpenAI};
pub use schemars::JsonSchema;
pub use tool::{
    Tool, ToolCall, ToolDefinition, ToolRegistry, ToolResult, ToolSchema, Tools,
    schema_for as tool_schema,
};

/// Provider clients.
pub mod providers;
