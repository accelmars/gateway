use super::openai_compat::OpenAiCompatibleAdapter;

/// NVIDIA NIM adapter — NVIDIA Inference Microservices hosted API.
/// OpenAI-compatible. 1M context. Mamba-2 hybrid architecture.
/// Free dev tier available (40 RPM ceiling). Env var: NVIDIA_API_KEY.
/// Default model: nvidia/llama-3.1-nemotron-70b-instruct (~$0.10/$0.50 per 1M tokens).
pub fn new_nvidia_adapter(api_key: Option<String>) -> OpenAiCompatibleAdapter {
    OpenAiCompatibleAdapter::new(
        "nvidia",
        "https://integrate.api.nvidia.com/v1/chat/completions",
        api_key,
        "nvidia/llama-3.1-nemotron-70b-instruct",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use accelmars_gateway_core::ProviderAdapter;

    #[tokio::test]
    async fn nvidia_adapter_name() {
        let adapter = new_nvidia_adapter(Some("test-key".to_string()));
        assert_eq!(adapter.name(), "nvidia");
    }

    #[tokio::test]
    async fn nvidia_adapter_available_with_key() {
        let adapter = new_nvidia_adapter(Some("test-key".to_string()));
        assert!(adapter.is_available());
    }

    #[tokio::test]
    async fn nvidia_adapter_unavailable_without_key() {
        let adapter = new_nvidia_adapter(None);
        assert!(!adapter.is_available());
    }
}
