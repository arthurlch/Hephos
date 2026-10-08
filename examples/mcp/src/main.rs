mod domain;
mod mcp;
mod services;
mod tools;

use hephos::prelude::*;

#[tokio::main]
async fn main() -> Result<()> {
    mcp::server::serve().await
}
