use std::time::Duration;

use serde::{Deserialize, Serialize};

use accelmars_gateway_core::{
    AdapterError, ChunkedResponse, GatewayRequest, GatewayResponse, ProviderAdapter,
};

pub struct GeminiAdapter {
    client: reqwest::Client,
    handle: tokio::runtime::Handle,
    api_key: Option<String>,
    default_model: String,
}

impl GeminiAdapter {
    pub fn new(api_key: Option<String>, default_model: impl Into<String>) -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(120))
                .build()
                .expect("failed to build HTTP client"),
            handle: tokio::runtime::Handle::current(),
            api_key,
            default_model: default_model.into(),
        }
    }
}

// --- Gemini request types ---

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GeminiRequest {
    contents: Vec<GeminiContent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system_instruction: Option<GeminiSystemInstruction>,
    #[serde(skip_serializing_if = "Option::is_none")]
    generation_config: Option<GeminiGenerationConfig>,
}

#[derive(Serialize)]
struct GeminiContent {
    role: String,
    parts: Vec<GeminiPart>,
}

#[derive(Serialize, Deserialize)]
struct GeminiPart {
    text: String,
}

#[derive(Serialize)]
struct GeminiSystemInstruction {
    parts: Vec<GeminiPart>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GeminiGenerationConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    max_output_tokens: Option<u32>,
}

// --- Gemini response types ---

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GeminiResponse {
    candidates: Option<Vec<GeminiCandidate>>,
    usage_metadata: Option<GeminiUsageMetadata>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GeminiCandidate {
    content: Option<GeminiCandidateContent>,
    finish_reason: Option<String>,
}

#[derive(Deserialize)]
struct GeminiCandidateContent {
    parts: Option<Vec<GeminiResponsePart>>,
}

#[derive(Deserialize)]
struct GeminiResponsePart {
    text: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GeminiUsageMetadata {
    prompt_token_count: Option<u32>,
    candidates_token_count: Option<u32>,
}

// --- Pure functions (testable without HTTP) ---

pub(crate) fn build_gemini_request(req: &GatewayRequest) -> GeminiRequest {
    let mut contents = Vec::new();
    let mut system_text: Option<String> = None;

    for msg in &req.messages {
        match msg.role.as_str() {
            "system" => {
                system_text = Some(msg.content.clone());
            }
            "assistant" => {
                contents.push(GeminiContent {
                    role: "model".to_string(),
                    parts: vec![GeminiPart {
                        text: msg.content.clone(),
                    }],
                });
            }
            _ => {
                contents.push(GeminiContent {
                    role: "user".to_string(),
                    parts: vec![GeminiPart {
                        text: msg.content.clone(),
                    }],
                });
            }
        }
    }

    GeminiRequest {
        contents,
        system_instruction: system_text.map(|t| GeminiSystemInstruction {
            parts: vec![GeminiPart { text: t }],
        }),
        generation_config: Some(GeminiGenerationConfig {
            max_output_tokens: req.max_tokens,
        }),
    }
}

fn gemini_finish_reason_to_openai(reason: &str) -> &str {
    match reason {
        "STOP" => "stop",
        "MAX_TOKENS" => "length",
        "SAFETY" => "content_filter",
        _ => "stop",
    }
}

/// Parse a complete Gemini SSE response body into a [`ChunkedResponse`].
///
/// Each `data:` line is a JSON-encoded `GeminiResponse` partial chunk.
/// Content text is collected from every chunk that has it.
/// Token counts come from the last chunk that includes `usageMetadata`.
/// Malformed SSE events are silently skipped per the Design Decisions in GI-020.
pub(crate) fn parse_gemini_stream(
    body: &str,
    model: &str,
) -> Result<ChunkedResponse, AdapterError> {
    let mut chunks: Vec<String> = Vec::new();
    let mut tokens_in: u32 = 0;
    let mut tokens_out: u32 = 0;
    let mut finish_reason = "stop".to_string();

    for line in body.lines() {
        let data = match line.trim().strip_prefix("data: ") {
            Some(d) if !d.is_empty() => d,
            _ => continue,
        };

        let chunk: GeminiResponse = match serde_json::from_str(data) {
            Ok(c) => c,
            Err(_) => continue,
        };

        if let Some(candidates) = &chunk.candidates {
            if let Some(candidate) = candidates.first() {
                if let Some(text) = candidate
                    .content
                    .as_ref()
                    .and_then(|c| c.parts.as_ref())
                    .and_then(|parts| parts.first())
                    .and_then(|p| p.text.as_ref())
                {
                    if !text.is_empty() {
                        chunks.push(text.clone());
                    }
                }
                if let Some(reason) = &candidate.finish_reason {
                    finish_reason = gemini_finish_reason_to_openai(reason).to_string();
                }
            }
        }

        if let Some(usage) = &chunk.usage_metadata {
            if let Some(tin) = usage.prompt_token_count {
                tokens_in = tin;
            }
            if let Some(tout) = usage.candidates_token_count {
                tokens_out = tout;
            }
        }
    }

    if chunks.is_empty() {
        return Err(AdapterError::ParseError(
            "no content chunks in Gemini SSE response".to_string(),
        ));
    }

    Ok(ChunkedResponse {
        id: format!("gemini-{}", uuid::Uuid::new_v4()),
        model: model.to_string(),
        chunks,
        tokens_in,
        tokens_out,
        finish_reason,
    })
}

pub(crate) fn parse_gemini_response(resp: GeminiResponse) -> Result<GatewayResponse, AdapterError> {
    let candidates = resp
        .candidates
        .ok_or_else(|| AdapterError::ParseError("no candidates in response".to_string()))?;
    let candidate = candidates
        .first()
        .ok_or_else(|| AdapterError::ParseError("empty candidates array".to_string()))?;

    let content = candidate
        .content
        .as_ref()
        .and_then(|c| c.parts.as_ref())
        .and_then(|parts| parts.first())
        .and_then(|p| p.text.clone())
        .unwrap_or_default();

    let usage = resp.usage_metadata.as_ref();
    let finish_reason = candidate.finish_reason.as_deref().unwrap_or("STOP");

    Ok(GatewayResponse {
        id: format!("gemini-{}", uuid::Uuid::new_v4()),
        model: "gemini".to_string(),
        content,
        tokens_in: usage.and_then(|u| u.prompt_token_count).unwrap_or(0),
        tokens_out: usage.and_then(|u| u.candidates_token_count).unwrap_or(0),
        finish_reason: gemini_finish_reason_to_openai(finish_reason).to_string(),
    })
}

impl ProviderAdapter for GeminiAdapter {
    fn name(&self) -> &str {
        "gemini"
    }

    fn complete(&self, request: &GatewayRequest) -> Result<GatewayResponse, AdapterError> {
        self.handle.block_on(async {
            let api_key = self
                .api_key
                .as_deref()
                .ok_or_else(|| AdapterError::AuthError("GEMINI_API_KEY not set".to_string()))?;

            let url = format!(
                "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent?key={}",
                self.default_model, api_key
            );

            let body = build_gemini_request(request);

            let response = self
                .client
                .post(&url)
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

            let gemini_resp: GeminiResponse = response
                .json()
                .await
                .map_err(|e| AdapterError::ParseError(e.to_string()))?;

            let mut gw_resp = parse_gemini_response(gemini_resp)?;
            gw_resp.model = self.default_model.clone();
            Ok(gw_resp)
        })
    }

    fn complete_chunks(&self, request: &GatewayRequest) -> Result<ChunkedResponse, AdapterError> {
        self.handle.block_on(async {
            let api_key = self
                .api_key
                .as_deref()
                .ok_or_else(|| AdapterError::AuthError("GEMINI_API_KEY not set".to_string()))?;

            // streamGenerateContent with SSE output — verified 2026-04-28
            // https://ai.google.dev/api/generate-content#method:-models.streamgeneratecontent
            let url = format!(
                "https://generativelanguage.googleapis.com/v1beta/models/{}:streamGenerateContent?alt=sse&key={}",
                self.default_model, api_key
            );

            let body = build_gemini_request(request);

            let response = self
                .client
                .post(&url)
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

            parse_gemini_stream(&text, &self.default_model)
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
            tier: ModelTier::Quick,
            constraints: RoutingConstraints::default(),
            messages: vec![
                Message {
                    role: "system".to_string(),
                    content: "Be concise.".to_string(),
                },
                Message {
                    role: "user".to_string(),
                    content: "Hello".to_string(),
                },
                Message {
                    role: "assistant".to_string(),
                    content: "Hi!".to_string(),
                },
                Message {
                    role: "user".to_string(),
                    content: "How are you?".to_string(),
                },
            ],
            max_tokens: Some(2048),
            stream: false,
            metadata: Default::default(),
        }
    }

    #[test]
    fn build_request_extracts_system_to_instruction() {
        let req = test_request();
        let gemini = build_gemini_request(&req);

        // System message → systemInstruction, not in contents
        assert!(gemini.system_instruction.is_some());
        assert_eq!(
            gemini.system_instruction.as_ref().unwrap().parts[0].text,
            "Be concise."
        );

        // Contents should have 3 entries (user, model, user) — no system
        assert_eq!(gemini.contents.len(), 3);
        assert_eq!(gemini.contents[0].role, "user");
        assert_eq!(gemini.contents[1].role, "model"); // assistant → model
        assert_eq!(gemini.contents[2].role, "user");
    }

    #[test]
    fn build_request_maps_assistant_to_model() {
        let req = test_request();
        let gemini = build_gemini_request(&req);
        assert_eq!(gemini.contents[1].role, "model");
        assert_eq!(gemini.contents[1].parts[0].text, "Hi!");
    }

    #[test]
    fn build_request_sets_max_output_tokens() {
        let req = test_request();
        let gemini = build_gemini_request(&req);
        assert_eq!(
            gemini.generation_config.as_ref().unwrap().max_output_tokens,
            Some(2048)
        );
    }

    #[test]
    fn parse_response_extracts_text_and_usage() {
        let resp = GeminiResponse {
            candidates: Some(vec![GeminiCandidate {
                content: Some(GeminiCandidateContent {
                    parts: Some(vec![GeminiResponsePart {
                        text: Some("I'm fine!".to_string()),
                    }]),
                }),
                finish_reason: Some("STOP".to_string()),
            }]),
            usage_metadata: Some(GeminiUsageMetadata {
                prompt_token_count: Some(15),
                candidates_token_count: Some(8),
            }),
        };
        let gw = parse_gemini_response(resp).unwrap();
        assert_eq!(gw.content, "I'm fine!");
        assert_eq!(gw.tokens_in, 15);
        assert_eq!(gw.tokens_out, 8);
        assert_eq!(gw.finish_reason, "stop");
    }

    #[test]
    fn parse_response_maps_max_tokens_finish_reason() {
        let resp = GeminiResponse {
            candidates: Some(vec![GeminiCandidate {
                content: Some(GeminiCandidateContent {
                    parts: Some(vec![GeminiResponsePart {
                        text: Some("truncated...".to_string()),
                    }]),
                }),
                finish_reason: Some("MAX_TOKENS".to_string()),
            }]),
            usage_metadata: None,
        };
        let gw = parse_gemini_response(resp).unwrap();
        assert_eq!(gw.finish_reason, "length");
    }

    #[test]
    fn parse_response_handles_no_candidates() {
        let resp = GeminiResponse {
            candidates: None,
            usage_metadata: None,
        };
        let err = parse_gemini_response(resp).unwrap_err();
        assert!(matches!(err, AdapterError::ParseError(_)));
    }

    // --- Streaming: parse_gemini_stream ---

    #[test]
    fn gemini_streaming_parse_yields_three_chunks() {
        // Three content chunks; only the last has usageMetadata and finishReason.
        let sse_body = concat!(
            "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"Hello\"}]}}]}\n",
            "\n",
            "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\", there\"}]}}]}\n",
            "\n",
            "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\" world!\"}]},\"finishReason\":\"STOP\"}],",
            "\"usageMetadata\":{\"promptTokenCount\":7,\"candidatesTokenCount\":5}}\n",
        );
        let result = parse_gemini_stream(sse_body, "gemini-2.0-flash").unwrap();
        assert_eq!(result.chunks.len(), 3);
        assert_eq!(result.chunks[0], "Hello");
        assert_eq!(result.chunks[1], ", there");
        assert_eq!(result.chunks[2], " world!");
        assert!(result.tokens_in > 0, "tokens_in should be non-zero");
        assert!(result.tokens_out > 0, "tokens_out should be non-zero");
        assert_eq!(result.tokens_in, 7);
        assert_eq!(result.tokens_out, 5);
        assert_eq!(result.finish_reason, "stop");
        assert_eq!(result.model, "gemini-2.0-flash");
    }

    #[test]
    fn gemini_streaming_parse_skips_malformed_events() {
        let sse_body = concat!(
            "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"Hello\"}]}}]}\n",
            "\n",
            "data: not-valid-json\n",
            "\n",
            ": comment line ignored\n",
            "\n",
            "event: meta\n",
            "\n",
            "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\" world\"}]},\"finishReason\":\"STOP\"}],",
            "\"usageMetadata\":{\"promptTokenCount\":5,\"candidatesTokenCount\":3}}\n",
        );
        let result = parse_gemini_stream(sse_body, "gemini-2.0-flash").unwrap();
        assert_eq!(result.chunks, vec!["Hello", " world"]);
        assert_eq!(result.finish_reason, "stop");
        assert_eq!(result.tokens_in, 5);
        assert_eq!(result.tokens_out, 3);
    }

    #[test]
    fn gemini_streaming_parse_empty_body_returns_parse_error() {
        let result = parse_gemini_stream("", "gemini-2.0-flash");
        assert!(matches!(result, Err(AdapterError::ParseError(_))));
    }

    #[test]
    fn gemini_streaming_parse_maps_max_tokens_finish_reason() {
        let sse_body = concat!(
            "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"truncated\"}]},\"finishReason\":\"MAX_TOKENS\"}],",
            "\"usageMetadata\":{\"promptTokenCount\":10,\"candidatesTokenCount\":100}}\n",
        );
        let result = parse_gemini_stream(sse_body, "gemini-2.0-flash").unwrap();
        assert_eq!(result.finish_reason, "length");
        assert_eq!(result.chunks, vec!["truncated"]);
    }

    #[test]
    fn gemini_streaming_parse_empty_text_parts_excluded() {
        // Empty string parts should not add to chunks — only non-empty text included.
        let sse_body = concat!(
            "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"\"}]}}]}\n",
            "\n",
            "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"real\"}]},\"finishReason\":\"STOP\"}],",
            "\"usageMetadata\":{\"promptTokenCount\":4,\"candidatesTokenCount\":2}}\n",
        );
        let result = parse_gemini_stream(sse_body, "gemini-2.0-flash").unwrap();
        assert_eq!(result.chunks, vec!["real"]);
    }
}
