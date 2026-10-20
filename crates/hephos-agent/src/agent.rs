use std::time::Duration;

use futures::future::BoxFuture;
use hephos_core::{CancellationToken, Error, Result};
use schemars::JsonSchema;
use serde::de::DeserializeOwned;

use crate::message::{Message, TextStream};
use crate::model::{Completion, Model};
use crate::tool::{Tool, ToolCall, ToolDefinition, ToolResult, ToolSchema, Tools};
use tracing::Instrument;

/// Checkpoint persistence for durable agent runs.
pub trait AgentCheckpoint: Send + Sync {
    /// Load previously recorded messages for a specific turn, if available.
    fn load_turn(&self, turn: usize) -> BoxFuture<'_, Result<Option<Vec<Message>>>>;
    /// Save accumulated messages after completing a turn.
    fn save_turn(&self, turn: usize, messages: Vec<Message>) -> BoxFuture<'_, Result<()>>;
}

/// In-memory implementation of [`AgentCheckpoint`] for testing and ephemeral workflows.
#[derive(Debug, Clone, Default)]
pub struct InMemoryAgentCheckpoint {
    turns: std::sync::Arc<tokio::sync::RwLock<std::collections::HashMap<usize, Vec<Message>>>>,
}

impl InMemoryAgentCheckpoint {
    /// Create a new in-memory agent checkpoint store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of recorded turns.
    pub async fn turns_count(&self) -> usize {
        self.turns.read().await.len()
    }
}

impl AgentCheckpoint for InMemoryAgentCheckpoint {
    fn load_turn(&self, turn: usize) -> BoxFuture<'_, Result<Option<Vec<Message>>>> {
        Box::pin(async move {
            let guard = self.turns.read().await;
            Ok(guard.get(&turn).cloned())
        })
    }

    fn save_turn(&self, turn: usize, messages: Vec<Message>) -> BoxFuture<'_, Result<()>> {
        Box::pin(async move {
            let mut guard = self.turns.write().await;
            guard.insert(turn, messages);
            Ok(())
        })
    }
}

/// Bounds on a single agent run. There is no unbounded agent loop in Hephos.
///
/// Every run is constrained by a maximum number of model turns, a token ceiling,
/// a wall-clock timeout, and a per-turn tool-call fan-out cap. Hitting any bound
/// ends the run with an error, never silently.
#[derive(Debug, Clone)]
pub struct Limits {
    pub max_turns: u32,
    pub max_tokens: u32,
    pub timeout: Duration,
    pub max_tool_calls_per_turn: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_turns: 8,
            max_tokens: 4096,
            timeout: Duration::from_secs(60),
            max_tool_calls_per_turn: 16,
        }
    }
}

/// An agent: a model, a system prompt, a set of tools, and [`Limits`].
///
/// Built with a small chain of setters, then run. `run` returns the final text;
/// `run_typed` returns a deserialized value; `stream` returns text deltas. All
/// three enforce the same limits and share the same tool-dispatch loop.
pub struct Agent<M: Model> {
    model: M,
    system: String,
    tools: Tools,
    limits: Limits,
    cancel_token: Option<CancellationToken>,
}

impl<M: Model> std::fmt::Debug for Agent<M> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Agent")
            .field("system", &self.system)
            .field("tools", &self.tools)
            .field("limits", &self.limits)
            .field("cancel_token", &self.cancel_token)
            .finish_non_exhaustive()
    }
}

impl<M: Model> Agent<M> {
    pub fn new(model: M) -> Self {
        Agent {
            model,
            system: String::new(),
            tools: Tools::new(),
            limits: Limits::default(),
            cancel_token: None,
        }
    }

    pub fn system(mut self, system: impl Into<String>) -> Self {
        self.system = system.into();
        self
    }

    /// Register a single [`Tool`] on this agent.
    pub fn tool<T: Tool>(mut self, tool: T) -> Self {
        self.tools.register(tool);
        self
    }

    /// Register an entire [`Tools`] collection on this agent.
    pub fn tools(mut self, tools: Tools) -> Self {
        self.tools.extend(tools);
        self
    }

    /// Register an erased or dynamic [`ToolDefinition`] on this agent.
    pub fn tool_definition(mut self, def: ToolDefinition) -> Self {
        self.tools.register_definition(def);
        self
    }

    /// Access the registered tools collection.
    pub fn tools_ref(&self) -> &Tools {
        &self.tools
    }

    pub fn limits(mut self, limits: Limits) -> Self {
        self.limits = limits;
        self
    }

    /// Inherit cancellation from a context or shutdown token.
    pub fn cancel_token(mut self, cancel: CancellationToken) -> Self {
        self.cancel_token = Some(cancel);
        self
    }

    /// Run to a final text answer, dispatching tool calls along the way.
    pub async fn run(&self, input: &str) -> Result<String> {
        let message = self.drive(input, None).await?;
        Ok(message.content)
    }

    /// Run to a typed answer. The model is constrained to emit JSON matching
    /// `O`'s schema; the result is deserialized into `O`.
    pub async fn run_typed<O: DeserializeOwned + JsonSchema>(&self, input: &str) -> Result<O> {
        let schema = serde_json::to_value(schemars::schema_for!(O))
            .map_err(|e| Error::internal(format!("schema for output: {e}")))?;
        let message = self.drive(input, Some(schema)).await?;
        serde_json::from_str(&message.content)
            .map_err(|e| Error::internal(format!("model output did not match schema: {e}")))
    }

    /// Run with durable turn checkpointing. If interrupted, resuming with the same
    /// checkpoint store skips already-completed turns without re-invoking the model or tools.
    pub async fn run_durable<C: AgentCheckpoint>(
        &self,
        input: &str,
        checkpoint: &C,
    ) -> Result<String> {
        let message = self.drive_durable(input, None, checkpoint).await?;
        Ok(message.content)
    }

    /// Run with durable turn checkpointing constrained to structured output `O`.
    pub async fn run_typed_durable<O: DeserializeOwned + JsonSchema, C: AgentCheckpoint>(
        &self,
        input: &str,
        checkpoint: &C,
    ) -> Result<O> {
        let schema = serde_json::to_value(schemars::schema_for!(O))
            .map_err(|e| Error::internal(format!("schema for output: {e}")))?;
        let message = self.drive_durable(input, Some(schema), checkpoint).await?;
        serde_json::from_str(&message.content)
            .map_err(|e| Error::internal(format!("model output did not match schema: {e}")))
    }

    /// Stream the final answer as text deltas. Dropping the stream cancels
    /// generation. For tool-using agents, runs the tool dispatch loop until no
    /// further tools are called, then streams the final turn.
    pub async fn stream(&self, input: &str) -> Result<TextStream> {
        if self.tools.is_empty() {
            let completion = Completion {
                system: self.system.clone(),
                messages: vec![Message::user(input)],
                tools: Vec::new(),
                max_tokens: self.limits.max_tokens,
                response_schema: None,
            };
            self.model.stream(completion).await
        } else {
            let mut messages = vec![Message::user(input)];
            let tool_schemas = self.tool_schemas();
            for turn in 0..self.limits.max_turns {
                let turn_span = tracing::info_span!("agent.turn", turn = turn);
                let completion = Completion {
                    system: self.system.clone(),
                    messages: messages.clone(),
                    tools: tool_schemas.clone(),
                    max_tokens: self.limits.max_tokens,
                    response_schema: None,
                };
                let reply = self
                    .model
                    .complete(completion)
                    .instrument(turn_span.clone())
                    .await?;
                match parse_tool_calls(&reply) {
                    Some(calls) if !calls.is_empty() => {
                        if calls.len() > self.limits.max_tool_calls_per_turn {
                            return Err(Error::invalid(format!(
                                "model requested {} tool calls, exceeding the limit of {}",
                                calls.len(),
                                self.limits.max_tool_calls_per_turn
                            )));
                        }
                        messages.push(reply);
                        for call in calls {
                            let result = self.dispatch(call).instrument(turn_span.clone()).await?;
                            messages.push(tool_result_message(&result)?);
                        }
                    }
                    _ => {
                        let parts: Vec<Result<String>> = reply
                            .content
                            .split_inclusive(' ')
                            .map(|s| Ok(s.to_string()))
                            .collect();
                        return Ok(Box::pin(futures::stream::iter(parts)));
                    }
                }
            }
            Err(Error::internal(format!(
                "agent exceeded {} turns without a final answer",
                self.limits.max_turns
            )))
        }
    }

    fn tool_schemas(&self) -> Vec<ToolSchema> {
        self.tools.schemas()
    }

    /// The shared turn loop, bounded by every `Limits` dimension: it enforces the
    /// wall-clock `timeout` here, cancellation from `cancel_token`, and the turn count.
    async fn drive(
        &self,
        input: &str,
        response_schema: Option<serde_json::Value>,
    ) -> Result<Message> {
        let turns_fut = self.drive_turns(input, response_schema);
        let timeout_fut = tokio::time::timeout(self.limits.timeout, turns_fut);

        if let Some(cancel) = &self.cancel_token {
            tokio::select! {
                biased;
                _ = cancel.cancelled() => Err(Error::internal("agent execution cancelled")),
                res = timeout_fut => match res {
                    Ok(result) => result,
                    Err(_) => Err(Error::internal(format!(
                        "agent exceeded its time limit of {:?}",
                        self.limits.timeout
                    ))),
                }
            }
        } else {
            match timeout_fut.await {
                Ok(result) => result,
                Err(_) => Err(Error::internal(format!(
                    "agent exceeded its time limit of {:?}",
                    self.limits.timeout
                ))),
            }
        }
    }

    /// The turn/tool-dispatch loop. Call the model, dispatch any tool calls, repeat
    /// until the model returns a final answer or `max_turns` is hit.
    async fn drive_turns(
        &self,
        input: &str,
        response_schema: Option<serde_json::Value>,
    ) -> Result<Message> {
        let mut messages = vec![Message::user(input)];
        let tool_schemas = self.tool_schemas();
        for turn in 0..self.limits.max_turns {
            let turn_span = tracing::info_span!("agent.turn", turn = turn);
            let completion = Completion {
                system: self.system.clone(),
                messages: messages.clone(),
                tools: tool_schemas.clone(),
                max_tokens: self.limits.max_tokens,
                response_schema: response_schema.clone(),
            };
            let reply = self
                .model
                .complete(completion)
                .instrument(turn_span.clone())
                .await?;

            match parse_tool_calls(&reply) {
                Some(calls) if !calls.is_empty() => {
                    if calls.len() > self.limits.max_tool_calls_per_turn {
                        return Err(Error::invalid(format!(
                            "model requested {} tool calls, exceeding the limit of {}",
                            calls.len(),
                            self.limits.max_tool_calls_per_turn
                        )));
                    }
                    messages.push(reply);
                    for call in calls {
                        let result = self.dispatch(call).instrument(turn_span.clone()).await?;
                        messages.push(tool_result_message(&result)?);
                    }
                }
                _ => return Ok(reply),
            }
        }
        Err(Error::internal(format!(
            "agent exceeded {} turns without a final answer",
            self.limits.max_turns
        )))
    }

    async fn drive_durable<C: AgentCheckpoint>(
        &self,
        input: &str,
        response_schema: Option<serde_json::Value>,
        checkpoint: &C,
    ) -> Result<Message> {
        let turns_fut = self.drive_turns_durable(input, response_schema, checkpoint);
        let timeout_fut = tokio::time::timeout(self.limits.timeout, turns_fut);

        if let Some(cancel) = &self.cancel_token {
            tokio::select! {
                biased;
                _ = cancel.cancelled() => Err(Error::internal("agent execution cancelled")),
                res = timeout_fut => match res {
                    Ok(result) => result,
                    Err(_) => Err(Error::internal(format!(
                        "agent exceeded its time limit of {:?}",
                        self.limits.timeout
                    ))),
                }
            }
        } else {
            match timeout_fut.await {
                Ok(result) => result,
                Err(_) => Err(Error::internal(format!(
                    "agent exceeded its time limit of {:?}",
                    self.limits.timeout
                ))),
            }
        }
    }

    async fn drive_turns_durable<C: AgentCheckpoint>(
        &self,
        input: &str,
        response_schema: Option<serde_json::Value>,
        checkpoint: &C,
    ) -> Result<Message> {
        let mut messages = vec![Message::user(input)];
        let mut start_turn = 0;

        // Restore latest saved turn checkpoint, if one exists from a prior execution.
        for turn in 0..self.limits.max_turns as usize {
            if let Some(saved) = checkpoint.load_turn(turn).await? {
                messages = saved;
                start_turn = turn + 1;
            } else {
                break;
            }
        }

        // If the restored messages ended on a final answer (not tool execution), return it directly.
        if start_turn > 0 {
            if let Some(last) = messages.last() {
                if last.role == crate::message::Role::Assistant && parse_tool_calls(last).is_none()
                {
                    return Ok(last.clone());
                }
            }
        }

        let tool_schemas = self.tool_schemas();
        for turn in start_turn..self.limits.max_turns as usize {
            let turn_span = tracing::info_span!("agent.turn", turn = turn);
            let completion = Completion {
                system: self.system.clone(),
                messages: messages.clone(),
                tools: tool_schemas.clone(),
                max_tokens: self.limits.max_tokens,
                response_schema: response_schema.clone(),
            };
            let reply = self
                .model
                .complete(completion)
                .instrument(turn_span.clone())
                .await?;

            match parse_tool_calls(&reply) {
                Some(calls) if !calls.is_empty() => {
                    if calls.len() > self.limits.max_tool_calls_per_turn {
                        return Err(Error::invalid(format!(
                            "model requested {} tool calls, exceeding the limit of {}",
                            calls.len(),
                            self.limits.max_tool_calls_per_turn
                        )));
                    }
                    messages.push(reply);
                    for call in calls {
                        let result = self.dispatch(call).instrument(turn_span.clone()).await?;
                        messages.push(tool_result_message(&result)?);
                    }
                    checkpoint.save_turn(turn, messages.clone()).await?;
                }
                _ => {
                    messages.push(reply.clone());
                    checkpoint.save_turn(turn, messages.clone()).await?;
                    return Ok(reply);
                }
            }
        }
        Err(Error::internal(format!(
            "agent exceeded {} turns without a final answer",
            self.limits.max_turns
        )))
    }

    async fn dispatch(&self, call: ToolCall) -> Result<ToolResult> {
        let span = tracing::info_span!("agent.tool", tool = %call.name);
        self.tools.dispatch(call).instrument(span).await
    }
}

/// Provider-agnostic extraction of tool calls from a model message.
pub fn parse_tool_calls(message: &Message) -> Option<Vec<ToolCall>> {
    if message.tool_calls.is_empty() {
        None
    } else {
        Some(message.tool_calls.clone())
    }
}

fn tool_result_message(result: &ToolResult) -> Result<Message> {
    let content = serde_json::to_string(&result.output).map_err(|e| {
        Error::internal(format!(
            "failed to serialize tool result for '{}': {e}",
            result.name
        ))
    })?;
    Ok(Message {
        role: crate::message::Role::Tool,
        content,
        tool_calls: Vec::new(),
        tool_call_id: if result.call_id.is_empty() {
            None
        } else {
            Some(result.call_id.clone())
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limits_defaults_and_custom() {
        let default_limits = Limits::default();
        assert_eq!(default_limits.max_turns, 8);
        assert_eq!(default_limits.max_tokens, 4096);
        assert_eq!(default_limits.timeout, Duration::from_secs(60));
        assert_eq!(default_limits.max_tool_calls_per_turn, 16);

        let custom = Limits {
            max_turns: 5,
            max_tokens: 2048,
            timeout: Duration::from_secs(30),
            max_tool_calls_per_turn: 4,
        };
        assert_eq!(custom.max_turns, 5);
        assert_eq!(custom.max_tokens, 2048);
        assert_eq!(custom.timeout, Duration::from_secs(30));
        assert_eq!(custom.max_tool_calls_per_turn, 4);
    }

    #[test]
    fn tool_result_message_constructs_tool_role() {
        let res = ToolResult {
            call_id: "c1".into(),
            name: "calculator".into(),
            output: serde_json::json!({ "result": 42 }),
        };
        let msg = tool_result_message(&res).unwrap();
        assert_eq!(msg.role, crate::message::Role::Tool);
        assert_eq!(msg.tool_call_id.as_deref(), Some("c1"));
        assert_eq!(msg.content, r#"{"result":42}"#);
    }

    #[derive(serde::Deserialize, schemars::JsonSchema)]
    struct AddInput {
        a: i32,
        b: i32,
    }

    struct AddTool;

    impl Tool for AddTool {
        type Input = AddInput;
        type Output = i32;
        const NAME: &'static str = "add";
        const DESCRIPTION: &'static str = "adds two numbers";

        async fn call(&self, input: AddInput) -> Result<i32> {
            Ok(input.a + input.b)
        }
    }

    #[tokio::test]
    async fn tool_definition_invocation_and_error_handling() {
        let reg = ToolDefinition::new(AddTool);
        assert_eq!(reg.schema().name, "add");
        assert_eq!(reg.schema().description, "adds two numbers");

        // 1. Valid input
        let res = reg
            .call(serde_json::json!({ "a": 10, "b": 25 }), "c_123".into())
            .await
            .unwrap();
        assert_eq!(res.call_id, "c_123");
        assert_eq!(res.name, "add");
        assert_eq!(res.output, serde_json::json!(35));

        // 2. Invalid input (missing field)
        let err_missing = reg
            .call(serde_json::json!({ "a": 10 }), "c_123".into())
            .await
            .unwrap_err();
        assert!(matches!(err_missing, Error::Invalid(_)));

        // 3. Invalid input (wrong type)
        let err_type = reg
            .call(serde_json::json!({ "a": "ten", "b": 25 }), "c_123".into())
            .await
            .unwrap_err();
        assert!(matches!(err_type, Error::Invalid(_)));
    }

    struct EchoModel;

    impl Model for EchoModel {
        async fn complete(&self, _request: Completion) -> Result<Message> {
            Ok(Message::assistant("echo response"))
        }

        async fn stream(&self, _request: Completion) -> Result<TextStream> {
            Ok(Box::pin(futures::stream::empty()))
        }
    }

    #[tokio::test]
    async fn agent_unknown_tool_dispatch_fails() {
        let agent = Agent::new(EchoModel).tool(AddTool);
        let unknown_call = ToolCall {
            id: "call_sub".into(),
            name: "sub".into(),
            input: serde_json::json!({ "a": 5, "b": 2 }),
        };
        let err = agent.dispatch(unknown_call).await.unwrap_err();
        assert!(matches!(err, Error::Invalid(_)));
    }

    #[test]
    #[should_panic(expected = "already registered")]
    fn agent_duplicate_tool_panics() {
        let _agent = Agent::new(EchoModel).tool(AddTool).tool(AddTool);
    }

    struct SlowModel;

    impl Model for SlowModel {
        async fn complete(&self, _request: Completion) -> Result<Message> {
            tokio::time::sleep(Duration::from_millis(50)).await;
            Ok(Message::assistant("done"))
        }

        async fn stream(&self, _request: Completion) -> Result<TextStream> {
            Ok(Box::pin(futures::stream::empty()))
        }
    }

    #[tokio::test]
    async fn agent_timeout_enforcement() {
        let agent = Agent::new(SlowModel).limits(Limits {
            max_turns: 3,
            max_tokens: 100,
            timeout: Duration::from_millis(10),
            max_tool_calls_per_turn: 16,
        });

        let err = agent.run("hello").await.unwrap_err();
        assert!(matches!(err, Error::Internal(_)));
        assert!(err.to_string().contains("exceeded its time limit"));
    }

    #[tokio::test]
    async fn agent_cancellation_enforcement() {
        let cancel = CancellationToken::new();
        let cancel_clone = cancel.clone();

        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(5)).await;
            cancel_clone.cancel();
        });

        let agent = Agent::new(SlowModel)
            .limits(Limits {
                max_turns: 3,
                max_tokens: 100,
                timeout: Duration::from_secs(10),
                max_tool_calls_per_turn: 16,
            })
            .cancel_token(cancel);

        let err = agent.run("hello").await.unwrap_err();
        assert!(matches!(err, Error::Internal(_)));
        assert!(err.to_string().contains("cancelled"));
    }

    #[derive(Clone)]
    struct MultiTurnModel {
        turns: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    }

    impl Model for MultiTurnModel {
        async fn complete(&self, _request: Completion) -> Result<Message> {
            let turn = self.turns.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if turn == 0 {
                let call = ToolCall {
                    id: "call_inc".into(),
                    name: "inc".into(),
                    input: serde_json::json!({}),
                };
                let mut msg = Message::assistant("calling increment");
                msg.tool_calls.push(call);
                Ok(msg)
            } else {
                Ok(Message::assistant("increment completed successfully"))
            }
        }

        async fn stream(&self, _request: Completion) -> Result<TextStream> {
            Ok(Box::pin(futures::stream::empty()))
        }
    }

    #[derive(Clone)]
    struct IncTool {
        counter: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    }

    impl Tool for IncTool {
        type Input = serde_json::Value;
        type Output = usize;

        const NAME: &'static str = "inc";
        const DESCRIPTION: &'static str = "increments counter";

        async fn call(&self, _input: Self::Input) -> Result<Self::Output> {
            Ok(self
                .counter
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
                + 1)
        }
    }

    #[tokio::test]
    async fn agent_durable_resumes_from_saved_turn_without_reexecuting_tools() {
        let counter = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let model = MultiTurnModel {
            turns: std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        };

        let tool = IncTool {
            counter: counter.clone(),
        };

        let checkpoint = InMemoryAgentCheckpoint::new();

        // Run 1: completes both turns
        let agent1 = Agent::new(model.clone()).tool(tool);
        let res1 = agent1.run_durable("start", &checkpoint).await.unwrap();
        assert_eq!(res1, "increment completed successfully");
        assert_eq!(counter.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(checkpoint.turns_count().await, 2);

        // Run 2: same checkpoint, tool must NOT be called again
        let tool2 = IncTool {
            counter: counter.clone(),
        };
        let agent2 = Agent::new(model.clone()).tool(tool2);
        let res2 = agent2.run_durable("start", &checkpoint).await.unwrap();
        assert_eq!(res2, "increment completed successfully");
        // Tool counter is still 1!
        assert_eq!(counter.load(std::sync::atomic::Ordering::SeqCst), 1);
    }
}
