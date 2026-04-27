use super::openai_compat::OpenAiCompatibleAdapter;

/// Stepfun adapter — StepFun (Shanghai) API. PROCESS tier primary.
/// OpenAI-compatible. 97.3% AIME 2025. sensitive_excluded (Chinese-hosted).
/// Default model: step-3.5-flash ($0.10/$0.30 per 1M tokens). Env var: STEP_API_KEY.
/// Rate limit note: account must reach V1 (¥50 cumulative spend) for production RPM.
pub fn new_stepfun_adapter(api_key: Option<String>) -> OpenAiCompatibleAdapter {
    OpenAiCompatibleAdapter::new(
        "stepfun",
        "https://api.stepfun.ai/v1/chat/completions",
        api_key,
        "step-3.5-flash",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use accelmars_gateway_core::ProviderAdapter;

    #[tokio::test]
    async fn stepfun_adapter_name() {
        let adapter = new_stepfun_adapter(Some("test-key".to_string()));
        assert_eq!(adapter.name(), "stepfun");
    }

    #[tokio::test]
    async fn stepfun_adapter_available_with_key() {
        let adapter = new_stepfun_adapter(Some("test-key".to_string()));
        assert!(adapter.is_available());
    }

    #[tokio::test]
    async fn stepfun_adapter_unavailable_without_key() {
        let adapter = new_stepfun_adapter(None);
        assert!(!adapter.is_available());
    }
}
