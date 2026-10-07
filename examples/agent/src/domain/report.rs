use schemars::JsonSchema;
use serde::Deserialize;

/// Structured output. `run_typed::<Report>()` constrains the model to emit JSON
/// matching this schema and deserializes the result.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct Report {
    pub headline: String,
    pub bullet_points: Vec<String>,
}
