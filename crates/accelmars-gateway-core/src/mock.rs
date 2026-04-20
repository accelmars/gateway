use std::time::Duration;

use uuid::Uuid;

use crate::adapter::{AdapterError, ProviderAdapter};
use crate::types::{GatewayRequest, GatewayResponse};

/// Deterministic mock adapter for tests and CI.
///
/// Returns a fixed response (configurable). Supports optional artificial latency.
/// Always available. No network calls. No API keys required.
///
/// # Usage
/// Set `GATEWAY_MODE=mock` in the server — the server wires `MockAdapter`.
/// In tests: construct directly, pass to anything accepting `&dyn ProviderAdapter`.
/// Use `.with_name("provider")` to simulate a named provider in router tests.
pub struct MockAdapter {
    pub default_response: String,
    pub latency: Option<Duration>,
    name: String,
}

impl MockAdapter {
    pub fn new(default_response: impl Into<String>) -> Self {
        Self {
            default_response: default_response.into(),
            latency: None,
            name: "mock".to_string(),
        }
    }

    pub fn with_latency(mut self, latency: Duration) -> Self {
        self.latency = Some(latency);
        self
    }

    /// Override the adapter name (for router tests that need named mock providers).
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }
}

impl Default for MockAdapter {
    fn default() -> Self {
        Self::new("Mock response from AccelMars gateway.")
    }
}

impl ProviderAdapter for MockAdapter {
    fn name(&self) -> &str {
        &self.name
    }

    fn complete(&self, request: &GatewayRequest) -> Result<GatewayResponse, AdapterError> {
        if let Some(latency) = self.latency {
            std::thread::sleep(latency);
        }

        let tokens_in = estimate_tokens(&request.messages);
        let tokens_out = estimate_tokens_str(&self.default_response);

        Ok(GatewayResponse {
            id: format!("mock-{}", Uuid::new_v4()),
            model: "mock".to_string(),
            content: self.default_response.clone(),
            tokens_in,
            tokens_out,
            finish_reason: "stop".to_string(),
        })
    }

    fn is_available(&self) -> bool {
        true
    }
}

fn estimate_tokens(messages: &[crate::types::Message]) -> u32 {
    let total_chars: usize = messages.iter().map(|m| m.content.len()).sum();
    (total_chars / 4) as u32
}

fn estimate_tokens_str(s: &str) -> u32 {
    (s.len() / 4) as u32
}
