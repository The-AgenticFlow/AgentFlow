// crates/agent-client/src/runner.rs
//
// AgentRunner — the tool-calling loop.
//
// Ties together AnthropicClient and McpSession into a single
// `run()` method that drives an agent to completion.

use anyhow::{anyhow, bail, Result};
use serde_json::Value;
use tracing::{debug, info, warn};

use crate::{
    anthropic::AnthropicClient,
    fallback::FallbackClient,
    gemini::GeminiClient,
    mcp::McpSession,
    openai::OpenAiClient,
    types::{AgentDecision, AgentPersona, LlmClient, LlmResponse, Message},
};

// ── AgentRunner ───────────────────────────────────────────────────────────

pub struct AgentRunner {
    client: Box<dyn LlmClient>,
    mcp: McpSession,
}

impl AgentRunner {
    pub fn new(client: Box<dyn LlmClient>, mcp: McpSession) -> Self {
        Self { client, mcp }
    }

    /// Create a runner using environment variables.
    /// Detects provider via LLM_PROVIDER (defaults to fallback for automatic failover).
    pub async fn from_env() -> Result<Self> {
        let provider = std::env::var("LLM_PROVIDER").unwrap_or_else(|_| "fallback".to_string());

        let client: Box<dyn LlmClient> = match provider.as_str() {
            "openai" => Box::new(OpenAiClient::from_env()?),
            "gemini" => Box::new(GeminiClient::from_env()?),
            "anthropic" => Box::new(AnthropicClient::from_env()?),
            "fallback" => Box::new(FallbackClient::from_env()?),
            other => bail!(
                "Unknown LLM_PROVIDER: {}. Valid options: anthropic, gemini, openai, fallback",
                other
            ),
        };

        info!(provider = %provider, model = %client.model(), "AgentRunner initialized from env");

        let mcp = McpSession::connect_default().await?;
        Ok(Self::new(client, mcp))
    }

    /// Create a runner for a specific agent, using its registry `model_backend`.
    ///
    /// When `model_backend` is provided, it overrides `ANTHROPIC_MODEL` for the
    /// proxy/anthropic provider so the correct model is sent to the proxy.
    pub async fn from_env_for_agent(model_backend: Option<&str>) -> Result<Self> {
        let provider = std::env::var("LLM_PROVIDER").unwrap_or_else(|_| "fallback".to_string());

        let client: Box<dyn LlmClient> = match provider.as_str() {
            "openai" => Box::new(OpenAiClient::from_env()?),
            "gemini" => Box::new(GeminiClient::from_env()?),
            "anthropic" => match model_backend {
                Some(m) => Box::new(AnthropicClient::from_env_with_model(m)?),
                None => Box::new(AnthropicClient::from_env()?),
            },
            "fallback" => match model_backend {
                Some(m) => Box::new(FallbackClient::from_env_with_model(m)?),
                None => Box::new(FallbackClient::from_env()?),
            },
            other => bail!(
                "Unknown LLM_PROVIDER: {}. Valid options: anthropic, gemini, openai, fallback",
                other
            ),
        };

        info!(provider = %provider, model = %client.model(), "AgentRunner initialized for agent");

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

    // 3. Fall back: scan for the last '{' and try to parse from there to the end
    // This handles cases where there's a conversational preamble.
    if let Some(last_brace) = text.rfind('{') {
        let potential_json = &text[last_brace..];
        // We might need to find the matching '}' if there's trailing junk,
        // but often LLMs just end with the JSON object.
        if let Ok(d) = serde_json::from_str::<AgentDecision>(potential_json.trim()) {
            return Ok(d);
        }
    }

    // 4. Line by line fallback (original logic)
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
    fn test_extract_decision_fails_gracefully() {
        let err = extract_decision("I cannot assist with that.").unwrap_err();
        assert!(err.to_string().contains("Could not extract"));
    }
}
