use std::sync::Arc;

use crate::RegisteredMcpTool;
use hephos_agent::{ToolDefinition, ToolSchema, Tools};
use hephos_core::{Error, Result};
use rmcp::model::CallToolRequestParam;
use rmcp::service::{RoleClient, RunningService};
use serde::Serialize;
use serde::de::DeserializeOwned;

/// An MCP client that connects to an external MCP server to list and call tools.
#[derive(Clone)]
pub struct McpClient {
    service: Arc<RunningService<RoleClient, ()>>,
}

impl std::fmt::Debug for McpClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("McpClient").finish_non_exhaustive()
    }
}

impl McpClient {
    /// Connect to an MCP server over any transport implementing [`rmcp::transport::IntoTransport`].
    pub async fn connect<T, E, A>(transport: T) -> Result<Self>
    where
        T: rmcp::transport::IntoTransport<RoleClient, E, A>,
        E: std::error::Error + Send + Sync + 'static,
    {
        let service = rmcp::service::serve_client((), transport)
            .await
            .map_err(|e| Error::internal(format!("failed to connect MCP client: {e}")))?;
        Ok(McpClient {
            service: Arc::new(service),
        })
    }

    /// List tools provided by the connected MCP server.
    pub async fn list_tools(&self) -> Result<Vec<rmcp::model::Tool>> {
        let res = self
            .service
            .list_tools(None)
            .await
            .map_err(|e| Error::internal(format!("MCP list_tools failed: {e}")))?;
        Ok(res.tools)
    }

    /// Call an MCP tool by name with arbitrary arguments, returning JSON.
    pub async fn call_tool(
        &self,
        name: impl Into<String>,
        arguments: serde_json::Value,
    ) -> Result<serde_json::Value> {
        let args = match arguments {
            serde_json::Value::Object(map) => Some(map),
            serde_json::Value::Null => None,
            other => {
                return Err(Error::invalid(format!(
                    "tool arguments must be a JSON object, got: {other}"
                )));
            }
        };

        let param = CallToolRequestParam {
            name: name.into().into(),
            arguments: args,
        };

        let result = self
            .service
            .call_tool(param)
            .await
            .map_err(|e| Error::internal(format!("MCP call_tool failed: {e}")))?;

        if result.is_error == Some(true) {
            let msg = result
                .content
                .iter()
                .filter_map(|c| c.raw.as_text().map(|t| t.text.as_str()))
                .collect::<Vec<_>>()
                .join("\n");
            return Err(Error::invalid(format!("tool error: {msg}")));
        }

        if let Some(structured) = result.structured_content {
            Ok(structured)
        } else if let Some(t) = result.content.first().and_then(|c| c.raw.as_text()) {
            match serde_json::from_str(&t.text) {
                Ok(val) => Ok(val),
                Err(_) => Ok(serde_json::Value::String(t.text.clone())),
            }
        } else {
            Ok(serde_json::Value::Null)
        }
    }

    /// Call a tool with strongly typed input and output.
    pub async fn call_typed<I: Serialize, O: DeserializeOwned>(
        &self,
        name: &str,
        input: I,
    ) -> Result<O> {
        let args = serde_json::to_value(input)
            .map_err(|e| Error::invalid(format!("failed to serialize input: {e}")))?;
        let output = self.call_tool(name, args).await?;
        serde_json::from_value(output)
            .map_err(|e| Error::internal(format!("failed to deserialize output: {e}")))
    }

    /// Convert all tools provided by this MCP server into a [`hephos_agent::Tools`]
    /// collection, allowing an [`hephos_agent::Agent`] to consume them directly.
    pub async fn to_tools(&self) -> Result<Tools> {
        let remote_tools = self.list_tools().await?;
        let mut tools = Tools::new();
        for tool in remote_tools {
            let name = tool.name.to_string();
            let description = tool.description.unwrap_or_default().to_string();
            let input_schema = serde_json::Value::Object((*tool.input_schema).clone());
            let schema = ToolSchema::new(name.clone(), description, input_schema);

            let client = self.clone();
            let tool_name = name.clone();
            let def = ToolDefinition::dynamic(schema, move |args| {
                let client = client.clone();
                let tool_name = tool_name.clone();
                Box::pin(async move { client.call_tool(&tool_name, args).await })
            });
            tools.register_definition(def);
        }
        Ok(tools)
    }

    /// Convert a single named tool from this MCP server into an agent [`RegisteredMcpTool`].
    pub async fn to_tool(&self, name: &str) -> Result<RegisteredMcpTool> {
        let remote_tools = self.list_tools().await?;
        let tool = remote_tools
            .into_iter()
            .find(|t| t.name == name)
            .ok_or_else(|| Error::not_found(format!("MCP tool '{name}'")))?;

        let tool_name = tool.name.to_string();
        let description = tool.description.unwrap_or_default().to_string();
        let input_schema = serde_json::Value::Object((*tool.input_schema).clone());
        let schema = ToolSchema::new(tool_name.clone(), description, input_schema);

        let client = self.clone();
        let def = ToolDefinition::dynamic(schema, move |args| {
            let client = client.clone();
            let tool_name = tool_name.clone();
            Box::pin(async move { client.call_tool(&tool_name, args).await })
        });
        Ok(def)
    }
}
