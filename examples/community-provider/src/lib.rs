//! Minimal community provider example for AccelMars Gateway.
//!
//! Demonstrates `ProviderAdapter` implementation without any network calls.
//! `StubProvider` returns a deterministic echo response — suitable for local
//! development, cassette recording, and as a template for real providers.
//!
//! See `../README.md` for build instructions and `docs/ADDING-A-PROVIDER.md`
//! in the gateway repo for the full step-by-step guide.

use accelmars_gateway_core::{
    AdapterError, GatewayRequest, GatewayResponse, ModelTier, ProviderAdapter,
};

/// A minimal provider that echoes the last user message.
///
/// Implements `ProviderAdapter` with no network calls, no API key, and
/// deterministic output. Use this as the starting point for your own provider.
pub struct StubProvider {
    supported_tiers: Vec<ModelTier>,
}

impl StubProvider {
    pub fn new() -> Self {
        Self {
            supported_tiers: vec![ModelTier::Quick, ModelTier::Standard],
        }
    }

    /// Tiers this provider covers.
    ///
    /// Use this when building a registry or routing table that maps tiers to
    /// provider names. Not part of `ProviderAdapter` — it is metadata you
    /// declare alongside your adapter so callers can wire it correctly.
    pub fn supported_tiers(&self) -> &[ModelTier] {
        &self.supported_tiers
    }
}

impl Default for StubProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl ProviderAdapter for StubProvider {
    /// Identifies this provider in the registry and request routing.
    ///
    /// Must match the key used in `gateway.toml` under `[providers.stub]`
    /// and in `build_registry_from_config` in `main.rs`.
    fn name(&self) -> &str {
        "stub"
    }

    /// Execute a completion request.
    ///
    /// This stub echoes the last user message. A real provider would:
    ///   1. Map `request.tier` to a model ID via config (never hardcode model IDs here)
    ///   2. Serialize `request.messages` to the provider's API format
    ///   3. POST to the provider endpoint with the API key from env
    ///   4. Deserialize the response and map it to `GatewayResponse`
    ///   5. Return `Err(AdapterError::RateLimit { .. })` or `Err(AdapterError::AuthError(..))` as needed
    fn complete(&self, request: &GatewayRequest) -> Result<GatewayResponse, AdapterError> {
        let user_content = request
            .messages
            .last()
            .map(|m| m.content.as_str())
            .unwrap_or("(no message)");

        let content = format!("[stub] Echo: {user_content}");

        let tokens_in = request
            .messages
            .iter()
            .map(|m| m.content.len())
            .sum::<usize>()
            / 4;
        let tokens_out = content.len() / 4;

        Ok(GatewayResponse {
            id: "stub-response-1".to_string(),
            model: "stub-v1".to_string(),
            content,
            tokens_in: tokens_in as u32,
            tokens_out: tokens_out as u32,
            finish_reason: "stop".to_string(),
        })
    }

    /// Whether this provider is configured and reachable.
    ///
    /// For a real provider: check that the API key env var is set and non-empty.
    /// The gateway calls this on startup to populate `gateway status` output.
    fn is_available(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use accelmars_gateway_core::{Message, RoutingConstraints};

    use super::*;

    fn make_request(content: &str) -> GatewayRequest {
        GatewayRequest {
            tier: ModelTier::Quick,
            constraints: RoutingConstraints::default(),
            messages: vec![Message {
                role: "user".to_string(),
                content: content.to_string(),
            }],
            max_tokens: None,
            stream: false,
            metadata: Default::default(),
        }
    }

    #[test]
    fn name_is_stub() {
        assert_eq!(StubProvider::new().name(), "stub");
    }

    #[test]
    fn is_always_available() {
        assert!(StubProvider::new().is_available());
    }

    #[test]
    fn complete_echoes_user_message() {
        let provider = StubProvider::new();
        let req = make_request("hello world");
        let resp = provider.complete(&req).unwrap();
        assert!(resp.content.contains("hello world"));
        assert_eq!(resp.finish_reason, "stop");
        assert!(!resp.model.is_empty());
    }

    #[test]
    fn complete_nonzero_tokens_for_nonempty_input() {
        let provider = StubProvider::new();
        let req = make_request("what is the meaning of life?");
        let resp = provider.complete(&req).unwrap();
        assert!(resp.tokens_in > 0);
        assert!(resp.tokens_out > 0);
    }

    #[test]
    fn supported_tiers_covers_quick_and_standard() {
        let provider = StubProvider::new();
        let tiers = provider.supported_tiers();
        assert!(tiers.contains(&ModelTier::Quick));
        assert!(tiers.contains(&ModelTier::Standard));
    }
}
