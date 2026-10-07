mod domain;
mod mcp;
mod services;
mod tools;

use rivet::prelude::*;

#[tokio::main]
async fn main() -> Result<()> {
    mcp::server::serve().await
}
