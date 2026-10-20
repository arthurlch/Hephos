use hephos::mcp::McpServer;
use hephos::prelude::*;

use crate::tools::CalculatorTool;

pub fn server() -> McpServer {
    McpServer::new().tool(CalculatorTool)
}

pub fn router<S: Clone + Send + Sync + 'static>() -> Router<S> {
    server().into_router("/mcp")
}
