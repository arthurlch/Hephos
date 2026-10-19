mod domain;
mod mcp;
mod services;
mod tools;

#[cfg(test)]
mod tests;

use hephos::prelude::*;

#[tokio::main]
async fn main() -> Result<()> {
    if std::env::var("HEPHOS_MCP_HTTP").is_ok() {
        let app = App::new(()).routes(mcp::server::router());
        app.run().await
    } else {
        mcp::server::serve().await
    }
}
