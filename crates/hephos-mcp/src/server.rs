use std::future::Future;
use std::sync::Arc;

use hephos_agent::{Tool, ToolCall, Tools};
use hephos_core::{Error, Result, Router};
use rmcp::handler::server::ServerHandler;
use rmcp::model::{
    CallToolRequestParam, CallToolResult, Content, ListToolsResult, PaginatedRequestParam,
    ServerInfo,
};
use rmcp::service::{RequestContext, RoleServer};
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::tower::{
    StreamableHttpServerConfig, StreamableHttpService,
};

/// Build an MCP server that exposes Hephos tools.
///
/// Implements rmcp's [`ServerHandler`]. Each registered Hephos [`Tool`] is exposed
/// with its derived `schemars` input schema, with invocations deserializing arguments,
/// executing [`Tool::call`], and serializing results back to MCP responses.
#[derive(Clone, Debug)]
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
    ///
    /// **Security:** this service performs **no authentication** and imposes no body-size
    /// or session bound of its own. Mounting it on a Hephos [`Router`]/`App` applies
    /// the standard stack (request body limit, timeouts, rate limiting, panic isolation),
    /// which bounds request size and abuse. If you serve this service **standalone** (not
    /// behind the Hephos stack), add your own body-limit and timeout layers. For
    /// authenticated access use [`Self::into_protected_router`].
    pub fn into_service(self) -> StreamableHttpService<McpServer, LocalSessionManager> {
        StreamableHttpService::new(
            move || Ok(self.clone()),
            Arc::new(LocalSessionManager::default()),
            StreamableHttpServerConfig::default(),
        )
    }

    /// Mount as a Streamable HTTP endpoint on a Hephos [`Router`].
    ///
    /// **Unauthenticated:** anyone who can reach `prefix` can call every registered tool.
    /// The surrounding Hephos stack still applies body limits, timeouts, and rate limiting,
    /// but there is no identity check. Use [`Self::into_protected_router`] for any endpoint
    /// exposed beyond localhost / a trusted network.
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
