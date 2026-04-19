use std::time::Duration;

use crate::types::{GatewayRequest, GatewayResponse};

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
