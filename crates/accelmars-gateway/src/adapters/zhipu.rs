use super::openai_compat::OpenAiCompatibleAdapter;

/// Zhipu AI (Z.ai) adapter — GLM-4 series via OpenRouter.
/// No direct international endpoint; all traffic routes through OpenRouter.
/// The api_key parameter must be an OpenRouter API key (not a Zhipu key directly).
/// Configure gateway.toml with api_key_env = "OPENROUTER_API_KEY" for this adapter.
/// sensitive_excluded (Chinese-hosted). Model: zhipu/glm-4-plus ($1.05/$3.50 per 1M tokens).
pub fn new_zhipu_adapter(api_key: Option<String>) -> OpenAiCompatibleAdapter {
    OpenAiCompatibleAdapter::new(
        "zhipu",
        "https://openrouter.ai/api/v1/chat/completions",
        api_key,
        "zhipu/glm-4-plus",
    )
    .with_extra_header("HTTP-Referer", "https://accelmars.com")
    .with_extra_header("X-Title", "AccelMars Gateway")
}

#[cfg(test)]
mod tests {
    use super::*;
    use accelmars_gateway_core::ProviderAdapter;

    #[tokio::test]
    async fn zhipu_adapter_name() {
        let adapter = new_zhipu_adapter(Some("test-key".to_string()));
        assert_eq!(adapter.name(), "zhipu");
    }

    #[tokio::test]
    async fn zhipu_adapter_available_with_key() {
        let adapter = new_zhipu_adapter(Some("test-key".to_string()));
        assert!(adapter.is_available());
    }

    #[tokio::test]
    async fn zhipu_adapter_unavailable_without_key() {
        let adapter = new_zhipu_adapter(None);
        assert!(!adapter.is_available());
    }
}
