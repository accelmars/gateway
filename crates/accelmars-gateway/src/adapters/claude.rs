use std::time::Duration;

use serde::{Deserialize, Serialize};

use accelmars_gateway_core::{
    AdapterError, ChunkedResponse, GatewayRequest, GatewayResponse, ProviderAdapter,
};

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
    #[serde(skip_serializing_if = "Option::is_none")]
    stream: Option<bool>,
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

// --- Anthropic streaming event types ---

/// Flat envelope for all Anthropic SSE `data:` lines.
/// Each field is optional — only populated by the event types that carry it.
/// Unknown top-level fields are silently ignored (no `deny_unknown_fields`).
#[derive(Deserialize)]
pub(crate) struct ClaudeStreamEvent {
    #[serde(rename = "type")]
    event_type: String,
    /// Present on `message_start` — carries id, model, input token count.
    #[serde(default)]
    message: Option<ClaudeMessageStartContent>,
    /// Present on `content_block_delta` (text_delta) and `message_delta` (stop_reason).
    #[serde(default)]
    delta: Option<ClaudeStreamDelta>,
    /// Present on `message_delta` — carries output token count.
    #[serde(default)]
    usage: Option<ClaudeStreamUsage>,
}

#[derive(Deserialize)]
pub(crate) struct ClaudeMessageStartContent {
    id: String,
    model: String,
    usage: ClaudeInputUsage,
}

#[derive(Deserialize)]
pub(crate) struct ClaudeInputUsage {
    input_tokens: u32,
}

/// Shared delta struct used by `content_block_delta` and `message_delta`.
/// Fields default to zero-values when absent from a given event type.
#[derive(Deserialize)]
pub(crate) struct ClaudeStreamDelta {
    /// Event subtype — `"text_delta"` for content chunks. Empty string for `message_delta`.
    #[serde(rename = "type", default)]
    delta_type: String,
    /// Text content — present when `delta_type == "text_delta"`.
    #[serde(default)]
    text: String,
    /// Stop reason — present in `message_delta.delta`.
    #[serde(default)]
    stop_reason: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct ClaudeStreamUsage {
    /// Output token count from `message_delta.usage`.
    #[serde(default)]
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
        stream: None,
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

/// Parse an Anthropic Messages API SSE stream body into a [`ChunkedResponse`].
///
/// Handles the full Anthropic event lifecycle:
/// `message_start` → `content_block_start` → `content_block_delta`(×N) →
/// `content_block_stop` → `message_delta` → `message_stop`.
///
/// Only `text_delta` events contribute to `chunks`. All other event types
/// contribute metadata (tokens, id, model, finish_reason) or are no-ops.
/// Unknown future event types are skipped silently — never panics on new events.
pub(crate) fn parse_claude_stream(
    body: &str,
    fallback_model: &str,
) -> Result<ChunkedResponse, AdapterError> {
    let mut chunks: Vec<String> = Vec::new();
    let mut tokens_in: u32 = 0;
    let mut tokens_out: u32 = 0;
    let mut finish_reason = "stop".to_string();
    let mut response_id = "claude-stream-unknown".to_string();
    let mut response_model = fallback_model.to_string();

    for line in body.lines() {
        let data = match line.trim().strip_prefix("data: ") {
            Some(d) if !d.is_empty() => d,
            _ => continue,
        };

        let event: ClaudeStreamEvent = match serde_json::from_str(data) {
            Ok(e) => e,
            Err(_) => continue,
        };

        match event.event_type.as_str() {
            "message_start" => {
                if let Some(msg) = event.message {
                    tokens_in = msg.usage.input_tokens;
                    response_id = msg.id;
                    response_model = msg.model;
                }
            }
            "content_block_delta" => {
                if let Some(delta) = event.delta {
                    if delta.delta_type == "text_delta" && !delta.text.is_empty() {
                        chunks.push(delta.text);
                    }
                }
            }
            "message_delta" => {
                if let Some(delta) = &event.delta {
                    if let Some(stop_reason) = delta.stop_reason.as_deref() {
                        finish_reason = anthropic_stop_reason_to_openai(stop_reason).to_string();
                    }
                }
                if let Some(usage) = event.usage {
                    tokens_out = usage.output_tokens;
                }
            }
            // Recognized no-op events — listed explicitly so the catch-all is truly
            // for future unknown events only.
            "content_block_start" | "content_block_stop" | "message_stop" | "ping" => {}
            _ => {}
        }
    }

    if chunks.is_empty() {
        return Err(AdapterError::ParseError(
            "no content chunks in Claude SSE response".to_string(),
        ));
    }

    Ok(ChunkedResponse {
        id: response_id,
        model: response_model,
        chunks,
        tokens_in,
        tokens_out,
        finish_reason,
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

    fn complete_chunks(&self, request: &GatewayRequest) -> Result<ChunkedResponse, AdapterError> {
        self.handle.block_on(async {
            let api_key = self
                .api_key
                .as_deref()
                .ok_or_else(|| AdapterError::AuthError("ANTHROPIC_API_KEY not set".to_string()))?;

            let mut body = build_anthropic_request(request, &self.default_model);
            body.stream = Some(true);

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

            let text = response
                .text()
                .await
                .map_err(|e| AdapterError::ProviderError(e.to_string()))?;

            parse_claude_stream(&text, &self.default_model)
        })
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

    fn make_streaming_request() -> GatewayRequest {
        GatewayRequest {
            tier: ModelTier::Max,
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
    fn build_request_stream_defaults_to_none() {
        let req = test_request();
        let anthropic = build_anthropic_request(&req, "claude-sonnet-4-6");
        assert!(anthropic.stream.is_none());
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

    // --- Streaming: parse_claude_stream ---

    fn full_claude_sse_body() -> String {
        // All 6 Anthropic event types: message_start, content_block_start,
        // content_block_delta(×3), content_block_stop, message_delta, message_stop.
        // Also includes ping (no-op) and an unknown future event (must be skipped).
        concat!(
            "event: message_start\n",
            "data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_test001\",\"type\":\"message\",\"role\":\"assistant\",\"content\":[],\"model\":\"claude-sonnet-4-6\",\"stop_reason\":null,\"stop_sequence\":null,\"usage\":{\"input_tokens\":15,\"output_tokens\":1}}}\n",
            "\n",
            "event: content_block_start\n",
            "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n",
            "\n",
            "event: ping\n",
            "data: {\"type\":\"ping\"}\n",
            "\n",
            "event: content_block_delta\n",
            "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hello\"}}\n",
            "\n",
            "event: content_block_delta\n",
            "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\", there\"}}\n",
            "\n",
            "event: content_block_delta\n",
            "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\" world!\"}}\n",
            "\n",
            "event: content_block_stop\n",
            "data: {\"type\":\"content_block_stop\",\"index\":0}\n",
            "\n",
            "event: message_delta\n",
            "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\",\"stop_sequence\":null},\"usage\":{\"output_tokens\":9}}\n",
            "\n",
            "event: message_stop\n",
            "data: {\"type\":\"message_stop\"}\n",
        ).to_string()
    }

    #[test]
    fn claude_streaming_parse_yields_three_chunks() {
        let body = full_claude_sse_body();
        let result = parse_claude_stream(&body, "claude-sonnet-4-6").unwrap();
        assert_eq!(result.chunks.len(), 3);
        assert_eq!(result.chunks[0], "Hello");
        assert_eq!(result.chunks[1], ", there");
        assert_eq!(result.chunks[2], " world!");
        assert_eq!(result.tokens_in, 15);
        assert_eq!(result.tokens_out, 9);
        assert_eq!(result.finish_reason, "stop");
        assert_eq!(result.id, "msg_test001");
        assert_eq!(result.model, "claude-sonnet-4-6");
    }

    #[test]
    fn claude_streaming_parse_maps_max_tokens_stop_reason() {
        let body = concat!(
            "data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_t\",\"type\":\"message\",\"role\":\"assistant\",\"content\":[],\"model\":\"claude-sonnet-4-6\",\"stop_reason\":null,\"stop_sequence\":null,\"usage\":{\"input_tokens\":10,\"output_tokens\":1}}}\n",
            "\n",
            "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"truncated\"}}\n",
            "\n",
            "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"max_tokens\",\"stop_sequence\":null},\"usage\":{\"output_tokens\":100}}\n",
            "\n",
            "data: {\"type\":\"message_stop\"}\n",
        );
        let result = parse_claude_stream(body, "claude-sonnet-4-6").unwrap();
        assert_eq!(result.finish_reason, "length");
        assert_eq!(result.chunks, vec!["truncated"]);
        assert_eq!(result.tokens_in, 10);
        assert_eq!(result.tokens_out, 100);
    }

    #[test]
    fn claude_streaming_parse_skips_empty_text_deltas() {
        let body = concat!(
            "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"\"}}\n",
            "\n",
            "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"real\"}}\n",
            "\n",
            "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\",\"stop_sequence\":null},\"usage\":{\"output_tokens\":2}}\n",
            "\n",
            "data: {\"type\":\"message_stop\"}\n",
        );
        let result = parse_claude_stream(body, "claude-sonnet-4-6").unwrap();
        assert_eq!(result.chunks, vec!["real"]);
    }

    #[test]
    fn claude_streaming_parse_skips_malformed_and_unknown_events() {
        // Tests: malformed JSON, comment lines, and an unknown future event type.
        let body = concat!(
            "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hello\"}}\n",
            "\n",
            "data: not-valid-json\n",
            "\n",
            ": comment line ignored\n",
            "\n",
            "event: unknown_future_event\n",
            "data: {\"type\":\"unknown_future_event\",\"some_new_field\":\"value\"}\n",
            "\n",
            "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\" world\"}}\n",
            "\n",
            "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\",\"stop_sequence\":null},\"usage\":{\"output_tokens\":3}}\n",
            "\n",
            "data: {\"type\":\"message_stop\"}\n",
        );
        let result = parse_claude_stream(body, "claude-sonnet-4-6").unwrap();
        assert_eq!(result.chunks, vec!["Hello", " world"]);
        assert_eq!(result.finish_reason, "stop");
        assert_eq!(result.tokens_out, 3);
    }

    #[test]
    fn claude_streaming_parse_stop_sequence_maps_to_stop() {
        let body = concat!(
            "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"done\"}}\n",
            "\n",
            "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"stop_sequence\",\"stop_sequence\":\"\\n\\n\"},\"usage\":{\"output_tokens\":1}}\n",
            "\n",
            "data: {\"type\":\"message_stop\"}\n",
        );
        let result = parse_claude_stream(body, "claude-sonnet-4-6").unwrap();
        assert_eq!(result.finish_reason, "stop");
    }

    #[test]
    fn claude_streaming_parse_empty_body_returns_parse_error() {
        let result = parse_claude_stream("", "claude-sonnet-4-6");
        assert!(matches!(result, Err(AdapterError::ParseError(_))));
    }

    #[test]
    fn claude_streaming_parse_uses_fallback_model_when_no_message_start() {
        // When message_start is absent (or malformed), fallback_model is used.
        let body = concat!(
            "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"hi\"}}\n",
            "\n",
            "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\",\"stop_sequence\":null},\"usage\":{\"output_tokens\":1}}\n",
        );
        let result = parse_claude_stream(body, "claude-opus-4-6").unwrap();
        assert_eq!(result.model, "claude-opus-4-6");
        assert_eq!(result.id, "claude-stream-unknown");
    }

    #[test]
    fn claude_streaming_cassette_complete_chunks_yields_three_deltas() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/claude-streaming-success.json");
        let adapter = FixtureAdapter::from_file("claude", &path).unwrap();
        let req = make_streaming_request();
        let result = adapter.complete_chunks(&req).unwrap();
        assert!(
            result.chunks.len() >= 3,
            "expected ≥3 content deltas, got {}",
            result.chunks.len()
        );
        assert!(result.tokens_in > 0, "tokens_in must be > 0");
        assert!(result.tokens_out > 0, "tokens_out must be > 0");
        assert_eq!(result.finish_reason, "stop", "finish_reason must be 'stop'");
    }
}
