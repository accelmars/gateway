use super::openai_compat::OpenAiCompatibleAdapter;

/// OpenAI direct adapter — OpenAI API (GPT models).
/// sensitive_ok (US-hosted). CREATE tier alternative to Claude Sonnet.
/// Default model: gpt-4o ($2.50/$15.00 per 1M tokens). Env var: OPENAI_API_KEY.
pub fn new_openai_adapter(api_key: Option<String>) -> OpenAiCompatibleAdapter {
    OpenAiCompatibleAdapter::new(
        "openai",
        "https://api.openai.com/v1/chat/completions",
        api_key,
        "gpt-4o",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use accelmars_gateway_core::ProviderAdapter;

    #[tokio::test]
    async fn openai_adapter_name() {
        let adapter = new_openai_adapter(Some("test-key".to_string()));
        assert_eq!(adapter.name(), "openai");
    }

    #[tokio::test]
    async fn openai_adapter_available_with_key() {
        let adapter = new_openai_adapter(Some("test-key".to_string()));
        assert!(adapter.is_available());
    }

    #[tokio::test]
    async fn openai_adapter_unavailable_without_key() {
        let adapter = new_openai_adapter(None);
        assert!(!adapter.is_available());
    }
}
