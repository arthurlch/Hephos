//! Model providers.
//!
//! Each provider is a thin `reqwest` client implementing [`crate::Model`]. The
//! API key is read from the environment at construction. Nothing here is clever:
//! build a request, send it, map the response into a [`crate::Message`] or a
//! [`crate::TextStream`].

use futures::StreamExt;
use hephos_core::{Error, Result};
use serde::{Deserialize, Serialize};

use crate::message::{Message, Role, TextStream};
use crate::model::{Completion, Model};
use crate::tool::ToolCall;

/// Anthropic Messages API client.
pub struct Anthropic {
    api_key: String,
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
    pub fn new(api_key: impl Into<String>, model: impl Into<String>) -> Self {
        Anthropic {
            api_key: api_key.into(),
            model: model.into(),
            base_url: "https://api.anthropic.com".into(),
            http: reqwest::Client::new(),
        }
    }

    /// Override the base URL (e.g. for testing against a mock server or proxy).
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
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
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&payload)
            .send()
            .await
            .map_err(|e| Error::internal(format!("anthropic request failed: {e}")))?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
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
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&payload)
            .send()
            .await
            .map_err(|e| Error::internal(format!("anthropic stream request failed: {e}")))?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(Error::internal(format!(
                "anthropic API returned status {status}: {body}"
            )));
        }

        Ok(parse_sse_stream(resp, extract_anthropic_sse_delta))
    }
}

/// OpenAI Chat Completions API client.
pub struct OpenAI {
    api_key: String,
    model: String,
    base_url: String,
    http: reqwest::Client,
}

impl OpenAI {
    /// Construct from `OPENAI_API_KEY` (and optional `OPENAI_BASE_URL`).
    pub fn from_env(model: impl Into<String>) -> Result<Self> {
        let api_key = std::env::var("OPENAI_API_KEY")
            .map_err(|_| Error::invalid("OPENAI_API_KEY is not set"))?;
        let base_url =
            std::env::var("OPENAI_BASE_URL").unwrap_or_else(|_| "https://api.openai.com/v1".into());
        Ok(Self::new(api_key, model).with_base_url(base_url))
    }

    /// Construct with an explicit API key and model name.
    pub fn new(api_key: impl Into<String>, model: impl Into<String>) -> Self {
        OpenAI {
            api_key: api_key.into(),
            model: model.into(),
            base_url: "https://api.openai.com/v1".into(),
            http: reqwest::Client::new(),
        }
    }

    /// Override the base URL (e.g. for testing or local LLM gateways).
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }
}

impl Model for OpenAI {
    async fn complete(&self, request: Completion) -> Result<Message> {
        let url = format!("{}/chat/completions", self.base_url.trim_end_matches('/'));
        let payload = build_openai_payload(&self.model, &request, false)?;

        let resp = self
            .http
            .post(&url)
            .header("authorization", format!("Bearer {}", self.api_key))
            .header("content-type", "application/json")
            .json(&payload)
            .send()
            .await
            .map_err(|e| Error::internal(format!("openai request failed: {e}")))?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(Error::internal(format!(
                "openai API returned status {status}: {body}"
            )));
        }

        let body: OpenAiResponse = resp
            .json()
            .await
            .map_err(|e| Error::internal(format!("failed to parse openai response: {e}")))?;

        parse_openai_response(body)
    }

    async fn stream(&self, request: Completion) -> Result<TextStream> {
        let url = format!("{}/chat/completions", self.base_url.trim_end_matches('/'));
        let payload = build_openai_payload(&self.model, &request, true)?;

        let resp = self
            .http
            .post(&url)
            .header("authorization", format!("Bearer {}", self.api_key))
            .header("content-type", "application/json")
            .json(&payload)
            .send()
            .await
            .map_err(|e| Error::internal(format!("openai stream request failed: {e}")))?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(Error::internal(format!(
                "openai API returned status {status}: {body}"
            )));
        }

        Ok(parse_sse_stream(resp, extract_openai_sse_delta))
    }
}

// ---------------- Anthropic serialization helpers ----------------

#[derive(Serialize)]
struct AnthropicPayload<'a> {
    model: &'a str,
    max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<&'a str>,
    messages: Vec<AnthropicMessagePayload<'a>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<AnthropicToolPayload<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    stream: bool,
}

#[derive(Serialize)]
struct AnthropicMessagePayload<'a> {
    role: &'a str,
    content: AnthropicContentPayload<'a>,
}

#[derive(Serialize)]
#[serde(untagged)]
enum AnthropicContentPayload<'a> {
    Text(&'a str),
    Blocks(Vec<serde_json::Value>),
}

#[derive(Serialize)]
struct AnthropicToolPayload<'a> {
    name: &'a str,
    description: &'a str,
    input_schema: &'a serde_json::Value,
}

fn build_anthropic_payload<'a>(
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

fn extract_anthropic_sse_delta(line: &str) -> Option<String> {
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

// ---------------- OpenAI serialization helpers ----------------

#[derive(Serialize)]
struct OpenAiPayload<'a> {
    model: &'a str,
    messages: Vec<serde_json::Value>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    response_format: Option<serde_json::Value>,
    max_tokens: u32,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    stream: bool,
}

fn build_openai_payload<'a>(
    model: &'a str,
    request: &'a Completion,
    stream: bool,
) -> Result<OpenAiPayload<'a>> {
    let mut messages = Vec::new();
    if !request.system.trim().is_empty() {
        messages.push(serde_json::json!({
            "role": "system",
            "content": request.system
        }));
    }

    for msg in &request.messages {
        match msg.role {
            Role::User => {
                messages.push(serde_json::json!({
                    "role": "user",
                    "content": msg.content
                }));
            }
            Role::Assistant => {
                if msg.tool_calls.is_empty() {
                    messages.push(serde_json::json!({
                        "role": "assistant",
                        "content": msg.content
                    }));
                } else {
                    let calls: Vec<serde_json::Value> = msg
                        .tool_calls
                        .iter()
                        .map(|c| {
                            serde_json::json!({
                                "id": c.id,
                                "type": "function",
                                "function": {
                                    "name": c.name,
                                    "arguments": serde_json::to_string(&c.input).unwrap_or_default()
                                }
                            })
                        })
                        .collect();
                    let mut obj = serde_json::json!({
                        "role": "assistant",
                        "tool_calls": calls
                    });
                    if !msg.content.is_empty() {
                        obj["content"] = serde_json::json!(msg.content);
                    }
                    messages.push(obj);
                }
            }
            Role::Tool => {
                messages.push(serde_json::json!({
                    "role": "tool",
                    "tool_call_id": msg.tool_call_id.as_deref().unwrap_or(""),
                    "content": msg.content
                }));
            }
            Role::System => {
                messages.push(serde_json::json!({
                    "role": "system",
                    "content": msg.content
                }));
            }
        }
    }

    let mut tools = Vec::new();
    for t in &request.tools {
        tools.push(serde_json::json!({
            "type": "function",
            "function": {
                "name": t.name,
                "description": t.description,
                "parameters": t.input_schema
            }
        }));
    }

    let response_format = request.response_schema.as_ref().map(|schema| {
        serde_json::json!({
            "type": "json_schema",
            "json_schema": {
                "name": "response",
                "strict": true,
                "schema": schema
            }
        })
    });

    Ok(OpenAiPayload {
        model,
        messages,
        tools,
        response_format,
        max_tokens: request.max_tokens,
        stream,
    })
}

#[derive(Deserialize)]
struct OpenAiResponse {
    choices: Vec<OpenAiChoice>,
}

#[derive(Deserialize)]
struct OpenAiChoice {
    message: OpenAiResponseMessage,
}

#[derive(Deserialize)]
struct OpenAiResponseMessage {
    content: Option<String>,
    #[serde(default)]
    tool_calls: Vec<OpenAiToolCallResponse>,
}

#[derive(Deserialize)]
struct OpenAiToolCallResponse {
    #[serde(default)]
    id: String,
    function: OpenAiFunctionResponse,
}

#[derive(Deserialize)]
struct OpenAiFunctionResponse {
    name: String,
    arguments: String,
}

fn parse_openai_response(body: OpenAiResponse) -> Result<Message> {
    let first = body
        .choices
        .into_iter()
        .next()
        .ok_or_else(|| Error::internal("openai response contained no choices"))?;

    let text_content = first.message.content.unwrap_or_default();
    let mut tool_calls = Vec::new();

    for tc in first.message.tool_calls {
        let input: serde_json::Value = serde_json::from_str(&tc.function.arguments)
            .map_err(|e| Error::internal(format!("invalid json in tool arguments: {e}")))?;
        tool_calls.push(ToolCall {
            id: tc.id,
            name: tc.function.name,
            input,
        });
    }

    if !tool_calls.is_empty() {
        Ok(Message::assistant_with_tools(text_content, tool_calls))
    } else {
        Ok(Message::assistant(text_content))
    }
}

fn extract_openai_sse_delta(line: &str) -> Option<String> {
    let payload = line.strip_prefix("data:")?.trim();
    if payload.is_empty() || payload == "[DONE]" {
        return None;
    }
    let val: serde_json::Value = serde_json::from_str(payload).ok()?;
    val.get("choices")?
        .get(0)?
        .get("delta")?
        .get("content")?
        .as_str()
        .map(ToString::to_string)
}

// ---------------- Generic SSE parser ----------------

fn parse_sse_stream<F>(response: reqwest::Response, extractor: F) -> TextStream
where
    F: Fn(&str) -> Option<String> + Send + Sync + 'static,
{
    let stream = async_stream::try_stream! {
        let mut byte_stream = response.bytes_stream();
        let mut buffer = String::new();

        while let Some(chunk_res) = byte_stream.next().await {
            let chunk = chunk_res.map_err(|e| Error::internal(format!("network error in stream: {e}")))?;
            let text = std::str::from_utf8(&chunk)
                .map_err(|e| Error::internal(format!("invalid utf-8 in sse stream: {e}")))?;
            buffer.push_str(text);

            while let Some(pos) = buffer.find('\n') {
                let line = buffer[..pos].trim_end_matches('\r').to_string();
                buffer.drain(..=pos);

                if let Some(delta) = extractor(&line) {
                    if !delta.is_empty() {
                        yield delta;
                    }
                }
            }
        }
    };
    Box::pin(stream)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tool::ToolSchema;

    #[test]
    fn anthropic_sse_delta_extraction() {
        let line = r#"data: {"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": "hello "}}"#;
        assert_eq!(extract_anthropic_sse_delta(line), Some("hello ".into()));

        let irrelevant = r#"data: {"type": "message_start"}"#;
        assert_eq!(extract_anthropic_sse_delta(irrelevant), None);
    }

    #[test]
    fn openai_sse_delta_extraction() {
        let line = r#"data: {"id":"1","choices":[{"index":0,"delta":{"content":"world"},"finish_reason":null}]}"#;
        assert_eq!(extract_openai_sse_delta(line), Some("world".into()));

        let done = "data: [DONE]";
        assert_eq!(extract_openai_sse_delta(done), None);
    }

    #[test]
    fn anthropic_payload_structure() {
        let completion = Completion {
            system: "helpful assistant".into(),
            messages: vec![
                Message::user("hi"),
                Message::assistant_with_tools(
                    "calling",
                    vec![ToolCall::with_id(
                        "t1",
                        "search",
                        serde_json::json!({"q": "test"}),
                    )],
                ),
                Message::tool_result("t1", "result"),
            ],
            tools: vec![ToolSchema::new(
                "search",
                "search",
                serde_json::json!({"type": "object"}),
            )],
            max_tokens: 1024,
            response_schema: None,
        };

        let payload = build_anthropic_payload("claude-3-5-sonnet", &completion, false).unwrap();
        assert_eq!(payload.model, "claude-3-5-sonnet");
        assert_eq!(payload.system, Some("helpful assistant"));
        assert_eq!(payload.messages.len(), 3);
        assert_eq!(payload.tools.len(), 1);
    }

    #[test]
    fn openai_payload_structure() {
        let completion = Completion {
            system: "system prompt".into(),
            messages: vec![Message::user("hello")],
            tools: vec![],
            max_tokens: 512,
            response_schema: Some(serde_json::json!({"type": "object"})),
        };

        let payload = build_openai_payload("gpt-4o", &completion, false).unwrap();
        assert_eq!(payload.model, "gpt-4o");
        assert!(payload.response_format.is_some());
        assert_eq!(payload.messages.len(), 2); // system + user
    }
}
