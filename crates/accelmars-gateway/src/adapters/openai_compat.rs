use std::time::Duration;

use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use serde::{Deserialize, Serialize};

use accelmars_gateway_core::{AdapterError, GatewayRequest, GatewayResponse, ProviderAdapter};

/// Shared adapter for providers that speak the OpenAI chat completions format natively.
/// DeepSeek, OpenRouter, and Groq all use this — near-passthrough with auth differences.
pub struct OpenAiCompatibleAdapter {
    adapter_name: String,
    client: reqwest::Client,
    handle: tokio::runtime::Handle,
    base_url: String,
    api_key: Option<String>,
    default_model: String,
    extra_headers: HeaderMap,
}

impl OpenAiCompatibleAdapter {
    pub fn new(
        name: impl Into<String>,
        base_url: impl Into<String>,
        api_key: Option<String>,
        default_model: impl Into<String>,
    ) -> Self {
        Self {
            adapter_name: name.into(),
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(120))
                .build()
                .expect("failed to build HTTP client"),
            handle: tokio::runtime::Handle::current(),
            base_url: base_url.into(),
            api_key,
            default_model: default_model.into(),
            extra_headers: HeaderMap::new(),
        }
    }

    pub fn with_extra_header(mut self, key: &str, value: &str) -> Self {
        if let (Ok(name), Ok(val)) = (
            key.parse::<reqwest::header::HeaderName>(),
            HeaderValue::from_str(value),
        ) {
            self.extra_headers.insert(name, val);
        }
        self
    }
}

#[derive(Serialize)]
pub(crate) struct OaiRequest {
    pub model: String,
    pub messages: Vec<OaiMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    pub stream: bool,
}

#[derive(Serialize, Deserialize, Clone)]
pub(crate) struct OaiMessage {
    pub role: String,
    pub content: String,
}

#[derive(Deserialize)]
pub(crate) struct OaiResponse {
    pub id: Option<String>,
    pub model: Option<String>,
    pub choices: Vec<OaiChoice>,
    pub usage: Option<OaiUsage>,
}

#[derive(Deserialize)]
pub(crate) struct OaiChoice {
    pub message: OaiChoiceMessage,
    pub finish_reason: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct OaiChoiceMessage {
    pub content: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct OaiUsage {
    pub prompt_tokens: Option<u32>,
    pub completion_tokens: Option<u32>,
}

pub(crate) fn build_oai_request(req: &GatewayRequest, model: &str) -> OaiRequest {
    OaiRequest {
        model: model.to_string(),
        messages: req
            .messages
            .iter()
            .map(|m| OaiMessage {
                role: m.role.clone(),
                content: m.content.clone(),
            })
            .collect(),
        max_tokens: req.max_tokens,
        stream: false,
    }
}

pub(crate) fn parse_oai_response(
    resp: OaiResponse,
    adapter_name: &str,
) -> Result<GatewayResponse, AdapterError> {
    let choice = resp
        .choices
        .first()
        .ok_or_else(|| AdapterError::ParseError("empty choices array".to_string()))?;
    let content = choice.message.content.clone().unwrap_or_default();
    let usage = resp.usage.as_ref();

    Ok(GatewayResponse {
        id: resp.id.unwrap_or_else(|| format!("{adapter_name}-unknown")),
        model: resp.model.unwrap_or_else(|| adapter_name.to_string()),
        content,
        tokens_in: usage.and_then(|u| u.prompt_tokens).unwrap_or(0),
        tokens_out: usage.and_then(|u| u.completion_tokens).unwrap_or(0),
        finish_reason: choice
            .finish_reason
            .clone()
            .unwrap_or_else(|| "stop".to_string()),
    })
}

pub(crate) fn map_http_error(status: u16, body: &str) -> AdapterError {
    match status {
        429 => AdapterError::RateLimit { retry_after: None },
        401 | 403 => AdapterError::AuthError(body.to_string()),
        402 => AdapterError::ProviderError(format!("billing error: {body}")),
        408 | 504 => AdapterError::Timeout,
        _ => AdapterError::ProviderError(format!("HTTP {status}: {body}")),
    }
}

impl ProviderAdapter for OpenAiCompatibleAdapter {
    fn name(&self) -> &str {
        &self.adapter_name
    }

    fn complete(&self, request: &GatewayRequest) -> Result<GatewayResponse, AdapterError> {
        self.handle.block_on(async {
            let body = build_oai_request(request, &self.default_model);

            let mut req_builder = self
                .client
                .post(&self.base_url)
                .header(CONTENT_TYPE, "application/json")
                .json(&body);

            if let Some(ref key) = self.api_key {
                req_builder = req_builder.header(AUTHORIZATION, format!("Bearer {key}"));
            }

            for (name, value) in self.extra_headers.iter() {
                req_builder = req_builder.header(name, value);
            }

            let response = req_builder.send().await.map_err(|e| {
                if e.is_timeout() {
                    AdapterError::Timeout
                } else {
                    AdapterError::ProviderError(e.to_string())
                }
            })?;

            let status = response.status().as_u16();
            if status != 200 {
                let body_text = response.text().await.unwrap_or_default();
                return Err(map_http_error(status, &body_text));
            }

            let oai_resp: OaiResponse = response
                .json()
                .await
                .map_err(|e| AdapterError::ParseError(e.to_string()))?;

            parse_oai_response(oai_resp, &self.adapter_name)
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
            tier: ModelTier::Standard,
            constraints: RoutingConstraints::default(),
            messages: vec![
                Message {
                    role: "system".to_string(),
                    content: "You are helpful.".to_string(),
                },
                Message {
                    role: "user".to_string(),
                    content: "Hello".to_string(),
                },
            ],
            max_tokens: Some(1024),
            stream: false,
            metadata: Default::default(),
        }
    }

    #[test]
    fn build_request_preserves_messages_and_model() {
        let req = test_request();
        let oai = build_oai_request(&req, "test-model");
        assert_eq!(oai.model, "test-model");
        assert_eq!(oai.messages.len(), 2);
        assert_eq!(oai.messages[0].role, "system");
        assert_eq!(oai.messages[1].content, "Hello");
        assert_eq!(oai.max_tokens, Some(1024));
        assert!(!oai.stream);
    }

    #[test]
    fn parse_response_extracts_content_and_usage() {
        let resp = OaiResponse {
            id: Some("chatcmpl-abc".to_string()),
            model: Some("deepseek-chat".to_string()),
            choices: vec![OaiChoice {
                message: OaiChoiceMessage {
                    content: Some("Hello back!".to_string()),
                },
                finish_reason: Some("stop".to_string()),
            }],
            usage: Some(OaiUsage {
                prompt_tokens: Some(10),
                completion_tokens: Some(5),
            }),
        };
        let gw = parse_oai_response(resp, "test").unwrap();
        assert_eq!(gw.content, "Hello back!");
        assert_eq!(gw.tokens_in, 10);
        assert_eq!(gw.tokens_out, 5);
        assert_eq!(gw.finish_reason, "stop");
        assert_eq!(gw.model, "deepseek-chat");
    }

    #[test]
    fn parse_response_handles_empty_choices() {
        let resp = OaiResponse {
            id: None,
            model: None,
            choices: vec![],
            usage: None,
        };
        let err = parse_oai_response(resp, "test").unwrap_err();
        assert!(matches!(err, AdapterError::ParseError(_)));
    }

    #[test]
    fn map_error_rate_limit_429() {
        let err = map_http_error(429, "rate limited");
        assert!(matches!(err, AdapterError::RateLimit { .. }));
    }

    #[test]
    fn map_error_auth_401() {
        let err = map_http_error(401, "invalid key");
        assert!(matches!(err, AdapterError::AuthError(_)));
    }

    #[test]
    fn map_error_billing_402() {
        let err = map_http_error(402, "insufficient balance");
        assert!(matches!(err, AdapterError::ProviderError(_)));
    }
}
