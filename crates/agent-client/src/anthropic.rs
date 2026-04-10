// crates/agent-client/src/anthropic.rs
//
// AnthropicClient — calls the Anthropic Messages API with tool_use support.
//
// Uses the reqwest HTTP client. No SDK dependency needed — the API is simple
// enough to call directly and avoids version pinning issues.

use anyhow::{bail, Context, Result};
use reqwest::Client;

use serde_json::{json, Value};
use tracing::debug;

use crate::types::{ContentBlock, LlmClient, LlmResponse, Message, ToolSchema};

// ── Constants ─────────────────────────────────────────────────────────────

const DEFAULT_ANTHROPIC_API_URL: &str = "https://api.anthropic.com/v1/messages";
const ANTHROPIC_VERSION: &str = "2023-06-01";
const DEFAULT_MAX_TOKENS: u32 = 4096;

// ── Client ────────────────────────────────────────────────────────────────

pub struct AnthropicClient {
    http: Client,
    api_key: String,
    pub model: String,
    max_tokens: u32,
}

impl AnthropicClient {
    pub fn new(api_key: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            http: Client::new(),
            api_key: api_key.into(),
            model: model.into(),
            max_tokens: DEFAULT_MAX_TOKENS,
        }
    }

    /// Load API key from ANTHROPIC_API_KEY env var.
    /// Uses claude-3-5-haiku by default (fast + cheap for orchestration).
    pub fn from_env() -> Result<Self> {
        let key = std::env::var("ANTHROPIC_API_KEY").context("ANTHROPIC_API_KEY not set")?;
        let model = std::env::var("ANTHROPIC_MODEL")
            .unwrap_or_else(|_| "claude-3-5-haiku-20241022".to_string());
        Ok(Self::new(key, model))
    }

    pub fn with_max_tokens(mut self, n: u32) -> Self {
        self.max_tokens = n;
        self
    }
}

// ── Serialization helpers ─────────────────────────────────────────────────

/// Converts our `Message` enum into the raw JSON format Anthropic expects.
fn messages_to_json(messages: &[Message]) -> Value {
    let mut system_prompt = String::new();
    let mut turns: Vec<Value> = Vec::new();

    for msg in messages {
        match msg {
            Message::System { content } => {
                system_prompt = content.clone();
            }
            Message::User { content } => {
                turns.push(json!({ "role": "user", "content": content }));
            }
            Message::Assistant { content } => {
                let blocks: Vec<Value> = content
                    .iter()
                    .map(|b| match b {
                        ContentBlock::Text { text } => json!({ "type": "text", "text": text }),
                        ContentBlock::ToolUse { id, name, input } => json!({
                            "type":  "tool_use",
                            "id":    id,
                            "name":  name,
                            "input": input,
                        }),
                    })
                    .collect();
                turns.push(json!({ "role": "assistant", "content": blocks }));
            }
            Message::ToolResult {
                tool_use_id,
                content,
            } => {
                turns.push(json!({
                    "role": "user",
                    "content": [{
                        "type":        "tool_result",
                        "tool_use_id": tool_use_id,
                        "content":     content,
                    }]
                }));
            }
        }
    }

    json!({ "system": system_prompt, "messages": turns })
}

// ── Main API call ─────────────────────────────────────────────────────────

#[async_trait::async_trait]
impl LlmClient for AnthropicClient {
    /// Send messages to the API. Returns exactly one `LlmResponse`.
    async fn send(&self, messages: &[Message], tools: &[ToolSchema]) -> Result<LlmResponse> {
        let msg_json = messages_to_json(messages);
        let tools_json: Vec<Value> = tools
            .iter()
            .map(|t| {
                json!({
                    "name":         t.name,
                    "description":  t.description,
                    "input_schema": t.input_schema,
                })
            })
            .collect();

        let body = json!({
            "model":      self.model,
            "max_tokens": self.max_tokens,
            "system":     msg_json["system"],
            "messages":   msg_json["messages"],
            "tools":      tools_json,
        });

        let api_url = std::env::var("ANTHROPIC_API_URL")
            .unwrap_or_else(|_| DEFAULT_ANTHROPIC_API_URL.to_string());

        let resp = self
            .http
            .post(api_url)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .context("HTTP request to Anthropic API failed")?;

        let status = resp.status();
        let raw: Value = resp
            .json()
            .await
            .context("Failed to parse Anthropic response")?;

        if !status.is_success() {
            bail!(
                "Anthropic API error {}: {}",
                status,
                raw["error"]["message"].as_str().unwrap_or("unknown")
            );
        }

        debug!(
            stop_reason = raw["stop_reason"].as_str(),
            "← Anthropic response"
        );

        // Parse the first content block
        let content = &raw["content"];
        let block = content.get(0).context("Anthropic returned empty content")?;

        match block["type"].as_str() {
            Some("tool_use") => Ok(LlmResponse::ToolCall {
                id: block["id"].as_str().unwrap_or("").to_string(),
                name: block["name"].as_str().unwrap_or("").to_string(),
                args: block["input"].clone(),
            }),
            Some("text") => Ok(LlmResponse::Text(
                block["text"].as_str().unwrap_or("").to_string(),
            )),
            other => bail!("Unknown Anthropic content block type: {:?}", other),
        }
    }

    fn model(&self) -> &str {
        &self.model
    }
}
