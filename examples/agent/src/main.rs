mod agents;
mod domain;
mod model;

use futures::StreamExt;
use rivet::prelude::*;

use crate::domain::report::Report;
use crate::model::ScriptedModel;

#[tokio::main]
async fn main() -> Result<()> {
    let agent = agents::report::build(ScriptedModel::canned());

    let text = agent.run("Summarize Ada's account.").await?;
    println!("text:\n{text}\n");

    let report: Report = agent.run_typed("Summarize Ada's account.").await?;
    println!("structured:\n{report:#?}\n");

    print!("streamed:\n");
    let mut stream = agent.stream("Summarize Ada's account.").await?;
    while let Some(chunk) = stream.next().await {
        print!("{}", chunk?);
    }
    println!();
    Ok(())
}
