use super::openai_compat::OpenAiCompatibleAdapter;

/// Moonshot adapter — Moonshot AI (Beijing) / Kimi API. THINK tier fallback.
/// OpenAI-compatible. 262K context. Falls back from Qwen in THINK tier.
/// sensitive_excluded (Chinese-hosted). Env var: MOONSHOT_API_KEY.
/// Default model: kimi-k2.5 ($0.44/$2.00 per 1M tokens).
/// Rate limit note: Tier 0 starts at 3 RPM — pre-fund account to advance tier.
pub fn new_moonshot_adapter(api_key: Option<String>) -> OpenAiCompatibleAdapter {
    OpenAiCompatibleAdapter::new(
        "moonshot",
        "https://api.moonshot.cn/v1/chat/completions",
        api_key,
        "kimi-k2.5",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use accelmars_gateway_core::ProviderAdapter;

    #[tokio::test]
    async fn moonshot_adapter_name() {
        let adapter = new_moonshot_adapter(Some("test-key".to_string()));
        assert_eq!(adapter.name(), "moonshot");
    }

    #[tokio::test]
    async fn moonshot_adapter_available_with_key() {
        let adapter = new_moonshot_adapter(Some("test-key".to_string()));
        assert!(adapter.is_available());
    }

    #[tokio::test]
    async fn moonshot_adapter_unavailable_without_key() {
        let adapter = new_moonshot_adapter(None);
        assert!(!adapter.is_available());
    }
}
