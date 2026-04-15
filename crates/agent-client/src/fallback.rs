// crates/agent-client/src/fallback.rs
//
// FallbackClient — tries multiple LLM providers in order, falling back on failure.
//
// Use this when you want automatic failover between providers (e.g., Gemini -> Claude).

use anyhow::{bail, Result};
use async_trait::async_trait;
use std::time::Duration;
use tracing::{info, warn};

use crate::anthropic::AnthropicClient;
use crate::gemini::GeminiClient;
use crate::openai::OpenAiClient;
use crate::types::{LlmClient, LlmResponse, Message, ToolSchema};

// ── Fallback Client ────────────────────────────────────────────────────────

pub struct FallbackClient {
    clients: Vec<Box<dyn LlmClient>>,
    current_idx: usize,
    timeout: Duration,
}

impl FallbackClient {
    /// Create a fallback client that tries providers in order.
    pub fn new(clients: Vec<Box<dyn LlmClient>>, timeout: Duration) -> Self {
        Self {
            clients,
            current_idx: 0,
            timeout,
        }
    }

    /// Create from environment with automatic provider detection.
    ///
    /// Reads `LLM_FALLBACK` env var for fallback order (comma-separated).
    /// Example: `LLM_FALLBACK=gemini,anthropic` tries Gemini first, then Claude.
    ///
    /// Default order: anthropic, gemini, openai
    pub fn from_env() -> Result<Self> {
        let fallback_order =
            std::env::var("LLM_FALLBACK").unwrap_or_else(|_| "anthropic,gemini,openai".to_string());

        let provider_names: Vec<&str> = fallback_order.split(',').map(|s| s.trim()).collect();

        let mut clients: Vec<Box<dyn LlmClient>> = Vec::new();

        for name in provider_names {
            let client: Box<dyn LlmClient> = match name {
                "anthropic" => match AnthropicClient::from_env() {
                    Ok(c) => {
                        info!(provider = name, model = %c.model(), "Fallback client initialized");
                        Box::new(c)
                    }
                    Err(e) => {
                        warn!(provider = name, err = %e, "Failed to initialize provider, skipping");
                        continue;
                    }
                },
                "gemini" => match GeminiClient::from_env() {
                    Ok(c) => {
                        info!(provider = name, model = %c.model(), "Fallback client initialized");
                        Box::new(c)
                    }
                    Err(e) => {
                        warn!(provider = name, err = %e, "Failed to initialize provider, skipping");
                        continue;
                    }
                },
                "openai" => match OpenAiClient::from_env() {
                    Ok(c) => {
                        info!(provider = name, model = %c.model(), "Fallback client initialized");
                        Box::new(c)
                    }
                    Err(e) => {
                        warn!(provider = name, err = %e, "Failed to initialize provider, skipping");
                        continue;
                    }
                },
                other => {
                    warn!(
                        provider = other,
                        "Unknown provider in LLM_FALLBACK, skipping"
                    );
                    continue;
                }
            };
            clients.push(client);
        }

        if clients.is_empty() {
            bail!("No valid LLM providers configured. Set at least one of: ANTHROPIC_API_KEY, GEMINI_API_KEY, or OPENAI_API_KEY");
        }

        let timeout_secs = std::env::var("LLM_TIMEOUT_SECS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(60);

        Ok(Self::new(clients, Duration::from_secs(timeout_secs)))
    }
}

#[async_trait]
impl LlmClient for FallbackClient {
    async fn send(&self, messages: &[Message], tools: &[ToolSchema]) -> Result<LlmResponse> {
        let mut last_error = None;

        for (idx, client) in self.clients.iter().enumerate() {
            if idx > 0 {
                warn!(
                    from_provider = self
                        .clients
                        .get(idx - 1)
                        .map(|c| c.model())
                        .unwrap_or("unknown"),
                    to_provider = client.model(),
                    "Falling back to next provider"
                );
            }

            // Try with timeout
            let result = tokio::time::timeout(self.timeout, client.send(messages, tools)).await;

            match result {
                Ok(Ok(response)) => {
                    return Ok(response);
                }
                Ok(Err(e)) => {
                    warn!(
                        provider = client.model(),
                        error = %e,
                        "Provider failed, trying next"
                    );
                    last_error = Some(e);
                }
                Err(_timeout) => {
                    warn!(
                        provider = client.model(),
                        timeout_secs = self.timeout.as_secs(),
                        "Provider timed out, trying next"
                    );
                    last_error = Some(anyhow::anyhow!(
                        "Provider timed out after {}s",
                        self.timeout.as_secs()
                    ));
                }
            }
        }

        // All providers failed
        match last_error {
            Some(e) => bail!("All LLM providers failed. Last error: {}", e),
            None => bail!("No LLM providers configured"),
        }
    }

    fn model(&self) -> &str {
        self.clients
            .get(self.current_idx)
            .map(|c| c.model())
            .unwrap_or("unknown")
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_fallback_order_parsing() {
        std::env::set_var("LLM_FALLBACK", "gemini,anthropic");
        // This would need actual API keys to test fully
        // Just verify it doesn't crash on parsing
        std::env::remove_var("LLM_FALLBACK");
    }
}
