use std::collections::VecDeque;
use std::sync::Mutex;

use accelmars_gateway_core::{AdapterError, GatewayRequest, GatewayResponse, ProviderAdapter};

/// Test adapter that replays recorded responses in order.
/// Each call to `complete` pops the next response from the queue.
/// When the queue is empty, returns a ProviderError.
pub struct RecordedAdapter {
    adapter_name: String,
    responses: Mutex<VecDeque<Result<GatewayResponse, AdapterError>>>,
}

impl RecordedAdapter {
    pub fn new(
        name: impl Into<String>,
        responses: Vec<Result<GatewayResponse, AdapterError>>,
    ) -> Self {
        Self {
            adapter_name: name.into(),
            responses: Mutex::new(VecDeque::from(responses)),
        }
    }

    pub fn single_ok(name: impl Into<String>, response: GatewayResponse) -> Self {
        Self::new(name, vec![Ok(response)])
    }
}

impl ProviderAdapter for RecordedAdapter {
    fn name(&self) -> &str {
        &self.adapter_name
    }

    fn complete(&self, _request: &GatewayRequest) -> Result<GatewayResponse, AdapterError> {
        let mut queue = self.responses.lock().unwrap_or_else(|e| e.into_inner());
        queue.pop_front().unwrap_or_else(|| {
            Err(AdapterError::ProviderError(
                "no more recorded responses".to_string(),
            ))
        })
    }

    fn is_available(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use accelmars_gateway_core::{Message, ModelTier, RoutingConstraints};
    use std::time::Duration;

    fn test_request() -> GatewayRequest {
        GatewayRequest {
            tier: ModelTier::Quick,
            constraints: RoutingConstraints::default(),
            messages: vec![Message {
                role: "user".to_string(),
                content: "test".to_string(),
            }],
            max_tokens: None,
            stream: false,
            metadata: Default::default(),
        }
    }

    fn test_response(content: &str) -> GatewayResponse {
        GatewayResponse {
            id: "rec-1".to_string(),
            model: "recorded".to_string(),
            content: content.to_string(),
            tokens_in: 5,
            tokens_out: 10,
            finish_reason: "stop".to_string(),
        }
    }

    #[test]
    fn single_ok_returns_configured_response() {
        let adapter = RecordedAdapter::single_ok("test", test_response("hello"));
        let resp = adapter.complete(&test_request()).unwrap();
        assert_eq!(resp.content, "hello");
        assert_eq!(resp.model, "recorded");
    }

    #[test]
    fn multiple_responses_returned_in_order() {
        let adapter = RecordedAdapter::new(
            "test",
            vec![
                Ok(test_response("first")),
                Ok(test_response("second")),
                Err(AdapterError::RateLimit {
                    retry_after: Some(Duration::from_secs(30)),
                }),
            ],
        );
        let req = test_request();

        assert_eq!(adapter.complete(&req).unwrap().content, "first");
        assert_eq!(adapter.complete(&req).unwrap().content, "second");
        assert!(matches!(
            adapter.complete(&req),
            Err(AdapterError::RateLimit { .. })
        ));
    }

    #[test]
    fn empty_queue_returns_provider_error() {
        let adapter = RecordedAdapter::new("test", vec![]);
        let err = adapter.complete(&test_request()).unwrap_err();
        assert!(matches!(err, AdapterError::ProviderError(_)));
    }

    #[test]
    fn always_available() {
        let adapter = RecordedAdapter::new("test", vec![]);
        assert!(adapter.is_available());
        assert_eq!(adapter.name(), "test");
    }
}
