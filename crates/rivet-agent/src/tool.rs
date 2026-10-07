use rivet_core::Result;
use schemars::JsonSchema;
use serde::Serialize;
use serde::de::DeserializeOwned;

/// A strongly typed capability an agent may call.
///
/// Input and output are concrete Rust types. The input's JSON Schema is derived
/// with `schemars` and handed to the model, so the model cannot call a tool with
/// a shape the type system would reject. Validation beyond shape (ranges, cross
/// field rules) and authorization happen inside [`Tool::call`], returning the one
/// [`rivet_core::Error`] type.
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
///     async fn call(&self, input: SearchQuery) -> rivet::Result<Vec<UserSummary>> {
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
#[derive(Debug, Clone)]
pub struct ToolSchema {
    pub name: &'static str,
    pub description: &'static str,
    pub input_schema: serde_json::Value,
}

/// A model's request to invoke a tool, before dispatch.
#[derive(Debug, Clone)]
pub struct ToolCall {
    pub name: String,
    pub input: serde_json::Value,
}

/// The outcome of a tool invocation, fed back to the model.
#[derive(Debug, Clone)]
pub struct ToolResult {
    pub name: String,
    pub output: serde_json::Value,
}

/// Produce the [`ToolSchema`] for a tool type. Used by [`crate::Agent`] when
/// building a [`crate::Completion`].
pub fn schema_for<T: Tool>() -> ToolSchema {
    let schema = schemars::schema_for!(T::Input);
    ToolSchema {
        name: T::NAME,
        description: T::DESCRIPTION,
        input_schema: serde_json::to_value(schema).unwrap_or_default(),
    }
}
