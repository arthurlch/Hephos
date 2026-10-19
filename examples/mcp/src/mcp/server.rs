use hephos::mcp::McpServer;
use hephos::prelude::*;

use crate::services::directory::Directory;
use crate::tools::search_users::SearchUsers;

/// Construct the configured MCP server exposing the user search tool.
pub fn server() -> McpServer {
    let directory = Directory::load();
    McpServer::new().tool(SearchUsers::new(directory))
}

/// Register the tool and serve over stdio — the canonical transport for a local
/// MCP server launched by a client process.
pub async fn serve() -> Result<()> {
    server().serve_stdio().await
}

/// Mount the MCP server as a Streamable HTTP endpoint on a Hephos [`Router`].
pub fn router() -> Router<()> {
    server().into_router("/mcp")
}
