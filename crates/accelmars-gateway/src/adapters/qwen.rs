use super::openai_compat::OpenAiCompatibleAdapter;

/// Qwen adapter — Alibaba Cloud via Dashscope international endpoint.
/// OpenAI-compatible API. 1M context window. sensitive_excluded (Chinese-hosted).
/// Default model: qwen3.6-plus ($0.325/$1.95 per 1M tokens). Env var: DASHSCOPE_API_KEY.
pub fn new_qwen_adapter(api_key: Option<String>) -> OpenAiCompatibleAdapter {
    OpenAiCompatibleAdapter::new(
        "qwen",
        "https://dashscope-intl.aliyuncs.com/compatible-mode/v1/chat/completions",
        api_key,
        "qwen3.6-plus",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use accelmars_gateway_core::ProviderAdapter;

    #[tokio::test]
    async fn qwen_adapter_name() {
        let adapter = new_qwen_adapter(Some("test-key".to_string()));
        assert_eq!(adapter.name(), "qwen");
    }

    #[tokio::test]
    async fn qwen_adapter_available_with_key() {
        let adapter = new_qwen_adapter(Some("test-key".to_string()));
        assert!(adapter.is_available());
    }

    #[tokio::test]
    async fn qwen_adapter_unavailable_without_key() {
        let adapter = new_qwen_adapter(None);
        assert!(!adapter.is_available());
    }
}
