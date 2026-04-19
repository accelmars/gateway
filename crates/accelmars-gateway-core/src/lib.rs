pub mod adapter;
pub mod mock;
pub mod types;

pub use adapter::{AdapterError, ProviderAdapter};
pub use mock::MockAdapter;
pub use types::{
    Capability, CostPreference, GatewayRequest, GatewayResponse, Latency, Message, ModelTier,
    Privacy, RoutingConstraints,
};

#[cfg(test)]
mod tests {
    use std::str::FromStr;
    use std::time::Duration;

    use super::*;

    // --- ModelTier parsing ---

    #[test]
    fn model_tier_from_str_quick() {
        assert_eq!(ModelTier::from_str("quick").unwrap(), ModelTier::Quick);
    }

    #[test]
    fn model_tier_from_str_standard() {
        assert_eq!(
            ModelTier::from_str("standard").unwrap(),
            ModelTier::Standard
        );
    }

    #[test]
    fn model_tier_from_str_max() {
        assert_eq!(ModelTier::from_str("max").unwrap(), ModelTier::Max);
    }

    #[test]
    fn model_tier_from_str_ultra() {
        assert_eq!(ModelTier::from_str("ultra").unwrap(), ModelTier::Ultra);
    }

    #[test]
    fn model_tier_from_str_case_insensitive() {
        assert_eq!(
            ModelTier::from_str("STANDARD").unwrap(),
            ModelTier::Standard
        );
        assert_eq!(ModelTier::from_str("Quick").unwrap(), ModelTier::Quick);
    }

    #[test]
    fn model_tier_from_str_invalid_returns_err() {
        assert!(ModelTier::from_str("invalid").is_err());
        assert!(ModelTier::from_str("").is_err());
        assert!(ModelTier::from_str("haiku").is_err());
    }

    #[test]
    fn model_tier_display() {
        assert_eq!(ModelTier::Quick.to_string(), "quick");
        assert_eq!(ModelTier::Standard.to_string(), "standard");
        assert_eq!(ModelTier::Max.to_string(), "max");
        assert_eq!(ModelTier::Ultra.to_string(), "ultra");
    }

    // --- RoutingConstraints default ---

    #[test]
    fn routing_constraints_default_values() {
        let c = RoutingConstraints::default();
        assert_eq!(c.privacy, Privacy::Open);
        assert_eq!(c.latency, Latency::Normal);
        assert_eq!(c.cost, CostPreference::Default);
        assert!(c.capabilities.is_empty());
        assert!(c.provider.is_none());
    }

    // --- MockAdapter ---

    fn make_request() -> GatewayRequest {
        GatewayRequest {
            tier: ModelTier::Quick,
            constraints: RoutingConstraints::default(),
            messages: vec![Message {
                role: "user".to_string(),
                content: "Hello, mock.".to_string(),
            }],
            max_tokens: None,
            stream: false,
            metadata: Default::default(),
        }
    }

    #[test]
    fn mock_adapter_complete_returns_deterministic_response() {
        let adapter = MockAdapter::new("hello from mock");
        let request = make_request();
        let response = adapter.complete(&request).unwrap();
        assert_eq!(response.content, "hello from mock");
        assert_eq!(response.model, "mock");
        assert_eq!(response.finish_reason, "stop");
        assert!(response.id.starts_with("mock-"));
    }

    #[test]
    fn mock_adapter_complete_with_artificial_latency() {
        let adapter = MockAdapter::new("delayed").with_latency(Duration::from_millis(10));
        let request = make_request();
        let start = std::time::Instant::now();
        let response = adapter.complete(&request).unwrap();
        assert!(start.elapsed() >= Duration::from_millis(10));
        assert_eq!(response.content, "delayed");
    }

    #[test]
    fn mock_adapter_is_always_available() {
        let adapter = MockAdapter::default();
        assert!(adapter.is_available());
    }

    #[test]
    fn mock_adapter_name_is_mock() {
        let adapter = MockAdapter::default();
        assert_eq!(adapter.name(), "mock");
    }

    #[test]
    fn mock_adapter_token_estimates_nonzero_for_nonempty_input() {
        let adapter = MockAdapter::new("a response with several words in it");
        let request = make_request();
        let response = adapter.complete(&request).unwrap();
        assert!(response.tokens_in > 0);
        assert!(response.tokens_out > 0);
    }
}
