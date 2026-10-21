use hephos_core::{Error, Result};
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

/// A strongly typed capability an agent may call.
///
/// Input and output are concrete Rust types. The input's JSON Schema is derived
/// with `schemars` and handed to the model, so the model cannot call a tool with
/// a shape the type system would reject. Validation beyond shape (ranges, cross
/// field rules) and authorization happen inside [`Tool::call`], returning the one
/// [`hephos_core::Error`] type.
///
/// A tool captures its dependencies at construction. A database-backed tool holds
/// a `Db`; an authorized tool holds the caller's identity. The agent never needs
/// to know about application state.
///
/// ```ignore
/// pub struct SearchUsers { db: Db }
///
/// impl Tool for SearchUsers {
///     type Input = SearchQuery;
///     type Output = Vec<UserSummary>;
///     const NAME: &'static str = "search_users";
///     const DESCRIPTION: &'static str = "Search users by name or email.";
///
///     async fn call(&self, input: SearchQuery) -> hephos::Result<Vec<UserSummary>> {
///         if input.query.trim().is_empty() {
///             return Err(Error::invalid("query must not be empty"));
///         }
///         UserRepo::search(self.db.pool(), &input.query).await
///     }
/// }
/// ```
pub trait Tool: Send + Sync + 'static {
    type Input: DeserializeOwned + JsonSchema + Send;
    type Output: Serialize + Send;

    const NAME: &'static str;
    const DESCRIPTION: &'static str;

    fn call(
        &self,
        input: Self::Input,
    ) -> impl std::future::Future<Output = Result<Self::Output>> + Send;
}

/// The model-facing description of a tool: name, description, and input schema.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolSchema {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

impl ToolSchema {
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        input_schema: serde_json::Value,
    ) -> Self {
        ToolSchema {
            name: name.into(),
            description: description.into(),
            input_schema,
        }
    }
}

/// A model's request to invoke a tool, before dispatch.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolCall {
    #[serde(default)]
    pub id: String,
    pub name: String,
    pub input: serde_json::Value,
}

impl ToolCall {
    pub fn new(name: impl Into<String>, input: serde_json::Value) -> Self {
        ToolCall {
            id: String::new(),
            name: name.into(),
            input,
        }
    }

    pub fn with_id(
        id: impl Into<String>,
        name: impl Into<String>,
        input: serde_json::Value,
    ) -> Self {
        ToolCall {
            id: id.into(),
            name: name.into(),
            input,
        }
    }
}

/// The outcome of a tool invocation, fed back to the model.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolResult {
    #[serde(default)]
    pub call_id: String,
    pub name: String,
    pub output: serde_json::Value,
}

impl ToolResult {
    pub fn new(name: impl Into<String>, output: serde_json::Value) -> Self {
        ToolResult {
            call_id: String::new(),
            name: name.into(),
            output,
        }
    }

    pub fn with_call_id(
        call_id: impl Into<String>,
        name: impl Into<String>,
        output: serde_json::Value,
    ) -> Self {
        ToolResult {
            call_id: call_id.into(),
            name: name.into(),
            output,
        }
    }
}

/// Produce the [`ToolSchema`] for a tool type. Used by [`crate::Agent`] when
/// building a [`crate::Completion`].
pub fn schema_for<T: Tool>() -> ToolSchema {
    let schema = schemars::schema_for!(T::Input);
    ToolSchema {
        name: T::NAME.to_string(),
        description: T::DESCRIPTION.to_string(),
        input_schema: serde_json::to_value(schema).unwrap_or_default(),
    }
}

/// An erased, callable tool definition.
///
/// Wraps a strongly typed [`Tool`] or a dynamic tool closure, presenting a uniform
/// schema and execution interface.
#[derive(Clone)]
pub struct ToolDefinition {
    schema: ToolSchema,
    invoke: std::sync::Arc<
        dyn Fn(serde_json::Value, String) -> futures::future::BoxFuture<'static, Result<ToolResult>>
            + Send
            + Sync,
    >,
}

impl std::fmt::Debug for ToolDefinition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolDefinition")
            .field("schema", &self.schema)
            .finish_non_exhaustive()
    }
}

impl ToolDefinition {
    /// Wrap a strongly typed [`Tool`].
    pub fn new<T: Tool>(tool: T) -> Self {
        let schema = schema_for::<T>();
        let name = T::NAME.to_string();
        let tool = std::sync::Arc::new(tool);
        let invoke = std::sync::Arc::new(move |raw: serde_json::Value, call_id: String| {
            let tool = std::sync::Arc::clone(&tool);
            let name = name.clone();
            Box::pin(async move {
                let input: T::Input = serde_json::from_value(raw)
                    .map_err(|e| Error::invalid(format!("invalid input for {name}: {e}")))?;
                let output = tool.call(input).await?;
                let output = serde_json::to_value(output)
                    .map_err(|e| Error::internal(format!("tool {name} output: {e}")))?;
                Ok(ToolResult {
                    call_id,
                    name,
                    output,
                })
            }) as futures::future::BoxFuture<'static, Result<ToolResult>>
        });
        ToolDefinition { schema, invoke }
    }

    /// Create a dynamic tool definition from a schema and an asynchronous invocation function.
    pub fn dynamic<F>(schema: ToolSchema, invoke_fn: F) -> Self
    where
        F: Fn(serde_json::Value) -> futures::future::BoxFuture<'static, Result<serde_json::Value>>
            + Send
            + Sync
            + 'static,
    {
        let name = schema.name.clone();
        let invoke_fn = std::sync::Arc::new(invoke_fn);
        let invoke = std::sync::Arc::new(move |raw: serde_json::Value, call_id: String| {
            let invoke_fn = std::sync::Arc::clone(&invoke_fn);
            let name = name.clone();
            Box::pin(async move {
                let output = invoke_fn(raw).await?;
                Ok(ToolResult {
                    call_id,
                    name,
                    output,
                })
            }) as futures::future::BoxFuture<'static, Result<ToolResult>>
        });
        ToolDefinition { schema, invoke }
    }

    /// Access the tool's schema.
    pub fn schema(&self) -> &ToolSchema {
        &self.schema
    }

    /// Invoke the tool with raw JSON input.
    pub async fn call(&self, input: serde_json::Value, call_id: String) -> Result<ToolResult> {
        (self.invoke)(input, call_id).await
    }

    /// Invoke the tool with raw JSON input, returning just the output value.
    pub async fn call_raw(&self, input: serde_json::Value) -> Result<serde_json::Value> {
        let res = self.call(input, String::new()).await?;
        Ok(res.output)
    }
}

/// A registry and collection of tools that can be shared between an in-process
/// [`crate::Agent`] and an MCP server.
///
/// Implements "one tool definition, two consumers": register tools once into a
/// [`Tools`] collection, then pass it to [`crate::Agent::tools`] and
/// an MCP server (`McpServer::tools`).
#[derive(Clone, Debug, Default)]
pub struct Tools {
    items: Vec<ToolDefinition>,
}

/// Canonical alias for [`Tools`].
pub type ToolRegistry = Tools;

impl Tools {
    /// Create an empty tool collection.
    pub fn new() -> Self {
        Tools { items: Vec::new() }
    }

    /// Register a typed [`Tool`] using builder chaining.
    pub fn tool<T: Tool>(mut self, tool: T) -> Self {
        self.register(tool);
        self
    }

    /// Register a typed [`Tool`] by mutating the collection.
    pub fn register<T: Tool>(&mut self, tool: T) -> &mut Self {
        assert!(
            !self.items.iter().any(|t| t.schema().name == T::NAME),
            "tool with name '{}' is already registered in this collection",
            T::NAME
        );
        self.items.push(ToolDefinition::new(tool));
        self
    }

    /// Register a [`ToolDefinition`].
    pub fn register_definition(&mut self, def: ToolDefinition) -> &mut Self {
        assert!(
            !self
                .items
                .iter()
                .any(|t| t.schema().name == def.schema().name),
            "tool with name '{}' is already registered in this collection",
            def.schema().name
        );
        self.items.push(def);
        self
    }

    /// Extend this collection with another [`Tools`] collection.
    pub fn extend(&mut self, other: Tools) -> &mut Self {
        for def in other.items {
            self.register_definition(def);
        }
        self
    }

    /// Return schemas for all registered tools.
    pub fn schemas(&self) -> Vec<ToolSchema> {
        self.items.iter().map(|t| t.schema().clone()).collect()
    }

    /// Find a tool definition by name.
    pub fn get(&self, name: &str) -> Option<&ToolDefinition> {
        self.items.iter().find(|t| t.schema().name == name)
    }

    /// Return true if a tool with the given name is registered.
    pub fn contains(&self, name: &str) -> bool {
        self.get(name).is_some()
    }

    /// Dispatch a [`ToolCall`] to the matching registered tool.
    pub async fn dispatch(&self, call: ToolCall) -> Result<ToolResult> {
        let tool = self
            .get(&call.name)
            .ok_or_else(|| Error::invalid(format!("unknown tool: {}", call.name)))?;
        tool.call(call.input, call.id).await
    }

    /// Return the slice of registered tool definitions.
    pub fn definitions(&self) -> &[ToolDefinition] {
        &self.items
    }

    /// Return true if no tools are registered.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Return the number of registered tools.
    pub fn len(&self) -> usize {
        self.items.len()
    }
}

impl IntoIterator for Tools {
    type Item = ToolDefinition;
    type IntoIter = std::vec::IntoIter<ToolDefinition>;

    fn into_iter(self) -> Self::IntoIter {
        self.items.into_iter()
    }
}

impl<'a> IntoIterator for &'a Tools {
    type Item = &'a ToolDefinition;
    type IntoIter = std::slice::Iter<'a, ToolDefinition>;

    fn into_iter(self) -> Self::IntoIter {
        self.items.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Deserialize, JsonSchema)]
    struct MockInput {
        name: String,
        count: u32,
    }

    struct MockTool;

    impl Tool for MockTool {
        type Input = MockInput;
        type Output = String;
        const NAME: &'static str = "mock_tool";
        const DESCRIPTION: &'static str = "A mock tool for testing";

        async fn call(&self, input: Self::Input) -> Result<Self::Output> {
            Ok(format!("{}: {}", input.name, input.count))
        }
    }

    #[tokio::test]
    async fn schema_for_generates_valid_schema() {
        let schema = schema_for::<MockTool>();
        assert_eq!(schema.name, "mock_tool");
        assert_eq!(schema.description, "A mock tool for testing");
        assert!(schema.input_schema.get("properties").is_some());
        assert!(schema.input_schema["properties"].get("name").is_some());
        assert!(schema.input_schema["properties"].get("count").is_some());
    }

    #[tokio::test]
    async fn tool_call_and_result() {
        let tool = MockTool;
        let res = tool
            .call(MockInput {
                name: "item".into(),
                count: 3,
            })
            .await
            .unwrap();
        assert_eq!(res, "item: 3");

        let call = ToolCall::new(
            "mock_tool",
            serde_json::json!({ "name": "item", "count": 3 }),
        );
        assert_eq!(call.name, "mock_tool");

        let tool_res = ToolResult::new("mock_tool", serde_json::json!("item: 3"));
        assert_eq!(tool_res.name, "mock_tool");
    }

    #[tokio::test]
    async fn tools_collection_registration_and_dispatch() {
        let mut tools = Tools::new();
        tools.register(MockTool);

        assert_eq!(tools.len(), 1);
        assert!(!tools.is_empty());
        assert!(tools.contains("mock_tool"));
        assert!(!tools.contains("unknown_tool"));

        let schemas = tools.schemas();
        assert_eq!(schemas.len(), 1);
        assert_eq!(schemas[0].name, "mock_tool");

        let call = ToolCall::with_id(
            "call_1",
            "mock_tool",
            serde_json::json!({ "name": "apples", "count": 5 }),
        );
        let result = tools.dispatch(call).await.unwrap();
        assert_eq!(result.call_id, "call_1");
        assert_eq!(result.name, "mock_tool");
        assert_eq!(result.output, serde_json::json!("apples: 5"));

        // Dispatching to an unknown tool errors cleanly
        let bad_call = ToolCall::new("unknown_tool", serde_json::json!({}));
        let err = tools.dispatch(bad_call).await.unwrap_err();
        assert!(matches!(err, Error::Invalid(_)));
    }

    #[tokio::test]
    async fn dynamic_tool_definition() {
        let schema = ToolSchema::new(
            "dynamic_echo",
            "Echoes the input back",
            serde_json::json!({ "type": "object" }),
        );
        let def = ToolDefinition::dynamic(schema, |input| {
            Box::pin(async move { Ok(serde_json::json!({ "echoed": input })) })
        });

        let mut tools = Tools::new();
        tools.register_definition(def);

        let call = ToolCall::new(
            "dynamic_echo",
            serde_json::json!({ "msg": "hello from dynamic" }),
        );
        let res = tools.dispatch(call).await.unwrap();
        assert_eq!(
            res.output,
            serde_json::json!({ "echoed": { "msg": "hello from dynamic" } })
        );
    }
}
