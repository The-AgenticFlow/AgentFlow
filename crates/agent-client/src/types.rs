//
// Core types shared across all agent-client modules.

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

// ── Agent Persona ─────────────────────────────────────────────────────────

/// Loaded from a `.agent.md` YAML frontmatter block.
#[derive(Debug, Clone)]
pub struct AgentPersona {
    pub id: String,
    pub role: String,
    /// Full system prompt injected into the LLM.
    pub system_prompt: String,
}

impl AgentPersona {
    pub fn system_prompt(&self) -> &str {
        &self.system_prompt
    }
}

// ── Conversation Messages ─────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "role", rename_all = "lowercase")]
pub enum Message {
    System {
        content: String,
    },
    User {
        content: String,
    },
    /// The assistant asked for a tool to be called.
    Assistant {
        content: Vec<ContentBlock>,
    },
    /// The result of executing a tool call.
    ToolResult {
        tool_use_id: String,
        content: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Text {
        text: String,
    },
    ToolUse {
        id: String,
        name: String,
        input: Value,
    },
}

impl Message {
    pub fn system(text: impl Into<String>) -> Self {
        Message::System {
            content: text.into(),
        }
    }

    pub fn user(text: impl Into<String>) -> Self {
        Message::User {
            content: text.into(),
        }
    }

    pub fn tool_result(id: impl Into<String>, content: impl Into<String>) -> Self {
        Message::ToolResult {
            tool_use_id: id.into(),
            content: content.into(),
        }
    }

    pub fn assistant_tool_use(
        id: impl Into<String>,
        name: impl Into<String>,
        input: Value,
    ) -> Self {
        Message::Assistant {
            content: vec![ContentBlock::ToolUse {
                id: id.into(),
                name: name.into(),
                input,
            }],
        }
    }
}

// ── Tool Schema ───────────────────────────────────────────────────────────

/// Matches Anthropic's `tools` array format.
/// Populated by listing MCP server tools, then forwarded to the API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSchema {
    pub name: String,
    pub description: String,
    pub input_schema: Value, // JSON Schema object
}

/// The raw result returned by an MCP tool call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub content: Vec<ToolResultContent>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ToolResultContent {
    Text { text: String },
}

impl ToolResult {
    pub fn as_text(&self) -> String {
        self.content
            .iter()
            .filter_map(|c| match c {
                ToolResultContent::Text { text } => Some(text.as_str()),
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

// ── Agent Decision ────────────────────────────────────────────────────────

/// The final output of the tool-calling loop.
/// The LLM must return a JSON object matching this schema as its final text response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentDecision {
    /// The PocketFlow Action to return (e.g. "work_assigned", "no_work").
    pub action: String,
    /// Human-readable notes for logging or downstream agents (e.g. SENTINEL context).
    pub notes: String,

    /// Optional metadata for specific actions
    pub assign_to: Option<String>,
    pub ticket_id: Option<String>,
    pub issue_url: Option<String>,
}

// ── LLM Client Trait ──────────────────────────────────────────────────────

#[async_trait]
pub trait LlmClient: Send + Sync {
    /// Send messages to the API. Returns exactly one `LlmResponse`.
    async fn send(&self, messages: &[Message], tools: &[ToolSchema]) -> Result<LlmResponse>;

    /// Return the model name (for logging).
    fn model(&self) -> &str;
}

/// Parsed output of a single Messages API call.
pub enum LlmResponse {
    /// The model wants to call a tool.
    ToolCall {
        id: String,
        name: String,
        args: Value,
    },
    /// The model is done and returned a text response (final decision).
    Text(String),
}
