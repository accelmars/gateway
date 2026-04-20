use std::time::Duration;

use serde::{Deserialize, Serialize};

use accelmars_gateway_core::{AdapterError, GatewayRequest, GatewayResponse, ProviderAdapter};

pub struct ClaudeAdapter {
    client: reqwest::Client,
    handle: tokio::runtime::Handle,
    api_key: Option<String>,
    default_model: String,
}

impl ClaudeAdapter {
    pub fn new(api_key: Option<String>, default_model: impl Into<String>) -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(180))
                .build()
                .expect("failed to build HTTP client"),
            handle: tokio::runtime::Handle::current(),
            api_key,
            default_model: default_model.into(),
        }
    }
}

// --- Anthropic Messages API types ---

#[derive(Serialize)]
pub(crate) struct AnthropicRequest {
    model: String,
    max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<String>,
    messages: Vec<AnthropicMessage>,
}

#[derive(Serialize)]
struct AnthropicMessage {
    role: String,
    content: String,
}

#[derive(Deserialize)]
pub(crate) struct AnthropicResponse {
    id: String,
    model: String,
    content: Vec<AnthropicContent>,
    stop_reason: Option<String>,
    usage: AnthropicUsage,
}

#[derive(Deserialize)]
struct AnthropicContent {
    #[serde(rename = "type")]
    content_type: String,
    text: Option<String>,
}

#[derive(Deserialize)]
struct AnthropicUsage {
    input_tokens: u32,
    output_tokens: u32,
}

// --- Pure functions (testable without HTTP) ---

pub(crate) fn build_anthropic_request(req: &GatewayRequest, model: &str) -> AnthropicRequest {
    let mut system = None;
    let mut messages = Vec::new();

    for msg in &req.messages {
        if msg.role == "system" {
            system = Some(msg.content.clone());
        } else {
            messages.push(AnthropicMessage {
                role: msg.role.clone(),
                content: msg.content.clone(),
            });
        }
    }

    AnthropicRequest {
        model: model.to_string(),
        max_tokens: req.max_tokens.unwrap_or(4096),
        system,
        messages,
    }
}

fn anthropic_stop_reason_to_openai(reason: &str) -> &str {
    match reason {
        "end_turn" => "stop",
        "max_tokens" => "length",
        "stop_sequence" => "stop",
        _ => "stop",
    }
}

pub(crate) fn parse_anthropic_response(
    resp: AnthropicResponse,
) -> Result<GatewayResponse, AdapterError> {
    let content = resp
        .content
        .iter()
        .filter(|c| c.content_type == "text")
        .filter_map(|c| c.text.as_deref())
        .collect::<Vec<_>>()
        .join("");

    let finish_reason = resp.stop_reason.as_deref().unwrap_or("end_turn");

    Ok(GatewayResponse {
        id: resp.id,
        model: resp.model,
        content,
        tokens_in: resp.usage.input_tokens,
        tokens_out: resp.usage.output_tokens,
        finish_reason: anthropic_stop_reason_to_openai(finish_reason).to_string(),
    })
}

impl ProviderAdapter for ClaudeAdapter {
    fn name(&self) -> &str {
        "claude"
    }

    fn complete(&self, request: &GatewayRequest) -> Result<GatewayResponse, AdapterError> {
        self.handle.block_on(async {
            let api_key = self
                .api_key
                .as_deref()
                .ok_or_else(|| AdapterError::AuthError("ANTHROPIC_API_KEY not set".to_string()))?;

            let body = build_anthropic_request(request, &self.default_model);

            let response = self
                .client
                .post("https://api.anthropic.com/v1/messages")
                .header("x-api-key", api_key)
                .header("anthropic-version", "2023-06-01")
                .header("content-type", "application/json")
                .json(&body)
                .send()
                .await
                .map_err(|e| {
                    if e.is_timeout() {
                        AdapterError::Timeout
                    } else {
                        AdapterError::ProviderError(e.to_string())
                    }
                })?;

            let status = response.status().as_u16();
            if status != 200 {
                let body_text = response.text().await.unwrap_or_default();
                return Err(super::openai_compat::map_http_error(status, &body_text));
            }

            let anthropic_resp: AnthropicResponse = response
                .json()
                .await
                .map_err(|e| AdapterError::ParseError(e.to_string()))?;

            parse_anthropic_response(anthropic_resp)
        })
    }

    fn is_available(&self) -> bool {
        self.api_key.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use accelmars_gateway_core::{Message, ModelTier, RoutingConstraints};

    fn test_request() -> GatewayRequest {
        GatewayRequest {
            tier: ModelTier::Max,
            constraints: RoutingConstraints::default(),
            messages: vec![
                Message {
                    role: "system".to_string(),
                    content: "You are helpful.".to_string(),
                },
                Message {
                    role: "user".to_string(),
                    content: "Explain traits.".to_string(),
                },
            ],
            max_tokens: Some(2048),
            stream: false,
            metadata: Default::default(),
        }
    }

    #[test]
    fn build_request_extracts_system_message() {
        let req = test_request();
        let anthropic = build_anthropic_request(&req, "claude-sonnet-4-6");
        assert_eq!(anthropic.system, Some("You are helpful.".to_string()));
        assert_eq!(anthropic.messages.len(), 1);
        assert_eq!(anthropic.messages[0].role, "user");
        assert_eq!(anthropic.model, "claude-sonnet-4-6");
        assert_eq!(anthropic.max_tokens, 2048);
    }

    #[test]
    fn build_request_default_max_tokens_4096() {
        let mut req = test_request();
        req.max_tokens = None;
        let anthropic = build_anthropic_request(&req, "claude-sonnet-4-6");
        assert_eq!(anthropic.max_tokens, 4096);
    }

    #[test]
    fn parse_response_extracts_text_content() {
        let resp = AnthropicResponse {
            id: "msg_abc123".to_string(),
            model: "claude-sonnet-4-6".to_string(),
            content: vec![AnthropicContent {
                content_type: "text".to_string(),
                text: Some("Traits are Rust's interface system.".to_string()),
            }],
            stop_reason: Some("end_turn".to_string()),
            usage: AnthropicUsage {
                input_tokens: 25,
                output_tokens: 12,
            },
        };
        let gw = parse_anthropic_response(resp).unwrap();
        assert_eq!(gw.content, "Traits are Rust's interface system.");
        assert_eq!(gw.id, "msg_abc123");
        assert_eq!(gw.model, "claude-sonnet-4-6");
        assert_eq!(gw.tokens_in, 25);
        assert_eq!(gw.tokens_out, 12);
        assert_eq!(gw.finish_reason, "stop");
    }

    #[test]
    fn parse_response_maps_max_tokens_stop_reason() {
        let resp = AnthropicResponse {
            id: "msg_xyz".to_string(),
            model: "claude-sonnet-4-6".to_string(),
            content: vec![AnthropicContent {
                content_type: "text".to_string(),
                text: Some("truncated...".to_string()),
            }],
            stop_reason: Some("max_tokens".to_string()),
            usage: AnthropicUsage {
                input_tokens: 10,
                output_tokens: 100,
            },
        };
        let gw = parse_anthropic_response(resp).unwrap();
        assert_eq!(gw.finish_reason, "length");
    }

    #[test]
    fn parse_response_joins_multiple_text_blocks() {
        let resp = AnthropicResponse {
            id: "msg_multi".to_string(),
            model: "claude-sonnet-4-6".to_string(),
            content: vec![
                AnthropicContent {
                    content_type: "text".to_string(),
                    text: Some("Part one. ".to_string()),
                },
                AnthropicContent {
                    content_type: "text".to_string(),
                    text: Some("Part two.".to_string()),
                },
            ],
            stop_reason: Some("end_turn".to_string()),
            usage: AnthropicUsage {
                input_tokens: 5,
                output_tokens: 10,
            },
        };
        let gw = parse_anthropic_response(resp).unwrap();
        assert_eq!(gw.content, "Part one. Part two.");
    }
}
