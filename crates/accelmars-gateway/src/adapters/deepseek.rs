use super::openai_compat::OpenAiCompatibleAdapter;

/// DeepSeek V3 adapter — OpenAI-compatible API, near-passthrough.
/// Cost: $0.28/M input, $0.42/M output. Cache hits: $0.028/M (90% off).
/// True SSE streaming via `complete_chunks()` — uses `complete_chunks_oai_sse()` in `openai_compat`.
pub fn new_deepseek_adapter(api_key: Option<String>) -> OpenAiCompatibleAdapter {
    OpenAiCompatibleAdapter::new(
        "deepseek",
        "https://api.deepseek.com/v1/chat/completions",
        api_key,
        "deepseek-chat",
    )
}
