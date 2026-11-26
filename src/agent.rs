//! OpenAI-compatible ReAct loop: the model chooses among the same core tools
//! that MCP clients get, and `logcut` executes them locally.

use std::path::Path;

use anyhow::{Context, Result, bail};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::core::{
    self,
    models::{SliceBuildArgs, SliceContainerRequest, SliceLogRequest, SliceServiceRequest},
};

const SYSTEM_PROMPT: &str = "You are a debugging assistant. Use logcut tools to inspect logs, \
stacktraces and compiler output before answering. Prefer slice_log for files and piped output, \
slice_build for compiler errors, slice_container for Docker, and slice_service for systemd journals. \
Reports are already redacted; never ask the user to paste secrets.";

fn tool(name: &str, description: &str, schema: schemars::Schema) -> Value {
    let parameters = serde_json::to_value(schema).expect("schema serializes");
    json!({
        "type": "function",
        "function": {
            "name": name,
            "description": description,
            "parameters": parameters,
        }
    })
}

/// JSON tool definitions generated from the core parameter structs.
pub fn tool_definitions() -> Vec<Value> {
    vec![
        tool(
            "slice_log",
            "Redact, deduplicate and slice a log file, piped text, or raw log string",
            schemars::schema_for!(SliceLogRequest),
        ),
        tool(
            "slice_build",
            "Collapse compiler error cascades into root causes with source context",
            schemars::schema_for!(SliceBuildArgs),
        ),
        tool(
            "slice_container",
            "Slice Docker container logs with the logcut engine",
            schemars::schema_for!(SliceContainerRequest),
        ),
        tool(
            "slice_service",
            "Slice a systemd unit's journal with the logcut engine",
            schemars::schema_for!(SliceServiceRequest),
        ),
    ]
}

#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<Choice>,
}

#[derive(Deserialize)]
struct Choice {
    message: AssistantMessage,
}

#[derive(Deserialize)]
struct AssistantMessage {
    content: Option<String>,
    tool_calls: Option<Vec<ToolCall>>,
}

#[derive(Deserialize, Serialize)]
struct ToolCall {
    id: String,
    function: FunctionCall,
}

#[derive(Deserialize, Serialize)]
struct FunctionCall {
    name: String,
    arguments: String,
}

async fn execute_tool(name: &str, arguments: &str, workspace_root: &Path) -> Result<String> {
    let output = match name {
        "slice_log" => {
            let mut request: SliceLogRequest = serde_json::from_str(arguments)?;
            if request.workspace_root.is_none() {
                request.workspace_root = Some(workspace_root.to_string_lossy().into_owned());
            }
            serde_json::to_string(&core::slice_log(request)?)?
        }
        "slice_build" => {
            let mut request: SliceBuildArgs = serde_json::from_str(arguments)?;
            if request.workspace_root.is_none() {
                request.workspace_root = Some(workspace_root.to_string_lossy().into_owned());
            }
            serde_json::to_string(&core::slice_build(request)?)?
        }
        "slice_container" => {
            let mut request: SliceContainerRequest = serde_json::from_str(arguments)?;
            if request.workspace_root.is_none() {
                request.workspace_root = Some(workspace_root.to_string_lossy().into_owned());
            }
            serde_json::to_string(&core::slice_container(request).await?)?
        }
        "slice_service" => {
            let mut request: SliceServiceRequest = serde_json::from_str(arguments)?;
            if request.workspace_root.is_none() {
                request.workspace_root = Some(workspace_root.to_string_lossy().into_owned());
            }
            serde_json::to_string(&core::slice_service(request).await?)?
        }
        other => bail!("unknown tool: {other}"),
    };
    Ok(output)
}

/// Run a bounded tool-using conversation and return the final answer.
pub async fn ask(question: &str, workspace_root: &Path) -> Result<String> {
    let api_key =
        std::env::var("OPENAI_API_KEY").context("OPENAI_API_KEY is required for `logcut ask`")?;
    let model = std::env::var("OPENAI_MODEL").unwrap_or_else(|_| "gpt-4o-mini".into());
    let base_url =
        std::env::var("OPENAI_BASE_URL").unwrap_or_else(|_| "https://api.openai.com/v1".into());
    let client = Client::new();

    let mut messages = vec![
        json!({"role": "system", "content": SYSTEM_PROMPT}),
        json!({"role": "user", "content": question}),
    ];

    for _ in 0..8 {
        let response = client
            .post(format!("{base_url}/chat/completions"))
            .bearer_auth(&api_key)
            .json(&json!({
                "model": model,
                "messages": messages,
                "tools": tool_definitions(),
                "tool_choice": "auto",
            }))
            .send()
            .await?
            .error_for_status()?
            .json::<ChatResponse>()
            .await?;

        let mut message = response
            .choices
            .into_iter()
            .next()
            .context("empty chat completion")?
            .message;
        let calls = message.tool_calls.take().unwrap_or_default();

        if calls.is_empty() {
            return Ok(message.content.unwrap_or_default());
        }

        messages.push(json!({
            "role": "assistant",
            "content": message.content,
            "tool_calls": calls,
        }));
        for call in calls {
            let output = execute_tool(
                &call.function.name,
                &call.function.arguments,
                workspace_root,
            )
            .await?;
            messages.push(json!({
                "role": "tool",
                "tool_call_id": call.id,
                "content": output,
            }));
        }
    }

    bail!("agent exceeded 8 tool rounds")
}
