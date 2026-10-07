use rivet::mcp::McpServer;
use rivet::prelude::*;

use crate::services::directory::Directory;
use crate::tools::search_users::SearchUsers;

/// Register the tool and serve over stdio — the canonical transport for a local
/// MCP server launched by a client process.
pub async fn serve() -> Result<()> {
    let directory = Directory::load();
    McpServer::new()
        .tool(SearchUsers::new(directory))
        .serve_stdio()
        .await
}
