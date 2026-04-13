use axum::{
    http::{HeaderMap, StatusCode},
    routing::post,
    Json, Router,
};
use reqwest::Client;
use serde::Deserialize;
use serde_json::{json, Value};
use std::net::SocketAddr;
use tracing::{debug, error, info, warn};

#[derive(Debug, Deserialize)]
struct AnthropicRequest {
    model: String,
    messages: Vec<Value>,
    system: Option<String>,
    tools: Option<Vec<Value>>,
    max_tokens: Option<u32>,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let app = Router::new().route("/v1/messages", post(handle_messages));

    let port = std::env::var("PORT").unwrap_or_else(|_| "8080".to_string());
    let addr = format!("0.0.0.0:{}", port).parse::<SocketAddr>().unwrap();

    info!("Anthropic-to-OpenAI Proxy listening on {}", addr);
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

async fn handle_messages(
    _headers: HeaderMap,
    Json(payload): Json<AnthropicRequest>,
) -> (StatusCode, Json<Value>) {
    info!(
        turn = payload.messages.len(),
        model = payload.model,
        "Received Anthropic request"
    );

    // 1. Get OpenAI API Key
    let api_key = match std::env::var("OPENAI_API_KEY") {
        Ok(k) => k,
        Err(_) => {
            error!("OPENAI_API_KEY not set");
            return (
                StatusCode::BAD_REQUEST,
                Json(
                    json!({"error": {"message": "OPENAI_API_KEY must be set in the proxy environment"}}),
                ),
            );
        }
    };

    // 2. Transform Anthropic Request -> OpenAI Request
    let mut openai_messages = Vec::new();

    // System prompt -> role: system
    if let Some(sys) = payload.system {
        openai_messages.push(json!({ "role": "system", "content": sys }));
    }

    // Messages array
    for msg in payload.messages {
        let role = msg["role"].as_str().unwrap_or("user");
        let content = &msg["content"];

        if role == "assistant" && content.is_array() {
            // Check for tool_use in assistant message
            let mut tool_calls = Vec::new();
            let mut text_content = String::new();

            for block in content.as_array().unwrap() {
                match block["type"].as_str() {
                    Some("tool_use") => {
                        tool_calls.push(json!({
                            "id": block["id"],
                            "type": "function",
                            "function": {
                                "name": block["name"],
                                "arguments": block["input"].to_string()
                            }
                        }));
                    }
                    Some("text") => {
                        text_content.push_str(block["text"].as_str().unwrap_or(""));
                    }
                    _ => {}
                }
            }

            let mut mapped_msg = json!({ "role": "assistant" });
            if !text_content.is_empty() {
                mapped_msg["content"] = json!(text_content);
            }
            if !tool_calls.is_empty() {
                mapped_msg["tool_calls"] = json!(tool_calls);
            }
            openai_messages.push(mapped_msg);
        } else if role == "user" && content.is_array() {
            // Check for tool_result in user message
            let mut other_blocks = Vec::new();
            for block in content.as_array().unwrap() {
                if block["type"].as_str() == Some("tool_result") {
                    // OpenAI wants a separate message for EACH tool result
                    openai_messages.push(json!({
                        "role": "tool",
                        "tool_call_id": block["tool_use_id"],
                        "content": block["content"].as_str().unwrap_or("")
                    }));
                } else {
                    other_blocks.push(block.clone());
                }
            }
            if !other_blocks.is_empty() {
                openai_messages.push(json!({ "role": "user", "content": other_blocks }));
            }
        } else {
            // Simple string content or unknown role
            openai_messages.push(msg);
        }
    }

    // Tools -> role: tool and parameters
    let openai_tools: Option<Vec<Value>> = payload.tools.map(|tools| {
        tools
            .into_iter()
            .map(|t| {
                json!({
                    "type": "function",
                    "function": {
                        "name": t["name"],
                        "description": t["description"],
                        "parameters": t["input_schema"]
                    }
                })
            })
            .collect()
    });

    let openai_payload = json!({
        "model": std::env::var("OPENAI_MODEL").unwrap_or_else(|_| "gpt-4o".to_string()),
        "messages": openai_messages,
        "tools": openai_tools,
        "max_tokens": payload.max_tokens,
    });

    debug!(openai_payload = %openai_payload, "Forwarding to OpenAI");

    // 3. Call OpenAI
    let client = Client::new();
    let resp = match client
        .post("https://api.openai.com/v1/chat/completions")
        .bearer_auth(api_key)
        .json(&openai_payload)
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            error!(err = %e, "OpenAI request failed");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": {"message": format!("OpenAI request failed: {}", e)}})),
            );
        }
    };

    let status = resp.status();
    let openai_raw: Value = match resp.json().await {
        Ok(j) => j,
        Err(e) => {
            error!(err = %e, "Failed to parse OpenAI JSON");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": {"message": "Malformed OpenAI response"}})),
            );
        }
    };

    if !status.is_success() {
        warn!(status = %status, "OpenAI error response");
        return (
            StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
            Json(openai_raw),
        );
    }

    // 4. Transform OpenAI Response -> Anthropic Response
    let choice = &openai_raw["choices"][0];
    let message = &choice["message"];

    let mut anthropic_content = Vec::new();

    // Text content
    if let Some(text) = message["content"].as_str() {
        if !text.is_empty() {
            anthropic_content.push(json!({ "type": "text", "text": text }));
        }
    }

    // Tool calls
    if let Some(tool_calls) = message["tool_calls"].as_array() {
        for tc in tool_calls {
            let func = &tc["function"];
            let name = func["name"].as_str().unwrap_or("");
            let args_str = func["arguments"].as_str().unwrap_or("{}");
            let args: Value = serde_json::from_str(args_str).unwrap_or(json!({}));

            anthropic_content.push(json!({
                "type": "tool_use",
                "id": tc["id"],
                "name": name,
                "input": args
            }));
        }
    }

    let stop_reason = match choice["finish_reason"].as_str() {
        Some("tool_calls") => "tool_use",
        Some("stop") => "end_turn",
        _ => "end_turn",
    };

    let anthropic_resp = json!({
        "id": format!("openai-{}", openai_raw["id"].as_str().unwrap_or("")),
        "type": "message",
        "role": "assistant",
        "model": payload.model,
        "content": anthropic_content,
        "stop_reason": stop_reason,
        "usage": {
            "input_tokens": openai_raw["usage"]["prompt_tokens"],
            "output_tokens": openai_raw["usage"]["completion_tokens"]
        }
    });

    info!("Returning Anthropic response (mapped from OpenAI)");
    (StatusCode::OK, Json(anthropic_resp))
}
