//! Hephos MCP integration.
//!
//! Hephos does not define a protocol. It uses `rmcp`, the official Rust SDK for the
//! Model Context Protocol. This crate's only job is to turn a Hephos [`Tool`] into
//! an MCP tool and run a server over one of rmcp's transports (stdio for local
//! integrations, Streamable HTTP for networked ones), and to consume external MCP
//! servers as agent tools via [`McpClient`].
//!
//! The payoff: a tool written once (strongly typed input/output, validation,
//! authorization) is callable by a Hephos [`hephos_agent::Agent`] *and* exposable to
//! any MCP client, with no second implementation.

#![forbid(unsafe_code)]

use std::future::Future;
use std::sync::Arc;

pub use hephos_agent::{
    Tool, ToolCall, ToolDefinition, ToolDefinition as RegisteredMcpTool, ToolSchema, Tools,
};
use hephos_core::{Error, Result, Router};
use rmcp::handler::server::ServerHandler;
use rmcp::model::{
    CallToolRequestParam, CallToolResult, Content, ListToolsResult, PaginatedRequestParam,
    ServerInfo,
};
use rmcp::service::{RequestContext, RoleClient, RoleServer, RunningService};
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::tower::{
    StreamableHttpServerConfig, StreamableHttpService,
};
use serde::Serialize;
use serde::de::DeserializeOwned;

/// Build an MCP server that exposes Hephos tools.
///
/// Implements rmcp's [`ServerHandler`]. Each registered Hephos [`Tool`] is exposed
/// with its derived `schemars` input schema, with invocations deserializing arguments,
/// executing [`Tool::call`], and serializing results back to MCP responses.
#[derive(Clone)]
pub struct McpServer {
    name: String,
    version: String,
    tools: Tools,
    caller_identity: hephos_core::Identity,
}

impl McpServer {
    pub fn new() -> Self {
        McpServer {
            name: "hephos-mcp".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            tools: Tools::new(),
            caller_identity: hephos_core::Identity::Anonymous,
        }
    }

    /// Set the caller identity for tools executed via this MCP server.
    ///
    /// By default, tools invoked through MCP receive [`hephos_core::Identity::Anonymous`],
    /// enforcing least privilege and safe boundary defaults.
    pub fn with_caller_identity(mut self, identity: hephos_core::Identity) -> Self {
        self.caller_identity = identity;
        self
    }

    /// Return the caller identity associated with this server.
    pub fn caller_identity(&self) -> &hephos_core::Identity {
        &self.caller_identity
    }

    /// Set custom server info name and version.
    pub fn with_info(mut self, name: impl Into<String>, version: impl Into<String>) -> Self {
        self.name = name.into();
        self.version = version.into();
        self
    }

    /// Register a single Hephos [`Tool`].
    pub fn tool<T: Tool>(mut self, tool: T) -> Self {
        self.tools.register(tool);
        self
    }

    /// Register an entire [`Tools`] collection on this MCP server.
    pub fn tools(mut self, tools: Tools) -> Self {
        self.tools.extend(tools);
        self
    }

    /// Return a reference to the registered tools collection.
    pub fn tools_ref(&self) -> &Tools {
        &self.tools
    }

    /// Serve over an arbitrary async I/O transport (e.g. duplex channels or pipes).
    pub async fn serve_io<I, O>(self, io: (I, O)) -> Result<()>
    where
        I: tokio::io::AsyncRead + Unpin + Send + 'static,
        O: tokio::io::AsyncWrite + Unpin + Send + 'static,
    {
        let running = rmcp::service::serve_server(self, io)
            .await
            .map_err(|e| Error::internal(format!("failed to start I/O MCP server: {e}")))?;
        running
            .waiting()
            .await
            .map_err(|e| Error::internal(format!("MCP I/O server terminated with error: {e}")))?;
        Ok(())
    }

    /// Serve over stdio — the canonical transport for a local MCP server launched
    /// by a client process.
    pub async fn serve_stdio(self) -> Result<()> {
        let (stdin, stdout) = rmcp::transport::io::stdio();
        self.serve_io((stdin, stdout)).await
    }

    /// Convert into a Tower service using rmcp's Streamable HTTP transport.
    pub fn into_service(self) -> StreamableHttpService<McpServer, LocalSessionManager> {
        StreamableHttpService::new(
            move || Ok(self.clone()),
            Arc::new(LocalSessionManager::default()),
            StreamableHttpServerConfig::default(),
        )
    }

    /// Mount as a Streamable HTTP endpoint on a Hephos [`Router`].
    pub fn into_router<S: Clone + Send + Sync + 'static>(self, prefix: &str) -> Router<S> {
        Router::new().nest_service(prefix, self.into_service())
    }

    /// Mount as a protected Streamable HTTP endpoint on a Hephos [`Router`],
    /// enforcing bearer token authentication on all incoming MCP connections.
    pub fn into_protected_router<S, F, Fut>(self, prefix: &str, verify: F) -> Router<S>
    where
        S: Clone + Send + Sync + 'static,
        F: Fn(String) -> Fut + Clone + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<hephos_core::Identity>> + Send + 'static,
    {
        let verify = Arc::new(verify);
        let service = self.into_service();
        let auth_layer = axum::middleware::from_fn(
            move |request: axum::extract::Request, next: axum::middleware::Next| {
                let verify = verify.clone();
                async move {
                    let token = request
                        .headers()
                        .get(axum::http::header::AUTHORIZATION)
                        .and_then(|h| h.to_str().ok())
                        .and_then(|h| {
                            let (scheme, cred) = h.split_once(' ')?;
                            scheme.eq_ignore_ascii_case("bearer").then_some(cred.trim())
                        });

                    let token = match token {
                        Some(t) if !t.is_empty() && t.len() <= 4096 => t.to_string(),
                        _ => {
                            return axum::response::IntoResponse::into_response(
                                Error::Unauthorized,
                            );
                        }
                    };

                    match verify(token).await {
                        Ok(identity) => {
                            let mut req = request;
                            req.extensions_mut().insert(identity);
                            next.run(req).await
                        }
                        Err(err) => axum::response::IntoResponse::into_response(err),
                    }
                }
            },
        );

        let wrapped = tower::ServiceBuilder::new()
            .layer(auth_layer)
            .service(service);

        Router::new().nest_service(prefix, wrapped)
    }
}

impl Default for McpServer {
    fn default() -> Self {
        Self::new()
    }
}

impl ServerHandler for McpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            server_info: rmcp::model::Implementation {
                name: self.name.clone(),
                version: self.version.clone(),
                title: None,
                icons: None,
                website_url: None,
            },
            ..Default::default()
        }
    }

    fn list_tools(
        &self,
        _request: Option<PaginatedRequestParam>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = std::result::Result<ListToolsResult, rmcp::model::ErrorData>> + Send + '_
    {
        let tools: Vec<rmcp::model::Tool> = self
            .tools
            .schemas()
            .into_iter()
            .map(|s| {
                let schema_obj = match serde_json::from_value(s.input_schema) {
                    Ok(obj) => obj,
                    Err(_) => serde_json::Map::new(),
                };
                rmcp::model::Tool::new(s.name, s.description, Arc::new(schema_obj))
            })
            .collect();

        std::future::ready(Ok(ListToolsResult {
            tools,
            next_cursor: None,
        }))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParam,
        _context: RequestContext<RoleServer>,
    ) -> std::result::Result<CallToolResult, rmcp::model::ErrorData> {
        let args = request
            .arguments
            .map(serde_json::Value::Object)
            .unwrap_or(serde_json::json!({}));
        let call = ToolCall::new(request.name.clone(), args);
        match self.tools.dispatch(call).await {
            Ok(result) => {
                let text = match serde_json::to_string_pretty(&result.output) {
                    Ok(s) => s,
                    Err(_) => result.output.to_string(),
                };
                Ok(CallToolResult {
                    content: vec![Content::text(text)],
                    structured_content: Some(result.output),
                    is_error: Some(false),
                    meta: None,
                })
            }
            Err(err) => Ok(CallToolResult::error(vec![Content::text(err.to_string())])),
        }
    }
}

/// An MCP client that connects to an external MCP server to list and call tools.
#[derive(Clone)]
pub struct McpClient {
    service: Arc<RunningService<RoleClient, ()>>,
}

impl McpClient {
    /// Connect to an MCP server over any transport implementing [`rmcp::transport::IntoTransport`].
    pub async fn connect<T, E, A>(transport: T) -> Result<Self>
    where
        T: rmcp::transport::IntoTransport<RoleClient, E, A>,
        E: std::error::Error + Send + Sync + 'static,
    {
        let service = rmcp::service::serve_client((), transport)
            .await
            .map_err(|e| Error::internal(format!("failed to connect MCP client: {e}")))?;
        Ok(McpClient {
            service: Arc::new(service),
        })
    }

    /// List tools provided by the connected MCP server.
    pub async fn list_tools(&self) -> Result<Vec<rmcp::model::Tool>> {
        let res = self
            .service
            .list_tools(None)
            .await
            .map_err(|e| Error::internal(format!("MCP list_tools failed: {e}")))?;
        Ok(res.tools)
    }

    /// Call an MCP tool by name with arbitrary arguments, returning JSON.
    pub async fn call_tool(
        &self,
        name: impl Into<String>,
        arguments: serde_json::Value,
    ) -> Result<serde_json::Value> {
        let args = match arguments {
            serde_json::Value::Object(map) => Some(map),
            serde_json::Value::Null => None,
            other => {
                return Err(Error::invalid(format!(
                    "tool arguments must be a JSON object, got: {other}"
                )));
            }
        };

        let param = CallToolRequestParam {
            name: name.into().into(),
            arguments: args,
        };

        let result = self
            .service
            .call_tool(param)
            .await
            .map_err(|e| Error::internal(format!("MCP call_tool failed: {e}")))?;

        if result.is_error == Some(true) {
            let msg = result
                .content
                .iter()
                .filter_map(|c| c.raw.as_text().map(|t| t.text.as_str()))
                .collect::<Vec<_>>()
                .join("\n");
            return Err(Error::invalid(format!("tool error: {msg}")));
        }

        if let Some(structured) = result.structured_content {
            Ok(structured)
        } else if let Some(t) = result.content.first().and_then(|c| c.raw.as_text()) {
            match serde_json::from_str(&t.text) {
                Ok(val) => Ok(val),
                Err(_) => Ok(serde_json::Value::String(t.text.clone())),
            }
        } else {
            Ok(serde_json::Value::Null)
        }
    }

    /// Call a tool with strongly typed input and output.
    pub async fn call_typed<I: Serialize, O: DeserializeOwned>(
        &self,
        name: &str,
        input: I,
    ) -> Result<O> {
        let args = serde_json::to_value(input)
            .map_err(|e| Error::invalid(format!("failed to serialize input: {e}")))?;
        let output = self.call_tool(name, args).await?;
        serde_json::from_value(output)
            .map_err(|e| Error::internal(format!("failed to deserialize output: {e}")))
    }

    /// Convert all tools provided by this MCP server into a [`hephos_agent::Tools`]
    /// collection, allowing an [`hephos_agent::Agent`] to consume them directly.
    pub async fn to_tools(&self) -> Result<Tools> {
        let remote_tools = self.list_tools().await?;
        let mut tools = Tools::new();
        for tool in remote_tools {
            let name = tool.name.to_string();
            let description = tool.description.unwrap_or_default().to_string();
            let input_schema = serde_json::Value::Object((*tool.input_schema).clone());
            let schema = ToolSchema::new(name.clone(), description, input_schema);

            let client = self.clone();
            let tool_name = name.clone();
            let def = ToolDefinition::dynamic(schema, move |args| {
                let client = client.clone();
                let tool_name = tool_name.clone();
                Box::pin(async move { client.call_tool(&tool_name, args).await })
            });
            tools.register_definition(def);
        }
        Ok(tools)
    }

    /// Convert a single named tool from this MCP server into an agent [`RegisteredMcpTool`].
    pub async fn to_tool(&self, name: &str) -> Result<RegisteredMcpTool> {
        let remote_tools = self.list_tools().await?;
        let tool = remote_tools
            .into_iter()
            .find(|t| t.name == name)
            .ok_or_else(|| Error::not_found(format!("MCP tool '{name}'")))?;

        let tool_name = tool.name.to_string();
        let description = tool.description.unwrap_or_default().to_string();
        let input_schema = serde_json::Value::Object((*tool.input_schema).clone());
        let schema = ToolSchema::new(tool_name.clone(), description, input_schema);

        let client = self.clone();
        let def = ToolDefinition::dynamic(schema, move |args| {
            let client = client.clone();
            let tool_name = tool_name.clone();
            Box::pin(async move { client.call_tool(&tool_name, args).await })
        });
        Ok(def)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hephos_agent::Tool;
    use hephos_core::Result;
    use schemars::JsonSchema;
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
    struct Input {
        query: String,
    }

    #[derive(Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
    struct Output {
        result: String,
    }

    struct Search;

    impl Tool for Search {
        type Input = Input;
        type Output = Output;
        const NAME: &'static str = "search";
        const DESCRIPTION: &'static str = "search things";
        async fn call(&self, input: Input) -> Result<Output> {
            if input.query.is_empty() {
                return Err(Error::invalid("query must not be empty"));
            }
            Ok(Output {
                result: format!("found: {}", input.query),
            })
        }
    }

    #[test]
    fn registered_mcp_tool_matches_agent_schema() {
        let server = McpServer::new().tool(Search);
        assert_eq!(server.tools_ref().len(), 1);

        let registered = server.tools_ref().get("search").unwrap();
        let agent_schema = hephos_agent::tool_schema::<Search>();
        assert_eq!(registered.schema().name, agent_schema.name);
        assert_eq!(registered.schema().description, agent_schema.description);
        assert_eq!(registered.schema().input_schema, agent_schema.input_schema);
        assert!(registered.schema().input_schema.get("properties").is_some());
    }

    #[tokio::test]
    async fn registered_tool_invoke_succeeds_and_fails_on_empty() {
        let tool = RegisteredMcpTool::new(Search);
        let res = tool
            .call_raw(serde_json::json!({ "query": "rust" }))
            .await
            .unwrap();
        assert_eq!(res["result"], "found: rust");

        let err = tool.call_raw(serde_json::json!({ "query": "" })).await;
        assert!(err.is_err());
    }

    #[tokio::test]
    async fn server_client_duplex_end_to_end() {
        let (server_io, client_io) = tokio::io::duplex(4096);
        let server = McpServer::new().tool(Search);

        tokio::spawn(async move {
            if let Ok(running) = rmcp::service::serve_server(server, server_io).await {
                let _ = running.waiting().await;
            }
        });

        let client = McpClient::connect(client_io).await.unwrap();

        // 1. List tools
        let tools = client.list_tools().await.unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "search");

        // 2. Call tool successfully
        let out: Output = client
            .call_typed(
                "search",
                Input {
                    query: "hello".into(),
                },
            )
            .await
            .unwrap();
        assert_eq!(
            out,
            Output {
                result: "found: hello".into()
            }
        );

        // 3. Call tool failure (empty query)
        let err = client
            .call_typed::<_, Output>("search", Input { query: "".into() })
            .await;
        assert!(err.is_err());

        // 4. Consume MCP server tools directly as an Agent Tools collection
        let agent_tools = client.to_tools().await.unwrap();
        assert_eq!(agent_tools.len(), 1);
        let call = ToolCall::new("search", serde_json::json!({ "query": "hephos agent" }));
        let tool_res = agent_tools.dispatch(call).await.unwrap();
        assert_eq!(tool_res.output["result"], "found: hephos agent");
    }

    #[test]
    fn shared_tools_collection_between_server_and_agent() {
        let tools = Tools::new().tool(Search);
        assert_eq!(tools.len(), 1);

        let server = McpServer::new().tools(tools.clone());
        assert_eq!(server.tools_ref().len(), 1);

        let def = tools.get("search").unwrap();
        assert_eq!(def.schema().name, "search");
    }

    #[test]
    fn into_router_mounts_streamable_http() {
        let server = McpServer::new().tool(Search);
        let router: Router<()> = server.into_router("/mcp");
        let entries = router.entries();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].method, "ANY");
        assert_eq!(entries[0].path, "/mcp/*");
    }

    #[test]
    fn caller_identity_defaults_to_anonymous_and_supports_custom() {
        let server = McpServer::new();
        assert_eq!(*server.caller_identity(), hephos_core::Identity::Anonymous);

        let custom_id = uuid::Uuid::new_v4();
        let server =
            server.with_caller_identity(hephos_core::Identity::User(hephos_core::Principal {
                id: custom_id,
                roles: vec!["operator".into()],
            }));
        if let hephos_core::Identity::User(principal) = server.caller_identity() {
            assert_eq!(principal.id, custom_id);
            assert!(principal.has_role("operator"));
        } else {
            panic!("expected User identity");
        }
    }

    #[tokio::test]
    async fn into_protected_router_mounts_auth_guarded_endpoint() {
        let server = McpServer::new().tool(Search);
        let router: Router<()> = server.into_protected_router("/mcp", |token| async move {
            if token == "secret-token" {
                Ok(hephos_core::Identity::User(hephos_core::Principal {
                    id: uuid::Uuid::new_v4(),
                    roles: vec!["agent".into()],
                }))
            } else {
                Err(hephos_core::Error::Unauthorized)
            }
        });

        let entries = router.entries();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].method, "ANY");
        assert_eq!(entries[0].path, "/mcp/*");
    }
}
