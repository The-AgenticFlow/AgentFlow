// crates/agent-client/src/openai.rs
//
// OpenAiClient — calls the OpenAI Chat Completions API with tool support.
//
// Compatible with any OpenAI-compatible proxy (e.g. DeepSeek, OpenRouter, etc)
// via the OPENAI_API_URL environment variable.

use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use reqwest::Client;
use serde_json::{json, Value};
use tracing::debug;

use crate::types::{ContentBlock, LlmClient, LlmResponse, Message, ToolSchema};

// ── Constants ─────────────────────────────────────────────────────────────

const DEFAULT_OPENAI_API_URL: &str = "https://api.openai.com/v1/chat/completions";
const DEFAULT_MAX_TOKENS: u32 = 4096;

// ── Client ────────────────────────────────────────────────────────────────

pub struct OpenAiClient {
    http: Client,
    api_key: String,
    pub model: String,
    max_tokens: u32,
}

impl OpenAiClient {
    pub fn new(api_key: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            http: Client::new(),
            api_key: api_key.into(),
            model: model.into(),
            max_tokens: DEFAULT_MAX_TOKENS,
        }
    }

    /// Load API key from OPENAI_API_KEY env var.
    /// Uses gpt-4o-mini by default (fast + cheap for orchestration).
    pub fn from_env() -> Result<Self> {
        let key = std::env::var("OPENAI_API_KEY").context("OPENAI_API_KEY not set")?;
        let model = std::env::var("OPENAI_MODEL").unwrap_or_else(|_| "gpt-4o-mini".to_string());
        Ok(Self::new(key, model))
    }

    pub fn with_max_tokens(mut self, n: u32) -> Self {
        self.max_tokens = n;
        self
    }
}

// ── Serialization helpers ─────────────────────────────────────────────────

/// Converts our `Message` enum into the raw JSON format OpenAI expects.
fn messages_to_json(messages: &[Message]) -> Value {
    let mut turns: Vec<Value> = Vec::new();

    for msg in messages {
        match msg {
            Message::System { content } => {
                turns.push(json!({ "role": "system", "content": content }));
            }
            Message::User { content } => {
                turns.push(json!({ "role": "user", "content": content }));
            }
            Message::Assistant { content } => {
                let mut text_content = String::new();
                let mut tool_calls = Vec::new();

                for block in content {
                    match block {
                        ContentBlock::Text { text } => {
                            if !text_content.is_empty() {
                                text_content.push('\n');
                            }
                            text_content.push_str(text);
                        }
                        ContentBlock::ToolUse { id, name, input } => {
                            tool_calls.push(json!({
                                "id":   id,
                                "type": "function",
                                "function": {
                                    "name":      name,
                                    "arguments": input.to_string(),
                                }
                            }));
                        }
                    }
                }

                let mut assistant_msg = json!({ "role": "assistant" });
                if !text_content.is_empty() {
                    assistant_msg["content"] = json!(text_content);
                } else {
                    assistant_msg["content"] = Value::Null;
                }

                if !tool_calls.is_empty() {
                    assistant_msg["tool_calls"] = json!(tool_calls);
                }

                turns.push(assistant_msg);
            }
            Message::ToolResult {
                tool_use_id,
                content,
            } => {
                turns.push(json!({
                    "role":         "tool",
                    "tool_call_id": tool_use_id,
                    "content":      content,
                }));
            }
        }
    }

    json!(turns)
}

// ── Main API call ─────────────────────────────────────────────────────────

#[async_trait]
impl LlmClient for OpenAiClient {
    async fn send(&self, messages: &[Message], tools: &[ToolSchema]) -> Result<LlmResponse> {
        let messages_json = messages_to_json(messages);

        let mut body = json!({
            "model":      self.model,
            "max_tokens": self.max_tokens,
            "messages":   messages_json,
        });

        if !tools.is_empty() {
            let tools_json: Vec<Value> = tools
                .iter()
                .map(|t| {
                    json!({
                        "type": "function",
                        "function": {
                            "name":        t.name,
                            "description": t.description,
                            "parameters":  t.input_schema,
                        }
                    })
                })
                .collect();
            body["tools"] = json!(tools_json);
        }

        let api_url =
            std::env::var("OPENAI_API_URL").unwrap_or_else(|_| DEFAULT_OPENAI_API_URL.to_string());

        let resp = self
            .http
            .post(api_url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .context("HTTP request to OpenAI API failed")?;

        let status = resp.status();
        let raw: Value = resp
            .json()
            .await
            .context("Failed to parse OpenAI response")?;

        if !status.is_success() {
            let error_msg = raw["error"]["message"].as_str().unwrap_or("unknown");
            bail!("OpenAI API error {}: {}", status, error_msg);
        }

        debug!(model = %self.model, "← OpenAI response");

        let choice = raw["choices"]
            .get(0)
            .context("OpenAI returned no choices")?;
        let message = &choice["message"];

        if let Some(tool_calls) = message["tool_calls"].as_array() {
            if let Some(tool_call) = tool_calls.first() {
                let id = tool_call["id"].as_str().unwrap_or("").to_string();
                let name = tool_call["function"]["name"]
                    .as_str()
                    .unwrap_or("")
                    .to_string();
                let args_str = tool_call["function"]["arguments"].as_str().unwrap_or("{}");
                let args: Value = serde_json::from_str(args_str).unwrap_or(json!({}));

                return Ok(LlmResponse::ToolCall { id, name, args });
            }
        }

        let text = message["content"].as_str().unwrap_or("").to_string();
        Ok(LlmResponse::Text(text))
    }

    fn model(&self) -> &str {
        &self.model
    }
}
