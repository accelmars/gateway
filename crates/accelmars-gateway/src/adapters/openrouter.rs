use super::openai_compat::OpenAiCompatibleAdapter;

/// OpenRouter meta-provider — one adapter, 300+ models.
/// Uses OpenAI-compatible format with model IDs like "provider/model".
/// Valuable as fallback chain and for models without direct adapters.
pub fn new_openrouter_adapter(api_key: Option<String>) -> OpenAiCompatibleAdapter {
    OpenAiCompatibleAdapter::new(
        "openrouter",
        "https://openrouter.ai/api/v1/chat/completions",
        api_key,
        "deepseek/deepseek-chat",
    )
    .with_extra_header("HTTP-Referer", "https://accelmars.com")
    .with_extra_header("X-Title", "AccelMars Gateway")
}
