pub mod claude;
pub mod deepseek;
pub mod gemini;
pub mod groq;
pub mod openai_compat;
pub mod openrouter;
pub mod recorded;

pub use claude::ClaudeAdapter;
pub use deepseek::new_deepseek_adapter;
pub use gemini::GeminiAdapter;
pub use groq::new_groq_adapter;
pub use openai_compat::OpenAiCompatibleAdapter;
pub use openrouter::new_openrouter_adapter;
pub use recorded::RecordedAdapter;
