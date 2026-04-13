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

// ── Model-to-Provider Mapping ─────────────────────────────────────────────
//
// MODEL_PROVIDER_MAP maps model name prefixes to the provider client type
// that should handle them. Format: "prefix=provider,prefix=provider,..."
// Example: "glm=openai,gpt=openai,deepseek=openai,claude=anthropic,gemini=gemini"
//
// When a model_override (from the registry's model_backend) matches a prefix,
// the fallback chain is reordered so the matching provider is tried first.

fn resolve_provider_for_model(model: &str) -> Option<String> {
    let map = std::env::var("MODEL_PROVIDER_MAP").ok()?;
    for entry in map.split(',') {
        let entry = entry.trim();
        if let Some((prefix, provider)) = entry.split_once('=') {
            if model.starts_with(prefix.trim()) {
                return Some(provider.trim().to_string());
            }
        }
    }
    None
}

// ── Fallback Client ────────────────────────────────────────────────────────

pub struct FallbackClient {
    clients: Vec<Box<dyn LlmClient>>,
    current_idx: usize,
    timeout: Duration,
}

impl FallbackClient {
    pub fn new(clients: Vec<Box<dyn LlmClient>>, timeout: Duration) -> Self {
        Self {
            clients,
            current_idx: 0,
            timeout,
        }
    }

    pub fn from_env() -> Result<Self> {
        Self::build(None)
    }

    /// Like `from_env()`, but overrides the model for proxy/anthropic providers.
    ///
    /// Used when the registry specifies a `model_backend` for an agent.
    /// When MODEL_PROVIDER_MAP maps the model to a non-Anthropic provider
    /// (e.g., "glm=openai"), a client of the correct type is prepended to
    /// the fallback chain so the model is routed through the right API format.
    pub fn from_env_with_model(model_override: &str) -> Result<Self> {
        Self::build(Some(model_override))
    }

    fn build(model_override: Option<&str>) -> Result<Self> {
        let mut fallback_order =
            std::env::var("LLM_FALLBACK").unwrap_or_else(|_| "anthropic,gemini,openai".to_string());

        let proxy_active = std::env::var("PROXY_URL").is_ok()
            || std::env::var("ANTHROPIC_BASE_URL").is_ok();

        if proxy_active && !fallback_order.contains("proxy") {
            fallback_order = format!("proxy,{}", fallback_order);
        }

        // If a model override is provided and MODEL_PROVIDER_MAP maps it to a
        // specific provider type, prepend that provider so it's tried first.
        // This ensures models like "glm5" are routed through OpenAiClient
        // (which sends OpenAI-format requests) instead of AnthropicClient.
        let mapped_provider = model_override.and_then(|m| resolve_provider_for_model(m));
        if let Some(ref provider) = mapped_provider {
            let provider_entry = if proxy_active {
                format!("{}-proxy", provider)
            } else {
                provider.clone()
            };
            if !fallback_order.contains(&provider_entry) {
                fallback_order = format!("{},{}", provider_entry, fallback_order);
            }
            info!(
                model = model_override.unwrap_or(""),
                mapped_provider = %provider,
                fallback_order = %fallback_order,
                "Model mapped to provider via MODEL_PROVIDER_MAP"
            );
        }

        let provider_names: Vec<&str> = fallback_order.split(',').map(|s| s.trim()).collect();

        let mut clients: Vec<Box<dyn LlmClient>> = Vec::new();

        for name in provider_names {
            // Skip "proxy" and "anthropic" entries when the model was mapped to
            // a different provider — they'll be tried later as fallbacks only
            // if the mapped provider fails.
            if mapped_provider.is_some() && (name == "proxy" || name == "anthropic") {
                // Still include them as fallback (don't skip), but they'll use
                // the default Anthropic model rather than the override.
            }

            let client: Box<dyn LlmClient> = match name {
                "proxy" => {
                    let result = match model_override {
                        Some(m) => AnthropicClient::from_env_with_model(m),
                        None => AnthropicClient::from_env(),
                    };
                    match result {
                        Ok(c) => {
                            info!(provider = name, model = %c.model(), "Fallback client initialized (proxy)");
                            Box::new(c)
                        }
                        Err(e) => {
                            warn!(provider = name, err = %e, "Failed to initialize proxy provider, skipping");
                            continue;
                        }
                    }
                }
                "openai-proxy" => {
                    let default_model = std::env::var("OPENAI_MODEL").unwrap_or_else(|_| "gpt-4o-mini".to_string());
                    let model = model_override.unwrap_or(&default_model);
                    match OpenAiClient::from_proxy(model) {
                        Ok(c) => {
                            info!(provider = name, model = %c.model(), "Fallback client initialized (openai-proxy)");
                            Box::new(c)
                        }
                        Err(e) => {
                            warn!(provider = name, err = %e, "Failed to initialize openai-proxy provider, skipping");
                            continue;
                        }
                    }
                }
                "anthropic" => {
                    if proxy_active {
                        info!(provider = name, "Skipping direct anthropic — proxy is active");
                        continue;
                    }
                    let result = match model_override {
                        Some(m) => AnthropicClient::from_env_with_model(m),
                        None => AnthropicClient::from_env(),
                    };
                    match result {
                        Ok(c) => {
                            info!(provider = name, model = %c.model(), "Fallback client initialized");
                            Box::new(c)
                        }
                        Err(e) => {
                            warn!(provider = name, err = %e, "Failed to initialize provider, skipping");
                            continue;
                        }
                    }
                }
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
                "openai" => {
                    let result = match model_override {
                        Some(m) => OpenAiClient::from_env_with_model(m),
                        None => OpenAiClient::from_env(),
                    };
                    match result {
                        Ok(c) => {
                            info!(provider = name, model = %c.model(), "Fallback client initialized");
                            Box::new(c)
                        }
                        Err(e) => {
                            warn!(provider = name, err = %e, "Failed to initialize provider, skipping");
                            continue;
                        }
                    }
                }
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
    use super::*;

    #[test]
    fn test_fallback_order_parsing() {
        std::env::set_var("LLM_FALLBACK", "gemini,anthropic");
        std::env::remove_var("LLM_FALLBACK");
    }
}
