//! OpenAI Chat Completions provider implementation.

use hephos_core::security::SecretString;
use hephos_core::{Error, Result};
use serde::{Deserialize, Serialize};

use super::common::{default_http_client, parse_sse_stream};
use crate::message::{Message, Role, TextStream};
use crate::model::{Completion, Model};
use crate::tool::ToolCall;

/// OpenAI Chat Completions API client.
#[derive(Debug)]
pub struct OpenAI {
    api_key: SecretString,
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
    pub fn new(api_key: impl Into<SecretString>, model: impl Into<String>) -> Self {
        OpenAI {
            api_key: api_key.into(),
            model: model.into(),
            base_url: "https://api.openai.com/v1".into(),
            http: default_http_client(),
        }
    }

    /// Override the base URL (e.g. for testing or local LLM gateways).
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

impl Model for OpenAI {
    async fn complete(&self, request: Completion) -> Result<Message> {
        let url = format!("{}/chat/completions", self.base_url.trim_end_matches('/'));
        let payload = build_openai_payload(&self.model, &request, false)?;

        let resp = self
            .http
            .post(&url)
            .header(
                "authorization",
                format!("Bearer {}", self.api_key.expose_secret()),
            )
            .header("content-type", "application/json")
            .json(&payload)
            .send()
            .await
            .map_err(|e| Error::internal(format!("openai request failed: {e}")))?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                return Err(Error::rate_limited(format!("openai rate limit: {body}")));
            }
            if status == reqwest::StatusCode::SERVICE_UNAVAILABLE {
                return Err(Error::unavailable(format!("openai unavailable: {body}")));
            }
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
            .header(
                "authorization",
                format!("Bearer {}", self.api_key.expose_secret()),
            )
            .header("content-type", "application/json")
            .json(&payload)
            .send()
            .await
            .map_err(|e| Error::internal(format!("openai stream request failed: {e}")))?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                return Err(Error::rate_limited(format!("openai rate limit: {body}")));
            }
            if status == reqwest::StatusCode::SERVICE_UNAVAILABLE {
                return Err(Error::unavailable(format!("openai unavailable: {body}")));
            }
            return Err(Error::internal(format!(
                "openai API returned status {status}: {body}"
            )));
        }

        Ok(parse_sse_stream(resp, extract_openai_sse_delta))
    }
}

#[derive(Serialize)]
pub(crate) struct OpenAiPayload<'a> {
    pub(crate) model: &'a str,
    pub(crate) messages: Vec<serde_json::Value>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) tools: Vec<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) response_format: Option<serde_json::Value>,
    pub(crate) max_tokens: u32,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub(crate) stream: bool,
}

pub(crate) fn build_openai_payload<'a>(
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

pub(crate) fn extract_openai_sse_delta(line: &str) -> Option<String> {
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
