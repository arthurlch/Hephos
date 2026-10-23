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

mod client;
mod server;

pub use client::McpClient;
pub use server::McpServer;

pub use hephos_agent::{
    Tool, ToolCall, ToolDefinition, ToolDefinition as RegisteredMcpTool, ToolSchema, Tools,
};

#[cfg(test)]
mod tests {
    use super::*;
    use hephos_agent::Tool;
    use hephos_core::{Error, Result, Router};
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
