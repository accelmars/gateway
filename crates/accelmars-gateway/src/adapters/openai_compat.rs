use std::time::Duration;

use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use serde::{Deserialize, Serialize};

use accelmars_gateway_core::{
    AdapterError, ChunkedResponse, GatewayRequest, GatewayResponse, ProviderAdapter,
};

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

#[derive(Deserialize)]
pub(crate) struct OaiStreamChunk {
    pub id: Option<String>,
    pub model: Option<String>,
    #[serde(default)]
    pub choices: Vec<OaiStreamChoice>,
    pub usage: Option<OaiStreamUsage>,
}

#[derive(Deserialize)]
pub(crate) struct OaiStreamChoice {
    pub delta: OaiStreamDelta,
    pub finish_reason: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct OaiStreamDelta {
    pub content: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct OaiStreamUsage {
    pub prompt_tokens: Option<u32>,
    pub completion_tokens: Option<u32>,
}

pub(crate) fn parse_oai_sse_stream(
    body: &str,
    adapter_name: &str,
) -> Result<ChunkedResponse, AdapterError> {
    let mut chunks: Vec<String> = Vec::new();
    let mut tokens_in: u32 = 0;
    let mut tokens_out: u32 = 0;
    let mut finish_reason = "stop".to_string();
    let mut response_id = format!("{adapter_name}-unknown");
    let mut model = adapter_name.to_string();

    for line in body.lines() {
        let data = match line.trim().strip_prefix("data: ") {
            Some(d) if !d.is_empty() => d,
            _ => continue,
        };

        if data == "[DONE]" {
            break;
        }

        let chunk: OaiStreamChunk = match serde_json::from_str(data) {
            Ok(c) => c,
            Err(_) => continue,
        };

        if let Some(id) = chunk.id {
            response_id = id;
        }
        if let Some(m) = chunk.model {
            model = m;
        }

        if let Some(choice) = chunk.choices.first() {
            if let Some(content) = &choice.delta.content {
                if !content.is_empty() {
                    chunks.push(content.clone());
                }
            }
            if let Some(reason) = &choice.finish_reason {
                finish_reason = reason.clone();
            }
        }

        if let Some(usage) = chunk.usage {
            if let Some(tin) = usage.prompt_tokens {
                tokens_in = tin;
            }
            if let Some(tout) = usage.completion_tokens {
                tokens_out = tout;
            }
        }
    }

    if chunks.is_empty() {
        return Err(AdapterError::ParseError(
            "no content chunks in OAI SSE response".to_string(),
        ));
    }

    Ok(ChunkedResponse {
        id: response_id,
        model,
        chunks,
        tokens_in,
        tokens_out,
        finish_reason,
    })
}

pub(crate) async fn complete_chunks_oai_sse(
    client: &reqwest::Client,
    url: &str,
    api_key: Option<&str>,
    extra_headers: &HeaderMap,
    request: &GatewayRequest,
    model: &str,
    adapter_name: &str,
) -> Result<ChunkedResponse, AdapterError> {
    let mut body = build_oai_request(request, model);
    body.stream = true;

    let mut req_builder = client
        .post(url)
        .header(CONTENT_TYPE, "application/json")
        .json(&body);

    if let Some(key) = api_key {
        req_builder = req_builder.header(AUTHORIZATION, format!("Bearer {key}"));
    }

    for (name, value) in extra_headers.iter() {
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

    let text = response
        .text()
        .await
        .map_err(|e| AdapterError::ProviderError(e.to_string()))?;

    parse_oai_sse_stream(&text, adapter_name)
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

    fn complete_chunks(&self, request: &GatewayRequest) -> Result<ChunkedResponse, AdapterError> {
        self.handle.block_on(complete_chunks_oai_sse(
            &self.client,
            &self.base_url,
            self.api_key.as_deref(),
            &self.extra_headers,
            request,
            &self.default_model,
            &self.adapter_name,
        ))
    }

    fn is_available(&self) -> bool {
        self.api_key.is_some()
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use accelmars_gateway_core::{Message, ModelTier, RoutingConstraints};

    use super::super::fixture::FixtureAdapter;

    fn make_streaming_request() -> GatewayRequest {
        GatewayRequest {
            tier: ModelTier::Standard,
            constraints: RoutingConstraints::default(),
            messages: vec![Message {
                role: "user".to_string(),
                content: "Say hello in three words.".to_string(),
            }],
            max_tokens: Some(50),
            stream: true,
            metadata: Default::default(),
        }
    }

    #[test]
    fn deepseek_streaming_parse_yields_three_chunks() {
        let sse_body = concat!(
            "data: {\"id\":\"chatcmpl-001\",\"model\":\"deepseek-chat\",\"choices\":[{\"delta\":{\"content\":\"Hello\"},\"finish_reason\":null}]}\n",
            "\n",
            "data: {\"id\":\"chatcmpl-001\",\"model\":\"deepseek-chat\",\"choices\":[{\"delta\":{\"content\":\", there,\"},\"finish_reason\":null}]}\n",
            "\n",
            "data: {\"id\":\"chatcmpl-001\",\"model\":\"deepseek-chat\",\"choices\":[{\"delta\":{\"content\":\" world!\"},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":9,\"completion_tokens\":5}}\n",
            "\n",
            "data: [DONE]\n",
        );
        let result = parse_oai_sse_stream(sse_body, "deepseek").unwrap();
        assert_eq!(result.chunks.len(), 3);
        assert_eq!(result.chunks[0], "Hello");
        assert_eq!(result.chunks[1], ", there,");
        assert_eq!(result.chunks[2], " world!");
        assert_eq!(result.tokens_in, 9);
        assert_eq!(result.tokens_out, 5);
        assert_eq!(result.finish_reason, "stop");
        assert_eq!(result.model, "deepseek-chat");
    }

    #[test]
    fn deepseek_streaming_parse_skips_malformed_events() {
        let sse_body = concat!(
            "data: {\"id\":\"chatcmpl-001\",\"model\":\"deepseek-chat\",\"choices\":[{\"delta\":{\"content\":\"Hello\"},\"finish_reason\":null}]}\n",
            "\n",
            "data: not-valid-json\n",
            "\n",
            ": comment line ignored\n",
            "\n",
            "data: {\"id\":\"chatcmpl-001\",\"model\":\"deepseek-chat\",\"choices\":[{\"delta\":{\"content\":\" world\"},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":5,\"completion_tokens\":3}}\n",
            "\n",
            "data: [DONE]\n",
        );
        let result = parse_oai_sse_stream(sse_body, "deepseek").unwrap();
        assert_eq!(result.chunks, vec!["Hello", " world"]);
        assert_eq!(result.finish_reason, "stop");
        assert_eq!(result.tokens_in, 5);
        assert_eq!(result.tokens_out, 3);
    }

    #[test]
    fn deepseek_streaming_parse_empty_body_returns_parse_error() {
        let result = parse_oai_sse_stream("", "deepseek");
        assert!(matches!(result, Err(AdapterError::ParseError(_))));
    }

    #[test]
    fn deepseek_streaming_parse_skips_empty_content_role_chunk() {
        let sse_body = concat!(
            "data: {\"id\":\"chatcmpl-001\",\"model\":\"deepseek-chat\",\"choices\":[{\"delta\":{\"content\":\"\"},\"finish_reason\":null}]}\n",
            "\n",
            "data: {\"id\":\"chatcmpl-001\",\"model\":\"deepseek-chat\",\"choices\":[{\"delta\":{\"content\":\"real\"},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":4,\"completion_tokens\":2}}\n",
            "\n",
            "data: [DONE]\n",
        );
        let result = parse_oai_sse_stream(sse_body, "deepseek").unwrap();
        assert_eq!(result.chunks, vec!["real"]);
    }

    #[test]
    fn deepseek_streaming_cassette_complete_chunks_yields_three_deltas() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/deepseek-streaming-success.json");
        let adapter = FixtureAdapter::from_file("deepseek", &path).unwrap();
        let req = make_streaming_request();
        let result = adapter.complete_chunks(&req).unwrap();
        assert!(
            result.chunks.len() >= 3,
            "expected ≥3 content deltas, got {}",
            result.chunks.len()
        );
        assert!(result.tokens_in > 0, "tokens_in must be > 0");
        assert!(result.tokens_out > 0, "tokens_out must be > 0");
    }

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
