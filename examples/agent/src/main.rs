mod agents;
mod domain;

use futures::StreamExt;
use hephos::prelude::*;

use crate::agents::ScriptedModel;
use crate::domain::report::Report;

#[tokio::main]
async fn main() -> Result<()> {
    let agent = agents::report::build(ScriptedModel::canned());

    let text = agent.run("Summarize Ada's account.").await?;
    println!("text:\n{text}\n");

    let report: Report = agent.run_typed("Summarize Ada's account.").await?;
    println!("structured: {}", report.headline);
    for point in &report.bullet_points {
        println!("  - {point}");
    }
    println!();

    println!("streamed:");
    let mut stream = agent.stream("Summarize Ada's account.").await?;
    while let Some(chunk) = stream.next().await {
        print!("{}", chunk?);
    }
    println!();
    Ok(())
}
