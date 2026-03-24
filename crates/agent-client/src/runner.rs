// crates/agent-client/src/runner.rs
//
// AgentRunner — the tool-calling loop.
//
// Ties together AnthropicClient and McpSession into a single
// `run()` method that drives an agent to completion.

use anyhow::{anyhow, Result};
use serde_json::Value;
use tracing::{debug, info, warn};

use crate::{
    anthropic::{AnthropicClient, LlmResponse},
    mcp::McpSession,
    types::{AgentDecision, AgentPersona, Message},
};

// ── AgentRunner ───────────────────────────────────────────────────────────

pub struct AgentRunner {
    client: AnthropicClient,
    mcp:    McpSession,
}

impl AgentRunner {
    pub fn new(client: AnthropicClient, mcp: McpSession) -> Self {
        Self { client, mcp }
    }

    /// Create a runner using environment variables (ANTHROPIC_API_KEY, ANTHROPIC_MODEL)
    /// and the default Docker-based GitHub MCP server.
    pub async fn from_env() -> Result<Self> {
        let client = AnthropicClient::from_env()?;
        let mcp    = McpSession::connect_default().await?;
        Ok(Self::new(client, mcp))
    }

    /// Run a single agent turn to completion.
    ///
    /// 1. Fetches available tool schemas from the MCP server.
    /// 2. Calls the Anthropic API with the persona's system prompt and context.
    /// 3. Executes tool calls via MCP until the LLM returns a final text response.
    /// 4. Parses the final response as `AgentDecision` JSON.
    ///
    /// The system prompt should instruct the LLM to return ONLY a JSON object:
    /// `{"action": "<action_string>", "notes": "<free_text>"}`
    pub async fn run(
        &mut self,
        persona:   &AgentPersona,
        context:   Value,
        max_turns: usize,
    ) -> Result<AgentDecision> {
        // 1. Fetch current tool schemas from the MCP server
        let tools = self.mcp.list_tools().await?;
        info!(agent = persona.id, tools = tools.len(), "Agent runner starting");

        // 2. Seed the conversation
        let mut messages = vec![
            Message::system(format!(
                "{}\n\nYou MUST end your response with a JSON object on its own line: \
                 {{\"action\": \"<action>\", \"notes\": \"<notes>\"}}",
                persona.system_prompt()
            )),
            Message::user(serde_json::to_string_pretty(&context)?),
        ];

        // 3. Tool-calling loop
        for turn in 0..max_turns {
            debug!(agent = persona.id, turn, "LLM turn");

            match self.client.send(&messages, &tools).await? {
                LlmResponse::ToolCall { id, name, args } => {
                    info!(agent = persona.id, tool = name, "Executing tool");

                    // Execute the tool via MCP
                    let result = match self.mcp.call_tool(&name, args.clone()).await {
                        Ok(r)  => r.as_text(),
                        Err(e) => {
                            warn!(agent = persona.id, tool = name, err = %e, "Tool call failed");
                            format!("ERROR: {}", e)
                        }
                    };

                    // Feed the result back into the conversation
                    messages.push(Message::assistant_tool_use(id.clone(), name, args));
                    messages.push(Message::tool_result(id, result));
                }

                LlmResponse::Text(text) => {
                    info!(agent = persona.id, turn, "Agent reached decision");
                    debug!(decision = text, "Raw decision text");

                    // Extract the JSON decision from the last line of the response
                    let decision = extract_decision(&text)?;
                    return Ok(decision);
                }
            }
        }

        Err(anyhow!(
            "Agent '{}' exceeded max_turns ({}) without returning a decision",
            persona.id, max_turns
        ))
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────

/// Extracts `{"action": ..., "notes": ...}` from the agent's final text.
/// The LLM may include reasoning before the JSON object, so we scan for it.
fn extract_decision(text: &str) -> Result<AgentDecision> {
    // Try parsing the full text first (clean response)
    if let Ok(d) = serde_json::from_str::<AgentDecision>(text.trim()) {
        return Ok(d);
    }

    // Fall back: find the last JSON object in the text
    for line in text.lines().rev() {
        let trimmed = line.trim();
        if trimmed.starts_with('{') {
            if let Ok(d) = serde_json::from_str::<AgentDecision>(trimmed) {
                return Ok(d);
            }
        }
    }

    Err(anyhow!(
        "Could not extract AgentDecision JSON from response:\n{}",
        text
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_decision_clean() {
        let text = r#"{"action": "work_assigned", "notes": "Assigned T-001 to forge-1"}"#;
        let d = extract_decision(text).unwrap();
        assert_eq!(d.action, "work_assigned");
        assert_eq!(d.notes,  "Assigned T-001 to forge-1");
    }

    #[test]
    fn test_extract_decision_with_preamble() {
        let text = concat!(
            "I analyzed the tickets and worker slots.\n",
            "forge-1 is idle and T-001 is unassigned.\n",
            r#"{"action": "work_assigned", "notes": "Assigned T-001 to forge-1"}"#
        );
        let d = extract_decision(text).unwrap();
        assert_eq!(d.action, "work_assigned");
    }

    #[test]
    fn test_extract_decision_fails_gracefully() {
        let err = extract_decision("I cannot assist with that.").unwrap_err();
        assert!(err.to_string().contains("Could not extract"));
    }
}
