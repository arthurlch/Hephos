//! Anthropic Messages API provider implementation.

use hephos_core::security::SecretString;
use hephos_core::{Error, Result};
use serde::{Deserialize, Serialize};

use super::common::{default_http_client, parse_sse_stream};
use crate::message::{Message, Role, TextStream};
use crate::model::{Completion, Model};
use crate::tool::ToolCall;

/// Anthropic Messages API client.
#[derive(Debug)]
pub struct Anthropic {
    api_key: SecretString,
    model: String,
    base_url: String,
    http: reqwest::Client,
}

impl Anthropic {
    /// Construct from `ANTHROPIC_API_KEY`.
    pub fn from_env(model: impl Into<String>) -> Result<Self> {
        let api_key = std::env::var("ANTHROPIC_API_KEY")
            .map_err(|_| Error::invalid("ANTHROPIC_API_KEY is not set"))?;
        Ok(Self::new(api_key, model))
    }

    /// Construct with an explicit API key and model name.
    pub fn new(api_key: impl Into<SecretString>, model: impl Into<String>) -> Self {
        Anthropic {
            api_key: api_key.into(),
            model: model.into(),
            base_url: "https://api.anthropic.com".into(),
            http: default_http_client(),
        }
    }

    /// Override the base URL (e.g. for testing against a mock server or proxy).
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    /// Override the underlying HTTP client (e.g. for enterprise proxies or custom timeouts).
    pub fn with_client(mut self, client: reqwest::Client) -> Self {
        self.http = client;
        self
    }
}

impl Model for Anthropic {
    async fn complete(&self, request: Completion) -> Result<Message> {
        let url = format!("{}/v1/messages", self.base_url.trim_end_matches('/'));
        let has_schema = request.response_schema.is_some();
        let payload = build_anthropic_payload(&self.model, &request, false)?;

        let resp = self
            .http
            .post(&url)
            .header("x-api-key", self.api_key.expose_secret())
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&payload)
            .send()
            .await
            .map_err(|e| Error::internal(format!("anthropic request failed: {e}")))?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                return Err(Error::rate_limited(format!("anthropic rate limit: {body}")));
            }
            if status == reqwest::StatusCode::SERVICE_UNAVAILABLE {
                return Err(Error::unavailable(format!("anthropic unavailable: {body}")));
            }
            return Err(Error::internal(format!(
                "anthropic API returned status {status}: {body}"
            )));
        }

        let body: AnthropicResponse = resp
            .json()
            .await
            .map_err(|e| Error::internal(format!("failed to parse anthropic response: {e}")))?;

        parse_anthropic_response(body, has_schema)
    }

    async fn stream(&self, request: Completion) -> Result<TextStream> {
        let url = format!("{}/v1/messages", self.base_url.trim_end_matches('/'));
        let payload = build_anthropic_payload(&self.model, &request, true)?;

        let resp = self
            .http
            .post(&url)
            .header("x-api-key", self.api_key.expose_secret())
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&payload)
            .send()
            .await
            .map_err(|e| Error::internal(format!("anthropic stream request failed: {e}")))?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                return Err(Error::rate_limited(format!("anthropic rate limit: {body}")));
            }
            if status == reqwest::StatusCode::SERVICE_UNAVAILABLE {
                return Err(Error::unavailable(format!("anthropic unavailable: {body}")));
            }
            return Err(Error::internal(format!(
                "anthropic API returned status {status}: {body}"
            )));
        }

        Ok(parse_sse_stream(resp, extract_anthropic_sse_delta))
    }
}

#[derive(Serialize)]
pub(crate) struct AnthropicPayload<'a> {
    pub(crate) model: &'a str,
    pub(crate) max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) system: Option<&'a str>,
    pub(crate) messages: Vec<AnthropicMessagePayload<'a>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) tools: Vec<AnthropicToolPayload<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) tool_choice: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub(crate) stream: bool,
}

#[derive(Serialize)]
pub(crate) struct AnthropicMessagePayload<'a> {
    pub(crate) role: &'a str,
    pub(crate) content: AnthropicContentPayload<'a>,
}

#[derive(Serialize)]
#[serde(untagged)]
pub(crate) enum AnthropicContentPayload<'a> {
    Text(&'a str),
    Blocks(Vec<serde_json::Value>),
}

#[derive(Serialize)]
pub(crate) struct AnthropicToolPayload<'a> {
    pub(crate) name: &'a str,
    pub(crate) description: &'a str,
    pub(crate) input_schema: &'a serde_json::Value,
}

pub(crate) fn build_anthropic_payload<'a>(
    model: &'a str,
    request: &'a Completion,
    stream: bool,
) -> Result<AnthropicPayload<'a>> {
    let system = if request.system.trim().is_empty() {
        None
    } else {
        Some(request.system.as_str())
    };

    let mut messages = Vec::new();
    for msg in &request.messages {
        match msg.role {
            Role::User => {
                messages.push(AnthropicMessagePayload {
                    role: "user",
                    content: AnthropicContentPayload::Text(&msg.content),
                });
            }
            Role::Assistant => {
                if msg.tool_calls.is_empty() {
                    messages.push(AnthropicMessagePayload {
                        role: "assistant",
                        content: AnthropicContentPayload::Text(&msg.content),
                    });
                } else {
                    let mut blocks = Vec::new();
                    if !msg.content.is_empty() {
                        blocks.push(serde_json::json!({
                            "type": "text",
                            "text": msg.content
                        }));
                    }
                    for call in &msg.tool_calls {
                        blocks.push(serde_json::json!({
                            "type": "tool_use",
                            "id": call.id,
                            "name": call.name,
                            "input": call.input
                        }));
                    }
                    messages.push(AnthropicMessagePayload {
                        role: "assistant",
                        content: AnthropicContentPayload::Blocks(blocks),
                    });
                }
            }
            Role::Tool => {
                let tool_use_id = msg.tool_call_id.as_deref().unwrap_or("");
                let block = serde_json::json!({
                    "type": "tool_result",
                    "tool_use_id": tool_use_id,
                    "content": msg.content
                });
                messages.push(AnthropicMessagePayload {
                    role: "user",
                    content: AnthropicContentPayload::Blocks(vec![block]),
                });
            }
            Role::System => {}
        }
    }

    let mut tools = Vec::new();
    for t in &request.tools {
        tools.push(AnthropicToolPayload {
            name: &t.name,
            description: &t.description,
            input_schema: &t.input_schema,
        });
    }

    let tool_choice = if let Some(schema) = &request.response_schema {
        tools.push(AnthropicToolPayload {
            name: "__structured_output",
            description: "Structured response matching schema",
            input_schema: schema,
        });
        Some(serde_json::json!({
            "type": "tool",
            "name": "__structured_output"
        }))
    } else {
        None
    };

    Ok(AnthropicPayload {
        model,
        max_tokens: request.max_tokens,
        system,
        messages,
        tools,
        tool_choice,
        stream,
    })
}

#[derive(Deserialize)]
struct AnthropicResponse {
    content: Vec<serde_json::Value>,
}

fn parse_anthropic_response(body: AnthropicResponse, has_schema: bool) -> Result<Message> {
    if has_schema {
        for block in &body.content {
            if block.get("type").and_then(|v| v.as_str()) == Some("tool_use")
                && block.get("name").and_then(|v| v.as_str()) == Some("__structured_output")
            {
                if let Some(input) = block.get("input") {
                    let content = serde_json::to_string(input).map_err(|e| {
                        Error::internal(format!("failed to serialize structured output: {e}"))
                    })?;
                    return Ok(Message::assistant(content));
                }
            }
        }
    }

    let mut text_content = String::new();
    let mut tool_calls = Vec::new();

    for block in body.content {
        let block_type = block.get("type").and_then(|v| v.as_str());
        match block_type {
            Some("text") => {
                if let Some(text) = block.get("text").and_then(|v| v.as_str()) {
                    text_content.push_str(text);
                }
            }
            Some("tool_use") => {
                let id = block
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();
                let name = block
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();
                let input = block
                    .get("input")
                    .cloned()
                    .unwrap_or(serde_json::Value::Null);
                tool_calls.push(ToolCall { id, name, input });
            }
            _ => {}
        }
    }

    if !tool_calls.is_empty() {
        Ok(Message::assistant_with_tools(text_content, tool_calls))
    } else {
        Ok(Message::assistant(text_content))
    }
}

pub(crate) fn extract_anthropic_sse_delta(line: &str) -> Option<String> {
    let payload = line.strip_prefix("data:")?.trim();
    if payload.is_empty() {
        return None;
    }
    let val: serde_json::Value = serde_json::from_str(payload).ok()?;
    if val.get("type")?.as_str()? == "content_block_delta" {
        let delta = val.get("delta")?;
        if delta.get("type")?.as_str()? == "text_delta" {
            return delta.get("text")?.as_str().map(ToString::to_string);
        }
    }
    None
}
