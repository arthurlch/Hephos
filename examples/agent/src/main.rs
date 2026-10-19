mod agents;
mod domain;

use futures::StreamExt;
use hephos::prelude::*;

use crate::agents::ScriptedModel;
use crate::domain::report::Report;

#[tokio::main]
async fn main() -> Result<()> {
    if let Ok(key) = std::env::var("ANTHROPIC_API_KEY") {
        let model =
            std::env::var("ANTHROPIC_MODEL").unwrap_or_else(|_| "claude-3-5-sonnet-latest".into());
        let agent = agents::report::build(hephos::agent::Anthropic::new(key, model));
        run_agent(&agent).await
    } else if let Ok(key) = std::env::var("OPENAI_API_KEY") {
        let model = std::env::var("OPENAI_MODEL").unwrap_or_else(|_| "gpt-4o".into());
        let agent = agents::report::build(hephos::agent::OpenAI::new(key, model));
        run_agent(&agent).await
    } else {
        let agent = agents::report::build(ScriptedModel::canned());
        run_agent(&agent).await
    }
}

async fn run_agent<M: Model>(agent: &Agent<M>) -> Result<()> {
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
