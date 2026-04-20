use super::openai_compat::OpenAiCompatibleAdapter;

/// Groq adapter — ultra-fast inference (394-1000 TPS).
/// OpenAI-compatible API. Best for `latency: low` constraint.
pub fn new_groq_adapter(api_key: Option<String>) -> OpenAiCompatibleAdapter {
    OpenAiCompatibleAdapter::new(
        "groq",
        "https://api.groq.com/openai/v1/chat/completions",
        api_key,
        "llama-3.3-70b-versatile",
    )
}
