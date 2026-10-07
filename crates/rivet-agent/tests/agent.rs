//! Agent-layer tests against a deterministic `MockModel` — no network, no provider.
//! Exercises `run`, `run_typed` (structured output success + failure), `stream`,
//! and the derived tool schema.

use futures::StreamExt;
use rivet_agent::{Agent, Completion, Limits, Message, Model, TextStream, Tool, tool_schema};
use rivet_core::Result;
use schemars::JsonSchema;
use serde::Deserialize;

/// Returns a fixed reply. For `run` the reply is prose; for `run_typed` the test
/// supplies a reply that is (or isn't) valid JSON for the target type.
struct MockModel {
    reply: String,
}

impl Model for MockModel {
    async fn complete(&self, _request: Completion) -> Result<Message> {
        Ok(Message::assistant(self.reply.clone()))
    }

    async fn stream(&self, _request: Completion) -> Result<TextStream> {
        let parts: Vec<Result<String>> = self
            .reply
            .split_inclusive(' ')
            .map(|s| Ok(s.to_string()))
            .collect();
        Ok(Box::pin(futures::stream::iter(parts)))
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
struct Thing {
    x: u32,
}

#[derive(Deserialize, JsonSchema)]
struct SearchInput {
    #[allow(dead_code)]
    query: String,
}

struct SearchTool;

impl Tool for SearchTool {
    type Input = SearchInput;
    type Output = u32;
    const NAME: &'static str = "search";
    const DESCRIPTION: &'static str = "search things";

    async fn call(&self, _input: SearchInput) -> Result<u32> {
        Ok(1)
    }
}

#[tokio::test]
async fn run_returns_model_text() {
    let agent = Agent::new(MockModel {
        reply: "hello world".into(),
    });
    assert_eq!(agent.run("hi").await.unwrap(), "hello world");
}

#[tokio::test]
async fn run_typed_parses_structured_output() {
    let agent = Agent::new(MockModel {
        reply: r#"{"x":7}"#.into(),
    });
    let thing: Thing = agent.run_typed("hi").await.unwrap();
    assert_eq!(thing.x, 7);
}

#[tokio::test]
async fn run_typed_errors_when_output_is_not_json() {
    let agent = Agent::new(MockModel {
        reply: "definitely not json".into(),
    });
    assert!(agent.run_typed::<Thing>("hi").await.is_err());
}

#[tokio::test]
async fn stream_yields_text_deltas() {
    let agent = Agent::new(MockModel {
        reply: "a b c".into(),
    });
    let mut stream = agent.stream("hi").await.unwrap();
    let mut out = String::new();
    while let Some(chunk) = stream.next().await {
        out.push_str(&chunk.unwrap());
    }
    assert_eq!(out, "a b c");
}

#[tokio::test]
async fn agent_with_tool_and_limits_still_runs() {
    let agent = Agent::new(MockModel { reply: "ok".into() })
        .system("you are a tester")
        .tool(SearchTool)
        .limits(Limits::default());
    assert_eq!(agent.run("hi").await.unwrap(), "ok");
}

#[test]
fn tool_schema_exposes_name_description_and_input_properties() {
    let schema = tool_schema::<SearchTool>();
    assert_eq!(schema.name, "search");
    assert_eq!(schema.description, "search things");
    let has_query = schema
        .input_schema
        .get("properties")
        .and_then(|props| props.get("query"))
        .is_some();
    assert!(has_query, "input schema should expose the `query` property");
}
