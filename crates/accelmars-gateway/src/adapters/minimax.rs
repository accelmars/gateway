use super::openai_compat::OpenAiCompatibleAdapter;

/// MiniMax adapter — MiniMax (Beijing) API. PROCESS tier fallback.
/// OpenAI-compatible. Flat 500 RPM. Best $/output in the gateway at Process tier.
/// sensitive_excluded (Chinese-hosted). Env var: MINIMAX_API_KEY.
/// Default model: MiniMax-M2.5 ($0.15/$1.15 per 1M tokens).
pub fn new_minimax_adapter(api_key: Option<String>) -> OpenAiCompatibleAdapter {
    OpenAiCompatibleAdapter::new(
        "minimax",
        "https://api.minimax.io/v1/chat/completions",
        api_key,
        "MiniMax-M2.5",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use accelmars_gateway_core::ProviderAdapter;

    #[tokio::test]
    async fn minimax_adapter_name() {
        let adapter = new_minimax_adapter(Some("test-key".to_string()));
        assert_eq!(adapter.name(), "minimax");
    }

    #[tokio::test]
    async fn minimax_adapter_available_with_key() {
        let adapter = new_minimax_adapter(Some("test-key".to_string()));
        assert!(adapter.is_available());
    }

    #[tokio::test]
    async fn minimax_adapter_unavailable_without_key() {
        let adapter = new_minimax_adapter(None);
        assert!(!adapter.is_available());
    }
}
