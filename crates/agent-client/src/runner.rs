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
    fallback::FallbackClient,
    mcp::McpSession,
    types::{AgentDecision, AgentPersona, LlmClient, LlmResponse, Message},
};

pub struct AgentRunner {
    client: Box<dyn LlmClient>,
    mcp: McpSession,
}

impl AgentRunner {
    pub fn new(client: Box<dyn LlmClient>, mcp: McpSession) -> Self {
        Self { client, mcp }
    }

    /// Create a runner using environment variables.
    /// Always uses FallbackClient for automatic failover.
    pub async fn from_env() -> Result<Self> {
        Self::from_env_for_agent(None).await
    }

    /// Create a runner for a specific agent, using its registry `model_backend`.
    ///
    /// When `model_backend` is provided, FallbackClient routes to the correct
    /// provider based on MODEL_PROVIDER_MAP. When PROXY_URL is set, individual
    /// API keys are optional - the proxy handles all routing.
    pub async fn from_env_for_agent(model_backend: Option<&str>) -> Result<Self> {
        let client: Box<dyn LlmClient> = match model_backend {
            Some(m) => Box::new(FallbackClient::from_env_with_model(m)?),
            None => Box::new(FallbackClient::from_env()?),
        };

        info!(model = %client.model(), "AgentRunner initialized");

        let mcp = McpSession::connect_default().await?;
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
        persona: &AgentPersona,
        context: Value,
        max_turns: usize,
    ) -> Result<AgentDecision> {
        // 1. Fetch current tool schemas from the MCP server
        let tools = self.mcp.list_tools().await?;
        info!(
            agent = persona.id,
            tools = tools.len(),
            "Agent runner starting"
        );

        // 2. Seed the conversation
        let mut messages = vec![
            Message::system(format!(
                "{}\n\nYou are an autonomous orchestrator. \
                 If the provided context is empty or sparse, use your tools (like `list_issues` or `search_issues`) \
                 to fetch the current state of the repository before making a final decision. \
                 \n\nYou MUST end your final response with a JSON object on its own line: \
                 {{\"action\": \"<action>\", \"notes\": \"<notes>\", \"assign_to\": \"<worker_id>\", \"ticket_id\": \"<ticket_id>\"}}",
                persona.system_prompt()
            )),
            Message::user(serde_json::to_string_pretty(&context)?),
        ];

        // 3. Tool-calling loop
        for turn in 0..max_turns {
            info!(agent = persona.id, turn, "--- LLM Turn Starting ---");

            match self.client.send(&messages, &tools).await? {
                LlmResponse::ToolCall { id, name, args } => {
                    info!(agent = persona.id, tool = name, args = ?args, "LLM requested tool execution");

                    // Execute the tool via MCP
                    let result = match self.mcp.call_tool(&name, args.clone()).await {
                        Ok(r) => {
                            let text = r.as_text();
                            info!(
                                agent = persona.id,
                                tool = name,
                                result = text,
                                "Tool execution successful"
                            );
                            text
                        }
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
                    info!(agent = persona.id, "--- Agent reached final decision ---");
                    debug!(decision = text, "Raw decision text from LLM");

                    // Extract the JSON decision from the last line of the response
                    let decision = extract_decision(&text)?;
                    info!(
                        action = decision.action,
                        notes = decision.notes,
                        "Parsed decision"
                    );
                    return Ok(decision);
                }
            }
        }

        Err(anyhow!(
            "Agent '{}' exceeded max_turns ({}) without returning a decision",
            persona.id,
            max_turns
        ))
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────

/// Extracts `{"action": ..., "notes": ...}` from the agent's final text.
/// The LLM may include reasoning before the JSON object, so we scan for it.
fn extract_decision(text: &str) -> Result<AgentDecision> {
    // 1. Try parsing the full text first (clean response)
    if let Ok(d) = serde_json::from_str::<AgentDecision>(text.trim()) {
        return Ok(d);
    }

    // 2. Try finding markdown JSON blocks: ```json ... ```
    if let Some(start) = text.find("```json") {
        let remainder = &text[start + 7..];
        if let Some(end) = remainder.find("```") {
            let json_str = remainder[..end].trim();
            if let Ok(d) = serde_json::from_str::<AgentDecision>(json_str) {
                return Ok(d);
            }
        }
    }

    // 3. Find the start of a JSON object. We prefer `{"` (JSON object start)
    //    over any stray '{' in reasoning text, then fall back to rfind.
    let json_start = text.find("{\"").or_else(|| text.rfind('{'));

    if let Some(start) = json_start {
        let potential_json = &text[start..];

        if let Ok(d) = serde_json::from_str::<AgentDecision>(potential_json.trim()) {
            return Ok(d);
        }

        // 3b. Truncated JSON repair: LLM responses can be cut off before the
        //     closing '"}' or '}'. Try appending common truncation suffixes.
        let trimmed = potential_json.trim();
        if trimmed.starts_with('{') {
            let suffixes = ["}", "\"}", "\"\n}", "\n}"];
            for suffix in suffixes {
                let repaired = format!("{}{}", trimmed, suffix);
                if let Ok(d) = serde_json::from_str::<AgentDecision>(&repaired) {
                    warn!("Repaired truncated JSON by appending suffix");
                    return Ok(d);
                }
            }
        }
    }

    // 4. Line by line fallback (original logic)
    for line in text.lines().rev() {
        let trimmed = line.trim();
        if trimmed.starts_with('{') {
            if let Ok(d) = serde_json::from_str::<AgentDecision>(trimmed) {
                return Ok(d);
            }
            let suffixes = ["}", "\"}", "\"\n}", "\n}"];
            for suffix in suffixes {
                let repaired = format!("{}{}", trimmed, suffix);
                if let Ok(d) = serde_json::from_str::<AgentDecision>(&repaired) {
                    warn!("Repaired truncated JSON on line by appending suffix");
                    return Ok(d);
                }
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
        assert_eq!(d.notes, "Assigned T-001 to forge-1");
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
    fn test_extract_decision_with_reasoning_preamble() {
        let text = concat!(
            "**Reasoning:** Some reasoning text here.\n\n",
            r#"{"action": "merge_prs", "notes": "PR #40 needs merge."}"#
        );
        let d = extract_decision(text).unwrap();
        assert_eq!(d.action, "merge_prs");
        assert_eq!(d.notes, "PR #40 needs merge.");
    }

    #[test]
    fn test_extract_decision_truncated_json_missing_quote_and_brace() {
        let text = concat!(
            "**Reasoning:** Some reasoning text here.\n\n",
            r#"{"action": "merge_prs", "notes": "PR #40 needs merge."#
        );
        let d = extract_decision(text).unwrap();
        assert_eq!(d.action, "merge_prs");
        assert_eq!(d.notes, "PR #40 needs merge.");
    }

    #[test]
    fn test_extract_decision_fails_gracefully() {
        let err = extract_decision("I cannot assist with that.").unwrap_err();
        assert!(err.to_string().contains("Could not extract"));
    }
}
