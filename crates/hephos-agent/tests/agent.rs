//! Agent-layer tests against deterministic models — no network, no provider.
//! Exercises `run`, `run_typed` (structured output success + failure), `stream`,
//! tool dispatch across turns, max turns overrun, tool fan-out caps, and cancellation.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::StreamExt;
use hephos_agent::{
    Agent, Completion, Limits, Message, Model, TextStream, Tool, ToolCall, tool_schema,
};
use hephos_core::Result;
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

/// A sequential model that yields replies from a FIFO queue on each call.
struct SequenceModel {
    responses: Mutex<Vec<Message>>,
}

impl SequenceModel {
    fn new(responses: Vec<Message>) -> Self {
        SequenceModel {
            responses: Mutex::new(responses),
        }
    }
}

impl Model for SequenceModel {
    async fn complete(&self, _request: Completion) -> Result<Message> {
        let mut guard = self.responses.lock().unwrap();
        if guard.is_empty() {
            Ok(Message::assistant("done"))
        } else {
            Ok(guard.remove(0))
        }
    }

    async fn stream(&self, _request: Completion) -> Result<TextStream> {
        let mut guard = self.responses.lock().unwrap();
        let msg = if guard.is_empty() {
            Message::assistant("final")
        } else {
            guard.remove(0)
        };
        let parts = vec![Ok(msg.content)];
        Ok(Box::pin(futures::stream::iter(parts)))
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
struct Thing {
    x: u32,
}

#[derive(Deserialize, JsonSchema)]
struct SearchInput {
    query: String,
}

struct SearchTool;

impl Tool for SearchTool {
    type Input = SearchInput;
    type Output = u32;
    const NAME: &'static str = "search";
    const DESCRIPTION: &'static str = "search things";

    async fn call(&self, input: SearchInput) -> Result<u32> {
        if input.query == "rust" { Ok(42) } else { Ok(0) }
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

#[tokio::test]
async fn tool_dispatch_multi_turn_success() {
    let model = SequenceModel::new(vec![
        Message::assistant_with_tools(
            "searching...",
            vec![ToolCall::with_id(
                "call_1",
                "search",
                serde_json::json!({ "query": "rust" }),
            )],
        ),
        Message::assistant("search returned 42 matches"),
    ]);

    let agent = Agent::new(model).tool(SearchTool);
    let output = agent.run("how many matches for rust?").await.unwrap();
    assert_eq!(output, "search returned 42 matches");
}

#[tokio::test]
async fn tool_dispatch_with_structured_output() {
    let model = SequenceModel::new(vec![
        Message::assistant_with_tools(
            "checking",
            vec![ToolCall::with_id(
                "call_1",
                "search",
                serde_json::json!({ "query": "rust" }),
            )],
        ),
        Message::assistant(r#"{"x":42}"#),
    ]);

    let agent = Agent::new(model).tool(SearchTool);
    let result: Thing = agent.run_typed("lookup").await.unwrap();
    assert_eq!(result.x, 42);
}

#[tokio::test]
async fn max_turns_overrun_errors() {
    // Model perpetually asks for tools without ever producing a final text answer
    struct ForeverToolModel;
    impl Model for ForeverToolModel {
        async fn complete(&self, _request: Completion) -> Result<Message> {
            Ok(Message::assistant_with_tools(
                "looping",
                vec![ToolCall::with_id(
                    "call_x",
                    "search",
                    serde_json::json!({ "query": "rust" }),
                )],
            ))
        }
        async fn stream(&self, _request: Completion) -> Result<TextStream> {
            Ok(Box::pin(futures::stream::empty()))
        }
    }

    let agent = Agent::new(ForeverToolModel)
        .tool(SearchTool)
        .limits(Limits {
            max_turns: 3,
            max_tokens: 1000,
            timeout: Duration::from_secs(10),
            max_tool_calls_per_turn: 16,
        });

    let err = agent.run("loop").await.unwrap_err();
    assert!(
        err.to_string().contains("exceeded 3 turns"),
        "expected turn limit error, got: {err}"
    );
}

#[tokio::test]
async fn max_tool_calls_per_turn_cap_enforced() {
    struct SpammerModel;
    impl Model for SpammerModel {
        async fn complete(&self, _request: Completion) -> Result<Message> {
            let calls = vec![
                ToolCall::new("search", serde_json::json!({ "query": "a" })),
                ToolCall::new("search", serde_json::json!({ "query": "b" })),
                ToolCall::new("search", serde_json::json!({ "query": "c" })),
            ];
            Ok(Message::assistant_with_tools("spam", calls))
        }
        async fn stream(&self, _request: Completion) -> Result<TextStream> {
            Ok(Box::pin(futures::stream::empty()))
        }
    }

    let agent = Agent::new(SpammerModel).tool(SearchTool).limits(Limits {
        max_turns: 5,
        max_tokens: 1000,
        timeout: Duration::from_secs(10),
        max_tool_calls_per_turn: 2, // only allow 2
    });

    let err = agent.run("test").await.unwrap_err();
    assert!(
        err.to_string().contains("exceeding the limit of 2"),
        "expected fan-out cap error, got: {err}"
    );
}

#[tokio::test]
async fn tool_using_agent_streams_final_turn() {
    // Detect-then-stream: the tool turn uses complete(); the next complete() returns no
    // tools (the agent detects the final turn) and is re-issued via stream(). SequenceModel
    // draws complete() and stream() from one queue, so three entries are consumed in order.
    let model = SequenceModel::new(vec![
        // turn 0 (complete): request a tool
        Message::assistant_with_tools(
            "thinking",
            vec![ToolCall::with_id(
                "c1",
                "search",
                serde_json::json!({ "query": "rust" }),
            )],
        ),
        // turn 1 (complete): no tools -> triggers the final streamed turn
        Message::assistant("deciding to answer"),
        // the re-issued final turn (stream): the actually-streamed answer
        Message::assistant("final answer stream"),
    ]);

    let agent = Agent::new(model).tool(SearchTool);
    let mut stream = agent.stream("query").await.unwrap();
    let mut collected = String::new();
    while let Some(chunk) = stream.next().await {
        collected.push_str(&chunk.unwrap());
    }
    assert_eq!(collected, "final answer stream");
}

#[tokio::test]
async fn dropping_stream_cancels_generation() {
    let dropped = Arc::new(AtomicBool::new(false));
    let dropped_clone = dropped.clone();

    struct DropTrackerModel {
        dropped: Arc<AtomicBool>,
    }

    impl Model for DropTrackerModel {
        async fn complete(&self, _request: Completion) -> Result<Message> {
            Ok(Message::assistant("done"))
        }

        async fn stream(&self, _request: Completion) -> Result<TextStream> {
            struct Guard(Arc<AtomicBool>);
            impl Drop for Guard {
                fn drop(&mut self) {
                    self.0.store(true, Ordering::SeqCst);
                }
            }

            let guard = Guard(self.dropped.clone());
            let stream = async_stream::try_stream! {
                let _g = guard;
                yield "first".to_string();
                tokio::time::sleep(Duration::from_secs(10)).await;
                yield "second".to_string();
            };
            Ok(Box::pin(stream))
        }
    }

    let agent = Agent::new(DropTrackerModel { dropped });
    let mut stream = agent.stream("test").await.unwrap();

    let first = stream.next().await.unwrap().unwrap();
    assert_eq!(first, "first");

    // Drop the stream early
    drop(stream);

    assert!(
        dropped_clone.load(Ordering::SeqCst),
        "expected stream generator to be dropped and cancelled"
    );
}

#[tokio::test]
async fn tool_using_agent_streams_after_recovering_from_tool_error() {
    struct FailOnceTool;
    impl Tool for FailOnceTool {
        type Input = serde_json::Value;
        type Output = String;
        const NAME: &'static str = "fail_tool";
        const DESCRIPTION: &'static str = "fails with not_found";

        async fn call(&self, _input: Self::Input) -> Result<String> {
            Err(hephos_core::Error::not_found("item not found"))
        }
    }

    // turn 0 fails the tool; turn 1's complete() returns no tools (recovered) and is
    // re-issued via stream() for the real streamed final answer (three queue entries).
    let model = SequenceModel::new(vec![
        Message::assistant_with_tools(
            "trying tool",
            vec![ToolCall::with_id(
                "c_fail",
                "fail_tool",
                serde_json::json!({}),
            )],
        ),
        // complete() after the tool error: no tools -> triggers the final streamed turn
        Message::assistant("deciding to answer after tool error"),
        // the re-issued final turn (stream)
        Message::assistant("recovered stream output"),
    ]);

    let agent = Agent::new(model).tool(FailOnceTool);
    let mut stream = agent.stream("start").await.unwrap();
    let mut text = String::new();
    while let Some(chunk) = stream.next().await {
        text.push_str(&chunk.unwrap());
    }
    assert_eq!(text, "recovered stream output");
}

#[tokio::test]
async fn model_transient_errors_are_retried_then_succeed() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct FlakyModel {
        calls: AtomicUsize,
    }
    impl Model for FlakyModel {
        async fn complete(&self, _request: Completion) -> Result<Message> {
            // Fail the first two attempts with retryable errors, then succeed.
            let n = self.calls.fetch_add(1, Ordering::SeqCst);
            if n == 0 {
                Err(hephos_core::Error::unavailable("upstream blip"))
            } else if n == 1 {
                Err(hephos_core::Error::rate_limited("slow down"))
            } else {
                Ok(Message::assistant("recovered"))
            }
        }
        async fn stream(&self, _request: Completion) -> Result<TextStream> {
            Ok(Box::pin(futures::stream::iter(vec![Ok("x".to_string())])))
        }
    }

    let agent = Agent::new(FlakyModel {
        calls: AtomicUsize::new(0),
    });
    // Two transient failures are retried (backoff), the third attempt succeeds.
    assert_eq!(agent.run("hi").await.unwrap(), "recovered");
}

#[tokio::test]
async fn model_non_retryable_errors_fail_fast() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct AlwaysInvalidModel {
        calls: Arc<AtomicUsize>,
    }
    impl Model for AlwaysInvalidModel {
        async fn complete(&self, _request: Completion) -> Result<Message> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Err(hephos_core::Error::invalid("bad request"))
        }
        async fn stream(&self, _request: Completion) -> Result<TextStream> {
            Ok(Box::pin(
                futures::stream::iter(Vec::<Result<String>>::new()),
            ))
        }
    }

    let calls = Arc::new(AtomicUsize::new(0));
    let agent = Agent::new(AlwaysInvalidModel {
        calls: calls.clone(),
    });
    assert!(agent.run("hi").await.is_err());
    // A non-retryable error must NOT be retried.
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}
