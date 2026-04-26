use std::time::Duration;

use crate::types::{ChunkedResponse, GatewayRequest, GatewayResponse};

/// The universal provider adapter trait.
///
/// All provider SDKs live in the binary crate or dedicated adapter crates —
/// never in `accelmars-gateway-core`. This trait is the only boundary.
///
/// # ENGINE-LESSON Compliance
/// Rule 1: External I/O boundary is this trait (defined in -core, implemented outside).
/// Rule 8: No provider SDK imports in core — this file has zero external provider deps.
pub trait ProviderAdapter: Send + Sync {
    /// Provider identifier (e.g., "gemini", "deepseek", "claude", "mock").
    fn name(&self) -> &str;

    /// Execute a completion request, returning a normalized response.
    ///
    /// # Errors
    /// Returns [`AdapterError`] for rate limits, auth failures, timeouts, or parse errors.
    fn complete(&self, request: &GatewayRequest) -> Result<GatewayResponse, AdapterError>;

    /// Execute a streaming completion, returning content as an ordered `Vec` of chunks.
    ///
    /// Each element in the returned `Vec<String>` maps to one SSE `data:` event.
    /// The server emits them sequentially — the first element contains the opening
    /// tokens, the last contains the tail before `finish_reason`.
    ///
    /// # Default Implementation
    /// Calls [`complete`] and wraps `response.content` in a single-element `Vec`.
    /// This preserves Phase 1 streaming behavior (one SSE event, full content) for
    /// all adapters that have not yet implemented true token-level streaming.
    ///
    /// # Override
    /// Adapters that support server-sent streaming (Gemini, DeepSeek, Claude, etc.)
    /// override this method to return token-level granularity by consuming the
    /// provider's SSE stream inside a `tokio::task::spawn_blocking` closure.
    ///
    /// # Error Handling
    /// Returns [`AdapterError`] for all failure cases — same as [`complete`].
    /// A partial chunk `Vec` should NOT be returned on error; return `Err` instead.
    fn complete_chunks(&self, request: &GatewayRequest) -> Result<ChunkedResponse, AdapterError> {
        self.complete(request).map(|r| ChunkedResponse {
            id: r.id,
            model: r.model,
            chunks: vec![r.content],
            tokens_in: r.tokens_in,
            tokens_out: r.tokens_out,
            finish_reason: r.finish_reason,
        })
    }

    /// Whether this provider is currently reachable and configured.
    fn is_available(&self) -> bool;
}

/// Errors that any provider adapter may return.
#[derive(Debug, thiserror::Error)]
pub enum AdapterError {
    #[error("rate limited by provider — retry after {retry_after:?}")]
    RateLimit { retry_after: Option<Duration> },

    #[error("authentication error: {0}")]
    AuthError(String),

    #[error("provider request timed out")]
    Timeout,

    #[error("provider error: {0}")]
    ProviderError(String),

    #[error("failed to parse provider response: {0}")]
    ParseError(String),
}
