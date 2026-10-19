use std::sync::Mutex;

use hephos::agent::{Agent, Completion, Message, Model, TextStream, ToolCall};
use hephos::prelude::*;

use crate::domain::user::{SearchQuery, UserSummary};
use crate::mcp::server;
use crate::services::directory::Directory;
use crate::tools::search_users::SearchUsers;

/// A scripted deterministic model that yields messages from a FIFO queue.
struct SequenceModel {
    responses: Mutex<Vec<Message>>,
}

impl SequenceModel {
    fn new(responses: Vec<Message>) -> Self {
        SequenceModel {
            responses: Mutex::new(responses),
        }
    }
}

impl Model for SequenceModel {
    async fn complete(&self, _request: Completion) -> Result<Message> {
        let mut guard = self.responses.lock().unwrap();
        if guard.is_empty() {
            Ok(Message::assistant("done"))
        } else {
            Ok(guard.remove(0))
        }
    }

    async fn stream(&self, _request: Completion) -> Result<TextStream> {
        Ok(Box::pin(futures::stream::empty()))
    }
}

#[test]
fn routes_build() {
    // AGENTS.md canary test: asserts router builds without panic
    let _router = server::router();
}

#[test]
fn openapi_generation_validates() {
    let router = server::router();
    let doc = router.openapi("User Search MCP Server", "1.0.0");

    assert_eq!(doc["openapi"], "3.1.0");
    assert_eq!(doc["info"]["title"], "User Search MCP Server");
    assert_eq!(doc["info"]["version"], "1.0.0");
    assert!(doc["paths"]["/mcp/*"].is_object());
}

#[tokio::test]
async fn in_process_agent_calls_same_tool() {
    // Proves "the same tool is callable by an in-process agent with no second implementation"
    let model = SequenceModel::new(vec![
        Message::assistant_with_tools(
            "",
            vec![ToolCall::with_id(
                "call_1",
                "search_users",
                serde_json::json!({ "query": "ada" }),
            )],
        ),
        Message::assistant("Found Ada Lovelace: ada@example.com"),
    ]);

    let directory = Directory::load();
    let agent = Agent::new(model)
        .system("You help users find colleagues.")
        .tool(SearchUsers::new(directory));

    let reply = agent.run("Find Ada").await.unwrap();
    assert_eq!(reply, "Found Ada Lovelace: ada@example.com");
}

#[tokio::test]
async fn mcp_client_drives_server_over_duplex() {
    // Proves MCP server serves tool over duplex transport and is driven by an MCP client
    let (server_io, client_io) = tokio::io::duplex(4096);
    let mcp_srv = server::server();

    tokio::spawn(async move {
        let _ = mcp_srv.serve_io(tokio::io::split(server_io)).await;
    });

    let client = McpClient::connect(client_io).await.unwrap();

    // 1. List tools
    let tools = client.list_tools().await.unwrap();
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].name, "search_users");
    assert_eq!(
        tools[0].description.as_deref(),
        Some("Search the user directory by name or email.")
    );

    // 2. Call tool successfully
    let results: Vec<UserSummary> = client
        .call_typed(
            "search_users",
            SearchQuery {
                query: "alan".into(),
            },
        )
        .await
        .unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].name, "Alan Turing");

    // 3. Call tool failure (empty query)
    let err = client
        .call_typed::<_, Vec<UserSummary>>("search_users", SearchQuery { query: "".into() })
        .await;
    assert!(err.is_err());
}

#[tokio::test]
async fn mcp_client_tools_consumed_by_agent() {
    // Proves external MCP server tools can be converted to an Agent Tools collection
    let (server_io, client_io) = tokio::io::duplex(4096);
    let mcp_srv = server::server();

    tokio::spawn(async move {
        let _ = mcp_srv.serve_io(tokio::io::split(server_io)).await;
    });

    let client = McpClient::connect(client_io).await.unwrap();
    let remote_tools = client.to_tools().await.unwrap();

    let model = SequenceModel::new(vec![
        Message::assistant_with_tools(
            "",
            vec![ToolCall::with_id(
                "call_mcp_1",
                "search_users",
                serde_json::json!({ "query": "grace" }),
            )],
        ),
        Message::assistant("Found Grace Hopper"),
    ]);

    let agent = Agent::new(model)
        .system("MCP agent assistant")
        .tools(remote_tools);

    let reply = agent.run("Who is Grace?").await.unwrap();
    assert_eq!(reply, "Found Grace Hopper");
}
