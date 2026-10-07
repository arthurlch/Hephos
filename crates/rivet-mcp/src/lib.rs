//! Rivet MCP integration.
//!
//! Rivet does not define a protocol. It uses `rmcp`, the official Rust SDK for the
//! Model Context Protocol. This crate's only job is to turn a Rivet [`Tool`] into
//! an MCP tool and run a server over one of rmcp's transports (stdio for local
//! integrations, Streamable HTTP for networked ones).
//!
//! The payoff: a tool written once (strongly typed input/output, validation,
//! authorization) is callable by a Rivet [`rivet_agent::Agent`] *and* exposable to
//! any MCP client, with no second implementation.

#![forbid(unsafe_code)]

use rivet_agent::Tool;
use rivet_core::Result;

/// Build an MCP server that exposes Rivet tools.
///
/// BOUNDARY: this wraps rmcp's `ServerHandler`. Register each Rivet [`Tool`] as an
/// rmcp tool whose input schema is the tool's derived `schemars` schema and whose
/// handler deserializes the arguments, calls [`Tool::call`], and serializes the
/// output. Errors map through [`rivet_core::Error`] to MCP error responses.
pub struct McpServer {
    tools: Vec<RegisteredMcpTool>,
}

impl McpServer {
    pub fn new() -> Self {
        McpServer { tools: Vec::new() }
    }

    pub fn tool<T: Tool>(mut self, tool: T) -> Self {
        self.tools.push(RegisteredMcpTool::new(tool));
        self
    }

    /// Serve over stdio — the canonical transport for a local MCP server launched
    /// by a client process.
    ///
    /// BOUNDARY: call `rmcp::serve_server` with the stdio transport and a handler
    /// backed by `self.tools`.
    pub async fn serve_stdio(self) -> Result<()> {
        let _ = &self.tools;
        // BOUNDARY: `rmcp::transport::io::stdio()` + `ServerHandler`.
        Ok(())
    }

    /// Mount as a Streamable HTTP endpoint on an existing Rivet router.
    ///
    /// BOUNDARY: use rmcp's `transport-streamable-http-server` to serve the single
    /// MCP endpoint (POST + GET) and nest it under the app router. See
    /// ARCHITECTURE.md § MCP.
    pub fn into_route(self) {
        let _ = self.tools;
    }
}

impl Default for McpServer {
    fn default() -> Self {
        Self::new()
    }
}

// BOUNDARY: these fields are consumed when registering the tool with rmcp's
// `ServerHandler`; held here until that wiring lands (see `serve_stdio`).
#[allow(dead_code)]
struct RegisteredMcpTool {
    name: &'static str,
    description: &'static str,
    input_schema: serde_json::Value,
}

impl RegisteredMcpTool {
    fn new<T: Tool>(_tool: T) -> Self {
        let schema = rivet_agent::tool_schema::<T>();
        RegisteredMcpTool {
            name: schema.name,
            description: schema.description,
            input_schema: schema.input_schema,
        }
    }
}
