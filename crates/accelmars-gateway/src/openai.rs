use serde::{Deserialize, Serialize};

/// OpenAI Chat Completion Request — what clients send to the gateway.
///
/// The `model` field accepts tier names (`quick` / `standard` / `max` / `ultra`).
/// The `metadata` field carries AccelMars routing constraints (privacy, latency, cost,
/// capabilities, provider override).
#[derive(Debug, Clone, Deserialize)]
pub struct ChatCompletionRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
    pub stream: Option<bool>,
    /// AccelMars extension: routing constraints passed as arbitrary JSON object.
    #[serde(default)]
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

/// OpenAI Chat Completion Response.
#[derive(Debug, Clone, Serialize)]
pub struct ChatCompletionResponse {
    pub id: String,
    pub object: String,
    pub created: u64,
    pub model: String,
    pub choices: Vec<Choice>,
    pub usage: Usage,
}

#[derive(Debug, Clone, Serialize)]
pub struct Choice {
    pub index: u32,
    pub message: ChatMessage,
    pub finish_reason: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Usage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
}

/// OpenAI-format error response wrapper.
#[derive(Debug, Clone, Serialize)]
pub struct ErrorResponse {
    pub error: ErrorDetail,
}

#[derive(Debug, Clone, Serialize)]
pub struct ErrorDetail {
    pub message: String,
    #[serde(rename = "type")]
    pub error_type: String,
    pub code: String,
}

impl ErrorResponse {
    pub fn invalid_model(model: &str) -> Self {
        Self {
            error: ErrorDetail {
                message: format!(
                    "unknown model tier: '{model}' — expected quick, standard, max, or ultra"
                ),
                error_type: "invalid_request_error".to_string(),
                code: "invalid_model".to_string(),
            },
        }
    }

    pub fn invalid_request(message: impl Into<String>) -> Self {
        Self {
            error: ErrorDetail {
                message: message.into(),
                error_type: "invalid_request_error".to_string(),
                code: "invalid_request".to_string(),
            },
        }
    }

    pub fn provider_error(message: impl Into<String>) -> Self {
        Self {
            error: ErrorDetail {
                message: message.into(),
                error_type: "api_error".to_string(),
                code: "provider_error".to_string(),
            },
        }
    }
}

/// SSE chunk for streaming responses (`stream: true`).
#[derive(Debug, Clone, Serialize)]
pub struct ChatCompletionChunk {
    pub id: String,
    pub object: String,
    pub created: u64,
    pub model: String,
    pub choices: Vec<StreamChoice>,
}

#[derive(Debug, Clone, Serialize)]
pub struct StreamChoice {
    pub index: u32,
    pub delta: StreamDelta,
    pub finish_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct StreamDelta {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
}
